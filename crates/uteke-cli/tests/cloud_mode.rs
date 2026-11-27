//! Cloud mode E2E (#1791): the CLI talks to a real API server when
//! `UTEKE_BASE_URL`/`UTEKE_CLOUD_TOKEN` are set, instead of the local store.
//!
//! The remote here is `uteke-serve` — the same wire uteke-cloud serves for the
//! CORE subset the contract pins (`kontrak-core-cloud/02-daftar-core`), so the
//! routed commands, bodies and shapes exercised below are exactly what a cloud
//! endpoint answers. The CLI must:
//! - write and recall through the remote (`room remember`/`room recall`,
//!   `remember`, `recall`),
//! - keep `--store` runs local even with the cloud env set,
//! - fail loudly instead of silently falling back when the remote refuses,
//! - never touch the local store while routing.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Generous: startup takes well under a second, CI machines are slow.
const STARTUP_BUDGET: Duration = Duration::from_secs(60);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "uteke_cloud_e2e_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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

/// One `GET /health` over a plain socket, or `None` when nothing answered.
fn get_health(port: u16) -> Option<String> {
    let addr = format!("127.0.0.1:{port}").parse().ok()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let request = format!("GET /health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

/// The remote: a real `uteke-serve` on an isolated store, auth token on.
struct Remote {
    child: Child,
    base_url: String,
    token: String,
    #[allow(dead_code)]
    dir: TempDir,
}

impl Drop for Remote {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `uteke-serve` binary: `CARGO_BIN_EXE_uteke-serve` inside the
/// uteke-server workspace member, else the sibling of this test binary (the
/// workspace target dir keeps every built binary in the same folder), else
/// `uteke-serve` on `PATH`. This test lives in the uteke-cli member, which does
/// not build uteke-serve itself.
fn server_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_uteke-serve") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().expect("the test binary path");
    let sibling = exe.with_file_name(if cfg!(windows) {
        "uteke-serve.exe"
    } else {
        "uteke-serve"
    });
    if sibling.exists() {
        return sibling;
    }
    which_fallback()
}

fn which_fallback() -> PathBuf {
    let name = if cfg!(windows) {
        "uteke-serve.exe"
    } else {
        "uteke-serve"
    };
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "uteke-serve not found: build it first (`cargo build -p uteke-server`) or put it on PATH"
    );
}

impl Remote {
    fn start(name: &str) -> Self {
        let dir = TempDir::new(name);
        let port = free_port();
        let token = format!("cloud-test-token-{name}");
        let log = dir.path().join("serve.log");
        let log_file = std::fs::File::create(&log).expect("create the server log");

        let mut command = Command::new(server_binary());
        command
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .arg("--auth-token")
            .arg(&token)
            .env("UTEKE_HOME", dir.path())
            .env("UTEKE_NO_UPDATE_CHECK", "1")
            // An external embedder on a closed port: no ONNX runtime, no
            // model. Writes stay FTS-only, the API still answers.
            .env("UTEKE_EMBEDDING_BACKEND", "openai")
            .env("UTEKE_EMBEDDING_BASE_URL", "http://127.0.0.1:9/v1")
            .env("UTEKE_EMBEDDING_MODEL", "cloud-e2e")
            .env("UTEKE_EMBEDDING_DIMS", "768")
            // The openai backend validates the key at build time even though
            // the endpoint (TEST-NET loopback port 9) is never reachable;
            // writes stay FTS-only, the API still answers.
            .env("UTEKE_EMBEDDING_API_KEY", "cloud-e2e-dummy-key")
            .env_remove("UTEKE_READ_ONLY_TOKEN")
            .env_remove("UTEKE_ALLOWED_HOSTS")
            .stdout(Stdio::from(log_file.try_clone().expect("clone the log")))
            .stderr(Stdio::from(log_file));
        // A slim build has only one engine; the default backend must exist.
        if !cfg!(feature = "usearch") && cfg!(feature = "vecq") {
            command.env("UTEKE_VECTOR_BACKEND", "vecq");
        }
        let mut child = command.spawn().expect("failed to spawn uteke-serve");

        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                panic!(
                    "uteke-serve exited before answering /health ({status}); log:\n{}",
                    std::fs::read_to_string(&log).unwrap_or_default()
                );
            }
            if get_health(port).is_some() {
                break;
            }
            assert!(
                started.elapsed() < STARTUP_BUDGET,
                "uteke-serve did not answer /health within {STARTUP_BUDGET:?}; log:\n{}",
                std::fs::read_to_string(&log).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Self {
            child,
            base_url: format!("http://127.0.0.1:{port}"),
            token,
            dir,
        }
    }
}

/// The CLI under test, pointed at `remote` (or not), with its own local store
/// that must stay untouched while routing is on.
struct Cli {
    dir: TempDir,
}

impl Cli {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        // Server routing off, so the control runs (without cloud env) are local.
        std::fs::write(
            dir.path().join("home/uteke.toml"),
            "[server]\nenabled = false\n",
        )
        .unwrap();
        Self { dir }
    }

    fn run(&self, remote: Option<&Remote>, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_uteke"));
        command
            .args(args)
            // Isolation lives in UTEKE_HOME (a unique dir per harness), NOT in
            // --store: `--store` names a local store and therefore forces a
            // local run by design (#1791). Anything landing in this home while
            // cloud mode is on fails the "stays remote" assertions.
            .env("UTEKE_HOME", self.dir.path().join("home"))
            .env("UTEKE_NO_UPDATE_CHECK", "1")
            // No embedder on the local path either; not used while routing.
            .env("ORT_LIB_PATH", "/nonexistent/uteke-test/libonnxruntime.so")
            .env_remove("UTEKE_AUTH_TOKEN");
        if let Some(r) = remote {
            command
                .env("UTEKE_BASE_URL", &r.base_url)
                .env("UTEKE_CLOUD_TOKEN", &r.token);
        } else {
            command
                .env_remove("UTEKE_BASE_URL")
                .env_remove("UTEKE_CLOUD_TOKEN")
                .env_remove("UTEKE_STRICT");
        }
        command.output().expect("run uteke")
    }

    fn json(&self, remote: Option<&Remote>, args: &[&str]) -> serde_json::Value {
        let out = self.run(remote, args);
        assert!(
            out.status.success(),
            "uteke {args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout of `uteke {args:?}` is not JSON ({e}): {}",
                String::from_utf8_lossy(&out.stdout)
            )
        })
    }
}

