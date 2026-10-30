//! `uteke remember --author` without a room (#1427 phase 2): the author is
//! recorded on the memory, shown by `get` and `list`, and an invalid one fails
//! before anything is stored.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("uteke_author_flag_{name}_{}", std::process::id()));
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
fn author_is_recorded_without_a_room_and_shown_by_get_and_list() {
    let store = Store::new("recorded");
    let stored = store.json(&[
        "--json",
        "remember",
        "attributed note",
        "--author",
        "  claude-code ",
    ]);
    assert_eq!(stored["author"], "claude-code", "{stored}");
    let id = stored["id"].as_str().unwrap().to_string();

    let memory = store.json(&["--json", "get", &id]);
    assert_eq!(memory["author"], "claude-code", "{memory}");

    let plain = store.json(&["--json", "remember", "plain note"]);
    assert!(plain.get("author").is_none(), "{plain}");

    let listed = store.json(&["--json", "list"]);
    let authors: Vec<&serde_json::Value> = listed
        .as_array()
        .expect("list prints an array")
        .iter()
        .filter(|m| m["author"] == "claude-code")
        .collect();
    assert_eq!(authors.len(), 1, "{listed}");
}

#[test]
fn an_invalid_author_fails_before_anything_is_stored() {
    let store = Store::new("invalid");
    for bad in ["", "   ", &"y".repeat(129)] {
        let out = store.run(&["--json", "remember", "must not be stored", "--author", bad]);
        assert!(!out.status.success(), "{bad:?} was accepted");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("--author"), "{err}");
    }
    let listed = store.json(&["--json", "list"]);
    assert_eq!(listed.as_array().unwrap().len(), 0, "{listed}");
}
