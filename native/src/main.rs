use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_MESSAGE: u64 = 4 * 1024 * 1024;
type Events = Arc<Mutex<TcpStream>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    port: u16,
    token: String,
}

fn send(events: &Events, value: Value) {
    if let Ok(mut stream) = events.lock() {
        let _ = writeln!(stream, "{value}");
    }
}

fn message(reader: &mut impl BufRead) -> io::Result<Value> {
    let mut line = String::new();
    let count = reader.take(MAX_MESSAGE + 1).read_line(&mut line)?;
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Controller disconnected",
        ));
    }
    if count as u64 > MAX_MESSAGE || !line.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid controller message",
        ));
    }
    serde_json::from_str(&line).map_err(io::Error::other)
}

struct ManagedChild(Child);
impl Drop for ManagedChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(self.0.id() as i32, libc::SIGTERM);
        }
        #[cfg(windows)]
        let _ = self.0.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command.current_dir(binary.parent().unwrap());
    for name in [
        "XRAY_LOCATION_CONFIG",
        "XRAY_LOCATION_CONFDIR",
        "XRAY_TUN_FD",
        "xray.tun.fd",
    ] {
        command.env_remove(name);
    }
    command.stdin(Stdio::null());
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let parent = std::process::id() as libc::pid_t;
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    return Err(io::Error::other(
                        "Network service exited before Xray startup",
                    ));
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn cancelled(stop: &AtomicBool) -> io::Result<()> {
    if stop.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Connection cancelled",
        ))
    } else {
        Ok(())
    }
}

fn check(binary: &Path, config: &Path, stop: &AtomicBool) -> io::Result<()> {
    let mut child = ManagedChild(
        command(binary)
            .args(["run", "-test", "-config", config.to_str().unwrap()])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let output = child.0.stderr.take().unwrap();
    let buffer = thread::spawn(move || {
        let mut text = String::new();
        let _ = output.take(32768).read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        cancelled(stop)?;
        if let Some(status) = child.0.try_wait()? {
            let text = buffer.join().unwrap_or_default();
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "Xray rejected the configuration: {}",
                    text.trim()
                )))
            };
        }
        if Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Xray configuration check timed out",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn log_reader(reader: impl Read + Send + 'static, events: Events, ready: mpsc::Sender<()>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        loop {
            let mut bytes = Vec::new();
            match reader.by_ref().take(16384).read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                _ => {}
            }
            let line = String::from_utf8_lossy(&bytes);
            if line.contains("Xray ") && line.contains(" started") {
                let _ = ready.send(());
            }
            send(&events, json!({"type":"log", "message":line.trim()}));
        }
    });
}

fn binary(dir: &Path) -> PathBuf {
    dir.join(if cfg!(windows) { "xray.exe" } else { "xray" })
}

fn run_session(mut reader: BufReader<TcpStream>, events: Events, dir: &Path) -> io::Result<()> {
    let request = message(&mut reader)?;
    if request["type"] != "start" || !request["config"].is_object() {
        return Err(io::Error::other("Expected Xray configuration"));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let control_stop = stop.clone();
    let heartbeat_events = events.clone();
    thread::spawn(move || loop {
        match message(&mut reader) {
            Ok(value) if value["type"] == "ping" => send(&heartbeat_events, json!({"type":"pong"})),
            _ => {
                control_stop.store(true, Ordering::Relaxed);
                break;
            }
        }
    });
    cancelled(&stop)?;
    let temp = tempfile::Builder::new().prefix("xrayc-").tempdir()?;
    let config_path = temp.path().join("xray.json");
    fs::write(&config_path, serde_json::to_vec(&request["config"])?)?;
    let xray_path = binary(dir);
    check(&xray_path, &config_path, &stop)?;
    cancelled(&stop)?;
    let mut xray = ManagedChild(
        command(&xray_path)
            .args(["run", "-config", config_path.to_str().unwrap()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let (ready_tx, ready_rx) = mpsc::channel();
    log_reader(
        xray.0.stdout.take().unwrap(),
        events.clone(),
        ready_tx.clone(),
    );
    log_reader(xray.0.stderr.take().unwrap(), events.clone(), ready_tx);
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        cancelled(&stop)?;
        if xray.0.try_wait()?.is_some() {
            return Err(io::Error::other("Xray exited during startup."));
        }
        if ready_rx.try_recv().is_ok() {
            break;
        }
        if Instant::now() > deadline {
            return Err(io::Error::other("Xray startup timed out."));
        }
        thread::sleep(Duration::from_millis(50));
    }
    send(&events, json!({"type":"ready"}));
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if xray.0.try_wait()?.is_some() {
            return Err(io::Error::other(
                "Xray stopped unexpectedly. The connection has been closed.",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
    drop(xray);
    send(&events, json!({"type":"stopped"}));
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() == 2 && args[1] == "--controller" {
        if let Err(error) = xrayc_service::controller::run_stdio() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.len() == 2 && args[1] == "--version" {
        println!("xrayc-service {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.len() != 3 || args[1] != "--session" {
        eprintln!("Usage: xrayc-service --session <private-session-file>");
        std::process::exit(1);
    }
    if let Err(error) = run(&args[2]) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(path: &str) -> io::Result<()> {
    let file = fs::File::open(path)?;
    let session: Session = serde_json::from_reader(file.take(4096))?;
    if session.token.len() != 64 || !session.token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(io::Error::other("Invalid session token"));
    }
    let stream = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, session.port)),
        Duration::from_secs(5),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let events = Arc::new(Mutex::new(stream.try_clone()?));
    send(&events, json!({"type":"hello", "token":session.token}));
    let dir = env::current_exe()?.parent().unwrap().to_path_buf();
    let result = run_session(BufReader::new(stream), events.clone(), &dir);
    if let Err(error) = &result {
        if error.kind() == io::ErrorKind::Interrupted {
            send(&events, json!({"type":"stopped"}));
            return Ok(());
        }
        send(
            &events,
            json!({"type":"error", "message":error.to_string()}),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn reads_a_complete_control_message() {
        assert_eq!(
            message(&mut Cursor::new(b"{\"type\":\"stop\"}\n")).unwrap(),
            json!({"type":"stop"})
        );
    }
    #[test]
    fn requires_a_complete_bounded_message() {
        assert!(message(&mut Cursor::new(b"{}")).is_err());
        assert!(message(&mut Cursor::new(vec![b' '; MAX_MESSAGE as usize + 1])).is_err());
    }
    #[test]
    fn rejects_disconnected_controller() {
        assert_eq!(
            message(&mut Cursor::new(b"")).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
    #[cfg(unix)]
    #[test]
    fn stops_and_reaps_a_child_on_drop() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        assert!(child.try_wait().unwrap().is_none());
        drop(ManagedChild(child));
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }
}