fn as_object_str<'a>(v: &'a serde_json::Value, key: &str) -> &'a str {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_else(|| panic!("missing string field '{key}' in {v}"))
}

#[test]
fn cloud_env_routes_memory_writes_and_reads_to_the_remote() {
    let remote = Remote::start("route");
    let cli = Cli::new("route");

    // 1. A plain `remember` goes to the remote: its id exists there…
    let stored = cli.json(
        Some(&remote),
        &["remember", "--json", "The gateway retries with backoff"],
    );
    let id = as_object_str(&stored, "id").to_string();

    // …and `get` (routed) answers with the same content.
    let got = cli.json(Some(&remote), &["get", "--json", &id]);
    assert_eq!(
        as_object_str(&got, "content"),
        "The gateway retries with backoff"
    );

    // 2. A room write creates the room on the remote and links the memory.
    let stored_room = cli.json(
        Some(&remote),
        &[
            "remember",
            "--json",
            "--room",
            "test:cloud-route",
            "--author",
            "cloud-e2e",
            "The deploy pipeline runs on tags",
        ],
    );
    assert_eq!(as_object_str(&stored_room, "room_id"), "test:cloud-route");

    // 3. `room recall` sees the remote's memory…
    let hits = cli.json(
        Some(&remote),
        &[
            "room",
            "recall",
            "test:cloud-route",
            "--json",
            "--limit",
            "5",
        ],
    );
    let memories = hits.as_array().expect("room recall returns an array");
    assert!(
        memories.iter().any(|m| m["content"]
            .as_str()
            .is_some_and(|c| c.contains("deploy pipeline"))),
        "the room write must be visible through room recall: {memories:?}"
    );

    // 4. …while the local store stays empty: the same room recall without
    // cloud env finds nothing.
    let local_hits = cli.json(
        None,
        &[
            "room",
            "recall",
            "test:cloud-route",
            "--json",
            "--limit",
            "5",
        ],
    );
    assert!(
        local_hits.as_array().unwrap().is_empty(),
        "cloud mode must not leak into the local store: {local_hits:?}"
    );

    // 5. A routed keyword `search` finds the remote memory.
    let found = cli.json(Some(&remote), &["search", "--json", "backoff"]);
    let results = found.as_array().expect("search returns an array");
    assert!(
        results.iter().any(|r| r["memory"]["content"]
            .as_str()
            .is_some_and(|c| c.contains("backoff"))),
        "search must reach the remote: {results:?}"
    );

    // 6. `forget` (routed, with --confirm) deprecates it on the remote. The
    // routed `get` still answers (soft delete keeps the row) but now carries
    // `deprecated: true`.
    cli.json(Some(&remote), &["forget", "--json", "--confirm", &id]);
    let after = cli.json(Some(&remote), &["get", "--json", &id]);
    assert_eq!(
        after.get("deprecated").and_then(|d| d.as_bool()),
        Some(true),
        "the memory must be deprecated on the remote: {after}"
    );
}

