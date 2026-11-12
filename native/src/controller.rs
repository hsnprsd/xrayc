use crate::{config, Status};
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Emitter = Arc<dyn Fn(Status) + Send + Sync>;
struct State {
    status: Status,
    stop: Option<Sender<()>>,
}
#[derive(Clone)]
pub struct Controller {
    state: Arc<Mutex<State>>,
    emit: Emitter,
    dir: PathBuf,
}

impl Controller {
    pub fn new(dir: PathBuf, emit: Emitter) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                status: Status::default(),
                stop: None,
            })),
            emit,
            dir,
        }
    }
    pub fn status(&self) -> Status {
        self.state.lock().unwrap().status.clone()
    }
    fn status_update(&self, phase: &str, message: &str) {
        let status = Status {
            phase: phase.into(),
            message: message.into(),
            connected_at: if phase == "connected" {
                Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                )
            } else {
                None
            },
        };
        let mut state = self.state.lock().unwrap();
        if state.status.phase == "disconnecting" && matches!(phase, "connecting" | "connected") {
            return;
        }
        state.status = status.clone();
        (self.emit)(status);
    }
    pub fn connect(&self, text: &str) -> Result<(), String> {
        let config = config::parse(text)?;
        let (tx, rx) = mpsc::channel();
        {
            let mut state = self.state.lock().unwrap();
            if state.stop.is_some() {
                return Err("A connection is already active. Disconnect first.".into());
            }
            state.stop = Some(tx);
        }
        self.status_update("connecting", "Preparing your connection…");
        let controller = self.clone();
        thread::spawn(move || {
            let result = controller.session(config, &rx);
            let mut state = controller.state.lock().unwrap();
            let status = match result {
                Err(error) if state.status.phase != "disconnecting" => Status {
                    phase: "error".into(),
                    message: error,
                    connected_at: None,
                },
                _ => Status::default(),
            };
            state.stop = None;
            state.status = status.clone();
            (controller.emit)(status);
        });
        Ok(())
    }
    pub fn disconnect(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some(tx) = state.stop.clone() {
            let status = Status {
                phase: "disconnecting".into(),
                message: "Stopping Xray and closing the configured inbounds…".into(),
                connected_at: None,
            };
            state.status = status.clone();
            (self.emit)(status);
            let _ = tx.send(());
        }
    }
    fn session(&self, config: Value, stop: &Receiver<()>) -> Result<(), String> {
        for name in ["xray", "xrayc-service"] {
            if !binary(&self.dir, name).is_file() {
                return Err(format!(
                    "Missing {name}. Reinstall Xrayc or build its networking components."
                ));
            }
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let token = random_hex(32)?;
        let temp = tempfile::Builder::new()
            .prefix("xrayc-session-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let descriptor = temp.path().join("session.json");
        private_write(
            &descriptor,
            &serde_json::to_vec(
                &json!({"port":listener.local_addr().unwrap().port(),"token":token}),
            )
            .unwrap(),
        )?;
        let elevated = config::requires_elevation(&config);
        let service = binary(&self.dir, "xrayc-service");
        let process = if elevated {
            self.status_update(
                "connecting",
                "This configuration needs administrator access. Approve the system prompt…",
            );
            elevate(&service, &descriptor)?
        } else {
            launch(&service, &descriptor)?
        };
        let mut launcher = Launcher(process);
        let deadline = Instant::now() + Duration::from_secs(if elevated { 120 } else { 10 });
        let mut stream = loop {
            if stop.try_recv().is_ok() {
                return Ok(());
            }
            if let Ok((stream, _)) = listener.accept() {
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .map_err(|e| e.to_string())?;
                let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
                if let Ok(hello) = read_message(&mut reader) {
                    if hello["type"] == "hello" && hello["token"] == token {
                        break stream;
                    }
                }
            }
            if let Some(status) = launcher.0.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err(if elevated { "Administrator permission was denied or no authorization agent is running. Try connecting again." } else { "The network service could not start. Try reinstalling Xrayc." }.into());
                }
            }
            if Instant::now() > deadline {
                return Err("Network service startup or administrator approval timed out. Try connecting again.".into());
            }
            thread::sleep(Duration::from_millis(100));
        };
        drop(listener);
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let request = json!({"type":"start","config":config::runtime_config(&config)});
        writeln!(stream, "{request}").map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| e.to_string())?;
        let (events_tx, events_rx) = mpsc::channel();
        let read_stream = stream.try_clone().map_err(|e| e.to_string())?;
        thread::spawn(move || {
            let mut reader = BufReader::new(read_stream);
            while let Ok(value) = read_message(&mut reader) {
                if events_tx.send(value).is_err() {
                    return;
                }
            }
            let _ = events_tx.send(json!({"type":"closed"}));
        });
        self.status_update(
            "connecting",
            "Checking your configuration and starting Xray…",
        );
        let mut connected = false;
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut last_ping = Instant::now();
        let mut last_log = String::new();
        let result = loop {
            if stop.try_recv().is_ok() {
                break Ok(());
            }
            if !connected && Instant::now() > deadline {
                break Err("Xray took too long to start.".into());
            }
            if last_ping.elapsed() > Duration::from_secs(2) {
                if writeln!(stream, "{{\"type\":\"ping\"}}").is_err() {
                    break Err("The network service disconnected.".into());
                }
                last_ping = Instant::now();
            }
            match events_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(value) => match value["type"].as_str() {
                    Some("ready") => {
                        connected = true;
                        self.status_update(
                            "connected",
                            "Xray is running. Inbounds and routing follow your configuration.",
                        );
                    }
                    Some("log") => {
                        last_log = value["message"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(1000)
                            .collect();
                    }
                    Some("error") => {
                        let mut error = value["message"]
                            .as_str()
                            .unwrap_or("Xray startup failed.")
                            .to_owned();
                        if !last_log.is_empty() {
                            error.push_str(&format!("\n{last_log}"));
                        }
                        break Err(error);
                    }
                    Some("closed") => {
                        break Err(
                            "The network service stopped. The connection has been closed.".into(),
                        )
                    }
                    _ => {}
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err("The network service disconnected.".into())
                }
                _ => {}
            }
        };
        let _ = writeln!(stream, "{{\"type\":\"stop\"}}");
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let deadline = Instant::now() + Duration::from_secs(12);
        while Instant::now() < deadline {
            match events_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(value) if value["type"] == "stopped" || value["type"] == "closed" => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => {}
            }
        }
        result
    }
}

