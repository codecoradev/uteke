//! `uteke room link` adds an existing memory to a room without copying it
//! (#1589).

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("uteke_room_link_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        // Server routing off, so every command runs against the local store.
        std::fs::write(dir.join("home/uteke.toml"), "[server]\nenabled = false\n").unwrap();
        Self { dir }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uteke"))
            .arg("--store")
            .arg(self.dir.join("store"))
            .args(args)
            .env("UTEKE_HOME", self.dir.join("home"))
            .env("UTEKE_NO_UPDATE_CHECK", "1")
            // No embedder: memories are stored FTS-only, commands still run.
            .env("ORT_LIB_PATH", "/nonexistent/uteke-test/libonnxruntime.so")
            .env_remove("UTEKE_AUTH_TOKEN")
            .output()
            .expect("run uteke")
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "uteke {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "uteke {args:?} printed no JSON ({e}): {}",
                String::from_utf8_lossy(&out.stdout)
            )
        })
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn room_link_adds_an_existing_memory_once() {
    let store = Store::new("link");
    store.json(&["--json", "room", "create", "team"]);
    let id = store.json(&["--json", "remember", "linked by prefix"])["id"]
        .as_str()
        .unwrap()
        .to_string();

    let first = store.json(&[
        "--json",
        "room",
        "link",
        &id[..8],
        "team",
        "--author",
        "alice",
    ]);
    assert_eq!(first["linked"], true, "{first}");
    assert_eq!(first["memory_id"], id.as_str());
    let again = store.json(&["--json", "room", "link", &id, "team"]);
    assert_eq!(again["already_linked"], true, "{again}");

    let out = store.run(&["room", "link", &id, "no-such-room"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Room not found: no-such-room"));
    let out = store.run(&["room", "link", "ffffffff", "team"]);
    assert!(!out.status.success());
}