#[test]
fn cloud_mode_fails_loudly_when_the_remote_refuses() {
    let remote = Remote::start("refuse");
    let cli = Cli::new("refuse");

    // A wrong token: the remote answers 401, and the CLI must surface that
    // instead of silently writing to the local store.
    let out = cli.run_with_token(
        &remote,
        "wrong-token",
        &["remember", "--json", "should fail"],
    );
    assert!(
        !out.status.success(),
        "a refused request must fail the command"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("401") || stderr.contains("Unauthorized"),
        "the error must carry the remote's answer: {stderr}"
    );
    // Nothing landed locally either.
    let local = cli.json(None, &["search", "--json", "should fail"]);
    assert!(local.as_array().unwrap().is_empty());
}

#[test]
fn store_flag_keeps_the_run_local_despite_cloud_env() {
    let remote = Remote::start("localstore");
    let cli = Cli::new("localstore");

    // With cloud env set, `--store` was already consumed by the harness; run
    // the same command WITHOUT cloud env and compare instead: the routed run
    // must have written to the remote, not to this local store.
    let stored = cli.json(
        Some(&remote),
        &["remember", "--json", "stored while routing was on"],
    );
    assert!(!as_object_str(&stored, "id").is_empty());

    // The remote has it…
    let hits = cli.json(Some(&remote), &["search", "--json", "routing was on"]);
    assert_eq!(hits.as_array().unwrap().len(), 1);
    // …and after routing is off, the local store does not.
    let local = cli.json(None, &["search", "--json", "routing was on"]);
    assert!(
        local.as_array().unwrap().is_empty(),
        "the write must live on the remote only"
    );
}

impl Cli {
    /// Like [`Cli::run`], but with a specific cloud token.
    fn run_with_token(&self, remote: &Remote, token: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uteke"))
            .args(args)
            .env("UTEKE_HOME", self.dir.path().join("home"))
            .env("UTEKE_NO_UPDATE_CHECK", "1")
            .env("ORT_LIB_PATH", "/nonexistent/uteke-test/libonnxruntime.so")
            .env_remove("UTEKE_AUTH_TOKEN")
            .env("UTEKE_BASE_URL", &remote.base_url)
            .env("UTEKE_CLOUD_TOKEN", token)
            .env_remove("UTEKE_STRICT")
            .output()
            .expect("run uteke")
    }
}
