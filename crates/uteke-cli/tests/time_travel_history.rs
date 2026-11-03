//! `uteke list --at T --history` (#1598): memories forgotten after `T` are
//! listed as they were at `T`; without `--history` the output is unchanged.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "uteke_time_travel_history_{name}_{}",
            std::process::id()
        ));
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

fn ids(listed: &serde_json::Value) -> Vec<String> {
    listed
        .as_array()
        .expect("list prints an array")
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_history_shows_what_was_believed_then() {
    let store = Store::new("list");
    let id = store.json(&["--json", "remember", "the port is 8080"])["id"]
        .as_str()
        .unwrap()
        .to_string();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let between = chrono::Utc::now().to_rfc3339();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let out = store.run(&["forget", &id, "--reason", "outdated", "--confirm"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let plain = store.json(&["--json", "list", "--at", &between]);
    assert!(ids(&plain).is_empty(), "{plain}");
    let history = store.json(&["--json", "list", "--at", &between, "--history"]);
    assert_eq!(ids(&history), vec![id.clone()]);
    let now = chrono::Utc::now().to_rfc3339();
    let after = store.json(&["--json", "list", "--at", &now, "--history"]);
    assert!(ids(&after).is_empty(), "{after}");
}

#[test]
fn history_needs_at() {
    let store = Store::new("needs_at");
    for args in [
        &["list", "--history"][..],
        &["recall", "port", "--history"][..],
    ] {
        let out = store.run(args);
        assert!(!out.status.success(), "uteke {args:?} should fail");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("--at"), "{err}");
    }
}
