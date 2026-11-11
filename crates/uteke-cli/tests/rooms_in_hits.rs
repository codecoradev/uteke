//! #1713: the `rooms` field of a memory hit is CORE on the CLI `--json` of
//! `recall`, `search` and `room recall <id> <query>`: sorted by room id, the
//! title or (blank/missing title) the id as `room_name`, and no key at all for
//! a memory in no room.

use std::path::PathBuf;
use std::process::Command;

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("uteke_rooms_in_hits_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        // Server routing off, so every command runs against the local store.
        std::fs::write(dir.join("home/uteke.toml"), "[server]\nenabled = false\n").unwrap();
        Self { dir }
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let out = Command::new(env!("CARGO_BIN_EXE_uteke"))
            .arg("--store")
            .arg(self.dir.join("store"))
            .arg("--json")
            .args(args)
            .env("UTEKE_HOME", self.dir.join("home"))
            .env("UTEKE_NO_UPDATE_CHECK", "1")
            // No embedder: memories are stored FTS-only, commands still run.
            .env("ORT_LIB_PATH", "/nonexistent/uteke-test/libonnxruntime.so")
            .env_remove("UTEKE_AUTH_TOKEN")
            .output()
            .expect("run uteke");
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
fn recall_search_and_room_recall_json_carry_the_exact_rooms() {
    let s = Store::new();
    s.json(&["room", "create", "team:beta", "--title", "Beta room"]);
    s.json(&["room", "create", "project:alpha"]);
    let id = s.json(&[
        "remember",
        "orchid ledger kept in two rooms",
        "--room",
        "team:beta",
    ])["id"]
        .as_str()
        .unwrap()
        .to_string();
    s.json(&["room", "link", &id, "project:alpha"]);
    s.json(&["remember", "orchid ledger kept in no room"]);
    let expected = serde_json::json!([
        {"room_id": "project:alpha", "room_name": "project:alpha"},
        {"room_id": "team:beta", "room_name": "Beta room"},
    ]);

    for (what, out) in [
        (
            "recall",
            s.json(&["recall", "orchid ledger", "--strategy", "fts5"]),
        ),
        ("search", s.json(&["search", "orchid ledger"])),
        (
            "room recall",
            s.json(&["room", "recall", "project:alpha", "orchid ledger"]),
        ),
    ] {
        let hits = out
            .as_array()
            .unwrap_or_else(|| panic!("{what}: array expected: {out}"));
        let mut roomed = 0;
        for hit in hits {
            let hit_id = hit["memory"]["id"]
                .as_str()
                .or_else(|| hit["memory_id"].as_str())
                .or_else(|| hit["id"].as_str())
                .unwrap_or_else(|| panic!("{what}: hit without id: {hit}"));
            if hit_id == id {
                assert_eq!(hit["rooms"], expected, "{what}: {hit}");
                roomed += 1;
            } else {
                assert!(hit.get("rooms").is_none(), "{what}: {hit}");
            }
        }
        assert_eq!(
            roomed, 1,
            "{what}: the roomed memory is returned once: {out}"
        );
    }
}
