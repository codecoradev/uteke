//! `uteke remember --room` warns when it creates a room whose id is not of the
//! form `category:title` (#1715, stage 1), as `uteke room create` does.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("uteke_room_id_warn_{name}_{}", std::process::id()));
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

    fn ok(&self, args: &[&str]) -> Output {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "uteke {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn warning_for(id: &str) -> String {
    uteke_core::room_naming::room_id_warning(id).expect("non-conventional id")
}

#[test]
fn json_carries_warnings_only_when_the_write_creates_a_plain_room() {
    let store = Store::new("json");
    let json = |args: &[&str]| -> serde_json::Value {
        serde_json::from_slice(&store.ok(args).stdout).expect("JSON on stdout")
    };

    let first = json(&["--json", "remember", "first note", "--room", "plain-room"]);
    assert_eq!(
        first["warnings"],
        serde_json::json!([warning_for("plain-room")]),
        "{first}"
    );
    let second = json(&["--json", "remember", "second note", "--room", "plain-room"]);
    assert!(second.get("warnings").is_none(), "existing room: {second}");
    let conventional = json(&["--json", "remember", "third note", "--room", "ops:plain"]);
    assert!(conventional.get("warnings").is_none(), "{conventional}");
    let no_room = json(&["--json", "remember", "fourth note"]);
    assert!(no_room.get("warnings").is_none(), "{no_room}");
}

#[test]
fn human_output_prints_the_warning_on_stderr() {
    let store = Store::new("human");
    let out = store.ok(&["remember", "a note", "--room", "plain-room"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("Warning: {}", warning_for("plain-room"))),
        "{stderr}"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("Warning"),
        "the warning stays off stdout"
    );

    let out = store.ok(&["remember", "another note", "--room", "plain-room"]);
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("category:title"),
        "existing room is not warned"
    );
}
