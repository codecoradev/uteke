//! `uteke remember/update --valid-from/--valid-until` (#1598): the caller's
//! validity interval lands in the columns `list --at` reads, and a bad
//! interval fails before anything is written.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "uteke_validity_flags_{name}_{}",
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
fn remember_writes_the_interval_and_list_at_reads_it() {
    let store = Store::new("remember");
    let stored = store.json(&[
        "--json",
        "remember",
        "the lease runs until 2030",
        "--valid-from",
        "2025-01-01T00:00:00Z",
        "--valid-until",
        "2030-01-01T00:00:00Z",
    ]);
    let id = stored["id"].as_str().unwrap().to_string();
    let memory = store.json(&["--json", "get", &id]);
    assert_eq!(memory["valid_from"], "2025-01-01T00:00:00Z", "{memory}");
    assert_eq!(memory["valid_until"], "2030-01-01T00:00:00Z", "{memory}");

    let inside = store.json(&["--json", "list", "--at", "2029-06-01T00:00:00Z"]);
    assert_eq!(ids(&inside), vec![id.clone()]);
    let after = store.json(&["--json", "list", "--at", "2030-06-01T00:00:00Z"]);
    assert!(ids(&after).is_empty(), "{after}");
}

#[test]
fn a_bad_interval_stores_nothing() {
    let store = Store::new("refused");
    for args in [
        // The write time is the default start, so a past end needs a start.
        &[
            "remember",
            "ended fact",
            "--valid-until",
            "2020-01-01T00:00:00Z",
        ][..],
        &["remember", "ended fact", "--valid-from", "yesterday"][..],
        &[
            "remember",
            "ended fact",
            "--valid-from",
            "2021-01-01T00:00:00Z",
            "--valid-until",
            "2020-01-01T00:00:00Z",
        ][..],
    ] {
        let out = store.run(args);
        assert!(!out.status.success(), "uteke {args:?} should fail");
    }
    let listed = store.json(&["--json", "list"]);
    assert!(ids(&listed).is_empty(), "{listed}");

    // A past end with an explicit start is fine.
    store.json(&[
        "--json",
        "remember",
        "ended fact",
        "--valid-from",
        "2019-01-01T00:00:00Z",
        "--valid-until",
        "2020-01-01T00:00:00Z",
    ]);
}

#[test]
fn update_sets_the_interval_and_a_refusal_changes_nothing() {
    let store = Store::new("update");
    let id = store.json(&["--json", "remember", "the port is 8767"])["id"]
        .as_str()
        .unwrap()
        .to_string();
    let updated = store.json(&[
        "--json",
        "update",
        &id,
        "--valid-until",
        "2031-01-01T00:00:00Z",
    ]);
    assert_eq!(updated["valid_until"], "2031-01-01T00:00:00Z", "{updated}");
    let importance = updated["importance"].clone();

    // A start after the stored end is refused, and the importance given in
    // the same call is not applied either.
    let out = store.run(&[
        "update",
        &id,
        "--valid-from",
        "2032-01-01T00:00:00Z",
        "--importance",
        "0.9",
    ]);
    assert!(!out.status.success());
    let memory = store.json(&["--json", "get", &id]);
    assert_eq!(memory["importance"], importance, "{memory}");
    assert_eq!(memory["valid_until"], "2031-01-01T00:00:00Z", "{memory}");
}
