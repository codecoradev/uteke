//! `uteke namespace migrate --map <file>` (#1427 phase 3): a dry run writes
//! nothing, `--apply` writes an export next to the store and then moves the
//! namespace, and a second `--apply` changes nothing. The logic is tested in
//! uteke-core; this covers the command wiring.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "uteke_namespace_migrate_{name}_{}",
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

fn exports(store: &Store) -> Vec<PathBuf> {
    std::fs::read_dir(store.dir.join("store"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("namespace-migrate-"))
        })
        .collect()
}

#[test]
fn dry_run_then_apply_then_rerun() {
    let store = Store::new("flow");
    let id = store.json(&[
        "--json",
        "--namespace",
        "research-x",
        "remember",
        "migrate me",
    ])["id"]
        .as_str()
        .unwrap()
        .to_string();
    let map = store.dir.join("map.toml");
    std::fs::write(
        &map,
        "[map.\"research-x\"]\nroom = \"research-x\"\nauthor = \"agent-a\"\n",
    )
    .unwrap();
    let map = map.to_str().unwrap();

    let dry = store.json(&["--json", "namespace", "migrate", "--map", map]);
    assert_eq!(dry["applied"], false, "{dry}");
    assert_eq!(dry["namespaces"][0]["memories"], 1, "{dry}");
    assert_eq!(dry["namespaces"][0]["room_created"], true, "{dry}");
    assert_eq!(
        store.json(&["--json", "get", &id])["namespace"],
        "research-x"
    );
    assert!(exports(&store).is_empty(), "a dry run wrote an export");

    let applied = store.json(&["--json", "namespace", "migrate", "--map", map, "--apply"]);
    assert_eq!(applied["applied"], true, "{applied}");
    let written = exports(&store);
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(
        applied["export_path"].as_str().map(PathBuf::from),
        Some(written[0].clone())
    );
    let moved = store.json(&["--json", "get", &id]);
    assert_eq!(moved["namespace"], "default", "{moved}");
    assert_eq!(moved["author"], "agent-a", "{moved}");

    let again = store.json(&["--json", "namespace", "migrate", "--map", map, "--apply"]);
    assert_eq!(again["namespaces"][0]["found"], false, "{again}");
    assert!(again["export_path"].is_null(), "{again}");
    assert_eq!(exports(&store).len(), 1);
}

#[test]
fn a_bad_map_fails_and_a_drop_needs_allow_drop() {
    let store = Store::new("guards");
    store.json(&["--json", "--namespace", "junk", "remember", "drop me"]);
    let map = store.dir.join("map.toml");

    std::fs::write(&map, "[map.default]\ntag = \"x\"\n").unwrap();
    let out = store.run(&["namespace", "migrate", "--map", map.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("default"));

    std::fs::write(&map, "[map.junk]\ndrop = true\n").unwrap();
    let out = store.run(&[
        "namespace",
        "migrate",
        "--map",
        map.to_str().unwrap(),
        "--apply",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--allow-drop"));
    assert!(exports(&store).is_empty());

    let done = store.json(&[
        "--json",
        "namespace",
        "migrate",
        "--map",
        map.to_str().unwrap(),
        "--apply",
        "--allow-drop",
    ]);
    assert_eq!(done["namespaces"][0]["memories"], 1, "{done}");
    let listed = store.json(&["--json", "--namespace", "junk", "list"]);
    assert_eq!(listed.as_array().unwrap().len(), 0, "{listed}");
}
