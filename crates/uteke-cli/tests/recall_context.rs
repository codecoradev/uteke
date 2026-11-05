//! `uteke recall --context` on the default (unified) path (#1642): the context
//! layout is printed without a memory-only flag, for hits and for no hits.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "uteke_recall_context_{name}_{}",
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

    fn stdout(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "uteke {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn no_hit_prints_the_context_empty_line() {
    let store = Store::new("empty");
    store.stdout(&["remember", "plain words"]);
    let out = store.stdout(&["recall", "zebracorn", "--context"]);
    assert_eq!(
        out.trim_end(),
        "[No relevant memories found for: zebracorn]"
    );
}

#[test]
fn a_hit_prints_the_context_layout() {
    let store = Store::new("hit");
    store.stdout(&[
        "remember",
        "the pelican deploy uses blue green",
        "--tags",
        "ops",
    ]);
    let out = store.stdout(&["recall", "pelican", "--context", "--strategy", "fts5"]);
    let mut lines = out.lines();
    let header = lines.next().unwrap_or_default();
    assert!(
        header.starts_with("[Relevant Memories (1 results, "),
        "{out}"
    );
    let first = lines.next().unwrap_or_default();
    assert!(first.starts_with("1. ["), "{out}");
    assert!(
        first.ends_with("] the pelican deploy uses blue green [ops]"),
        "{out}"
    );
}
