//! `uteke remember --type` on a local store (#1439): an explicit type is stored,
//! an omitted one is inferred from the content.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("uteke_remember_type_{name}_{}", std::process::id()));
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
fn an_explicit_type_is_stored_and_an_omitted_one_is_inferred() {
    let store = Store::new("type");
    let get = |args: &[&str]| {
        let id = store.json(args)["id"].as_str().unwrap().to_string();
        store.json(&["--json", "get", &id])["memory_type"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        get(&["--json", "remember", "plain words", "--type", "procedure"]),
        "procedure"
    );
    assert_eq!(
        get(&["--json", "remember", "We decided to use SQLite"]),
        "decision"
    );
    assert_eq!(get(&["--json", "remember", "plain words again"]), "fact");
    let out = store.run(&["remember", "x", "--type", "bogus"]);
    assert!(!out.status.success());
}
