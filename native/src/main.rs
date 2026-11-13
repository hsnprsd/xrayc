use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use xrayc_service::core::EmbeddedCore;

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

fn run_session(mut reader: BufReader<TcpStream>, events: Events, dir: &Path) -> io::Result<()> {
    let request = message(&mut reader)?;
    if request["type"] != "start" || !request["config"].is_object() {
        return Err(io::Error::other("Expected Xray configuration"));
    }
    // Revalidate at the privileged boundary and sanitize logging even when a
    // session is invoked directly rather than through the desktop controller.
    let config =
        xrayc_service::config::parse(&request["config"].to_string()).map_err(io::Error::other)?;
    let config = xrayc_service::config::runtime_config(&config);
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
    let mut core = EmbeddedCore::load(dir)?;
    cancelled(&stop)?;
    // Start returns only after Xray's features and inbounds are started. No log
    // matching, child process or plaintext temporary configuration is needed.
    core.start(&config)?;
    if stop.load(Ordering::Relaxed) {
        core.stop()?;
        return cancelled(&stop);
    }
    send(&events, json!({"type":"ready"}));
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
    }
    core.stop()?;
    send(&events, json!({"type":"stopped"}));
    Ok(())
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn execute() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() == 2 && args[1] == "--controller" {
        return xrayc_service::controller::run_stdio();
    }
    let dir = env::current_exe()?.parent().unwrap().to_path_buf();
    if args.len() == 2 && args[1] == "--version" {
        let core = EmbeddedCore::load(&dir)?;
        println!(
            "xrayc-service {} (embedded Xray {})",
            env!("CARGO_PKG_VERSION"),
            core.version()
        );
        return Ok(());
    }
    if args.len() == 3 && args[1] == "--check-config" {
        let text = fs::read_to_string(&args[2])?;
        let config = xrayc_service::config::parse(&text).map_err(io::Error::other)?;
        env::set_current_dir(&dir)?;
        let core = EmbeddedCore::load(&dir)?;
        core.validate(&xrayc_service::config::runtime_config(&config))?;
        println!("Configuration accepted by embedded Xray {}", core.version());
        return Ok(());
    }
    if args.len() != 3 || args[1] != "--session" {
        return Err(io::Error::other("Usage: xrayc-service --controller | --session <private-session-file> | --check-config <file> | --version"));
    }
    run(&args[2], &dir)
}

fn run(path: &str, dir: &Path) -> io::Result<()> {
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
    // Preserve the old executable's relative certificate/asset path semantics.
    env::set_current_dir(dir)?;
    let result = run_session(BufReader::new(stream), events.clone(), dir);
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
}
