//! `uteke forget <id>` on a local store resolves ids like `DELETE /forget` and
//! MCP (#1439): a prefix works, and an id that is not stored is an error
//! instead of a silent "forgotten".

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("uteke_forget_id_{name}_{}", std::process::id()));
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
fn forget_resolves_a_prefix_and_refuses_an_unknown_id() {
    let store = Store::new("resolve");
    let stored = store.json(&["--json", "remember", "to be forgotten"]);
    let id = stored["id"].as_str().unwrap().to_string();

    // An id that is not stored fails and stores nothing new.
    let absent = "00000000-0000-4000-8000-000000000000";
    let out = store.run(&["forget", absent, "--confirm"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&format!("Memory not found: {absent}")),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // An unknown prefix too.
    let out = store.run(&["forget", "ffffffff", "--confirm"]);
    assert!(!out.status.success());

    // A prefix of a stored id works and reports the full id.
    let out = store.json(&["--json", "forget", &id[..8], "--confirm"]);
    assert_eq!(out["forgotten"], id.as_str(), "{out}");
}