struct Launcher(Child);
impl Drop for Launcher {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn random_hex(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|e| e.to_string())?;
    Ok(buffer.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|e| e.to_string())
}
fn binary(dir: &Path, name: &str) -> PathBuf {
    dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}
fn launch(service: &Path, session: &Path) -> Result<Child, String> {
    let mut command = Command::new(service);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .arg("--session")
        .arg(session)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())
}
#[cfg(target_os = "linux")]
fn elevate(service: &Path, session: &Path) -> Result<Child, String> {
    let has_pkexec = env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| dir.join("pkexec").is_file()));
    let mut command = if has_pkexec {
        Command::new("pkexec")
    } else {
        let mut command = Command::new("sudo");
        command.arg("-A");
        command
    };
    command.arg(service).arg("--session").arg(session).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map_err(|_| "Cannot request administrator access. Install polkit and an authentication agent (or configure sudo askpass).".into())
}
#[cfg(target_os = "macos")]
fn elevate(service: &Path, session: &Path) -> Result<Child, String> {
    let shell_quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let shell = format!(
        "{} --session {}",
        shell_quote(&service.to_string_lossy()),
        shell_quote(&session.to_string_lossy())
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        shell.replace('\\', "\\\\").replace('"', "\\\"")
    );
    Command::new("osascript")
        .args(["-e", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())
}
#[cfg(target_os = "windows")]
fn elevate(service: &Path, session: &Path) -> Result<Child, String> {
    use std::os::windows::process::CommandExt;
    let service = service.to_string_lossy().replace('\'', "''");
    let session = session.to_string_lossy().replace('\'', "''");
    let script = format!("$ErrorActionPreference='Stop'; $p=Start-Process -FilePath '{service}' -ArgumentList '--session', '\"{session}\"' -Verb RunAs -WindowStyle Hidden -PassThru -Wait; exit $p.ExitCode");
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())
}
fn read_message(reader: &mut impl BufRead) -> io::Result<Value> {
    use std::io::Read;
    let mut line = String::new();
    let count = reader
        .take((config::MAX_CONFIG_BYTES * 5) as u64)
        .read_line(&mut line)?;
    if count == 0 || !line.ends_with('\n') {
        return Err(io::Error::other("Incomplete control message"));
    }
    serde_json::from_str(&line).map_err(io::Error::other)
}
pub fn run_stdio() -> io::Result<()> {
    let output = Arc::new(Mutex::new(io::stdout()));
    let events = output.clone();
    let controller = Controller::new(
        env::current_exe()?.parent().unwrap().to_path_buf(),
        Arc::new(move |status| {
            let mut output = events.lock().unwrap();
            let _ = writeln!(output, "{}", json!({"event":"status","status":status}));
            let _ = output.flush();
        }),
    );
    let mut input = io::stdin().lock();
    while let Ok(request) = read_message(&mut input) {
        let id = request["id"].clone();
        let text = request["text"].as_str().unwrap_or("");
        let result = match request["type"].as_str() {
            Some("validate") => json!(config::validate(text)),
            Some("connect") => match controller.connect(text) {
                Ok(()) => json!({"ok":true}),
                Err(error) => json!({"ok":false,"error":error}),
            },
            Some("disconnect") => {
                controller.disconnect();
                json!({"ok":true})
            }
            Some("status") => json!(controller.status()),
            _ => json!({"ok":false,"error":"Unknown controller command"}),
        };
        let mut output = output.lock().unwrap();
        writeln!(output, "{}", json!({"id":id,"result":result}))?;
        output.flush()?;
    }
    controller.disconnect();
    let deadline = Instant::now() + Duration::from_secs(14);
    while controller.state.lock().unwrap().stop.is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generates_independent_full_length_secrets() {
        let a = random_hex(32).unwrap();
        let b = random_hex(32).unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
    #[test]
    fn idle_disconnect_is_idempotent() {
        let controller = Controller::new(PathBuf::new(), Arc::new(|_| {}));
        controller.disconnect();
        controller.disconnect();
        assert_eq!(controller.status(), Status::default());
    }
    #[test]
    fn invalid_config_keeps_the_connection_idle() {
        let controller = Controller::new(PathBuf::new(), Arc::new(|_| {}));
        assert!(controller.connect("{}").unwrap_err().contains("inbounds"));
        assert_eq!(controller.status(), Status::default());
    }
    #[test]
    fn writes_private_session_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session");
        private_write(&path, b"session").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"session");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
