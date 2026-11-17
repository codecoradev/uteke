//! Startup test for the `uteke-serve` binary: it must open the store, bind its
//! port and answer `GET /health`.
//!
//! v0.24.0 shipped a self-deadlock between "Lifecycle config loaded" and the
//! bind: `main` took the `Mutex<Uteke>` twice inside one `let ctx = ...`
//! statement, the first guard (a temporary) was still alive at the second
//! `lock()`, and the process sat on a futex forever. It never listened, never
//! answered `/health` (so Docker reported it unhealthy), ignored SIGTERM as PID 1
//! and kept the index file lock. No unit test starts `main`, so nothing noticed.
//! This test starts the real binary and waits for `/health`.
//!
//! It uses an external-embedder backend pointed at a closed port, so it needs no
//! ONNX runtime and no model download; the server must start without reaching it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Generous: startup takes well under a second, CI machines are slow.
const STARTUP_BUDGET: Duration = Duration::from_secs(60);

/// A fresh data directory, removed on drop.
struct TempHome(PathBuf);

impl TempHome {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("uteke-startup-test-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the test data dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The server process, killed and reaped on drop so a hang never leaks it.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A port nothing listens on right now. Another process could take it before
/// the server binds; that would fail the test loudly, not hide a hang.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("local address")
        .port()
}

/// One `GET /health` over a plain socket: the status line and the body, or
/// `None` when nothing answered.
fn get_health(port: u16) -> Option<String> {
    let addr = format!("127.0.0.1:{port}").parse().ok()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    // The API's host guard only accepts a loopback Host with the right port.
    let request = format!("GET /health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

#[test]
fn server_starts_listens_and_answers_health() {
    let home = TempHome::new();
    let log = home.path().join("serve.log");
    let log_file = std::fs::File::create(&log).expect("create the server log");
    let port = free_port();

    let mut command = Command::new(env!("CARGO_BIN_EXE_uteke-serve"));
    command
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .env("UTEKE_HOME", home.path())
        .env("UTEKE_NO_UPDATE_CHECK", "1")
        // An external embedder on a closed port: no ONNX runtime, no model.
        .env("UTEKE_EMBEDDING_BACKEND", "openai")
        .env("UTEKE_EMBEDDING_BASE_URL", "http://127.0.0.1:9/v1")
        .env("UTEKE_EMBEDDING_MODEL", "startup-test")
        .env("UTEKE_EMBEDDING_DIMS", "768")
        .env_remove("UTEKE_AUTH_TOKEN")
        .env_remove("UTEKE_READ_ONLY_TOKEN")
        .env_remove("UTEKE_ALLOWED_HOSTS")
        .stdout(Stdio::from(
            log_file.try_clone().expect("clone the log handle"),
        ))
        .stderr(Stdio::from(log_file));
    // A slim build has only one engine; the default backend must exist.
    if !cfg!(feature = "usearch") && cfg!(feature = "vecq") {
        command.env("UTEKE_VECTOR_BACKEND", "vecq");
    }
    let mut server = Server(command.spawn().expect("failed to spawn uteke-serve"));

    let started = Instant::now();
    let answer = loop {
        if let Some(status) = server.0.try_wait().expect("poll the server process") {
            panic!(
                "uteke-serve exited before answering /health ({status}); log:\n{}",
                std::fs::read_to_string(&log).unwrap_or_default()
            );
        }
        if let Some(answer) = get_health(port) {
            break answer;
        }
        assert!(
            started.elapsed() < STARTUP_BUDGET,
            "uteke-serve did not answer /health within {STARTUP_BUDGET:?}: it is hung at \
             startup (it must reach \"Uteke server listening\"); log:\n{}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(100));
    };

    let status_line = answer.lines().next().unwrap_or_default();
    assert!(
        status_line.ends_with(" 200 OK") || status_line.contains(" 200"),
        "GET /health must be 200, got: {status_line}"
    );
    assert!(
        answer.contains("\"status\":\"ok\""),
        "GET /health must report status ok, got: {answer}"
    );
}
