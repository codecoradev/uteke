//! Scope-isolation regression gate over the CLI (#1467, part 3).
//!
//! Seeds a store with two namespaces (`alpha`, `beta`) whose memories share one
//! vocabulary, then reads it through every CLI command that takes a scope. Each
//! memory ends in `of <namespace>`, so output scoped to one namespace must hold
//! its own marker and never the other's, nor the other's project tag.
//!
//! `uteke recall` needs the ONNX runtime, which CI does not have (memories are
//! stored FTS-only here, see `json_output_stays_clean`); the core, HTTP and MCP
//! suites cover vector recall. `docs/isolation-suite.md` lists the coverage.

use std::path::PathBuf;
use std::process::Command;

const Q: &str = "quartz meridian orchid ledger";
const SCOPES: [&str; 2] = ["alpha", "beta"];

struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("uteke_isolation_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        // Server routing off, so every command runs against the local store.
        std::fs::write(dir.join("home/uteke.toml"), "[server]\nenabled = false\n").unwrap();
        Self { dir }
    }

    fn run(&self, args: &[&str]) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_uteke"))
            .arg("--store")
            .arg(self.dir.join("store"))
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
            "uteke {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn seed(&self) {
        for scope in SCOPES {
            let tags = format!("project:{scope},shared");
            for i in 0..3 {
                let text = format!("{Q} note {i} of {scope}");
                self.run(&[
                    "--json",
                    "remember",
                    &text,
                    "--namespace",
                    scope,
                    "--tags",
                    &tags,
                ]);
            }
            let room = format!("room-{scope}");
            let text = format!("{Q} room entry of {scope}");
            self.run(&[
                "--json",
                "remember",
                &text,
                "--namespace",
                scope,
                "--tags",
                &tags,
                "--room",
                &room,
                "--author",
                "tester",
            ]);
            // Unrelated vocabulary: the other memories link to each other, this
            // one stays an orphan.
            let text = format!("solitary zebra fjord of {scope}");
            self.run(&["--json", "remember", &text, "--namespace", scope]);
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// (arguments, the output must hold an `of <scope>` marker).
fn surfaces(scope: &str) -> Vec<(Vec<String>, bool)> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let room = format!("room-{scope}");
    vec![
        (
            s(&["--json", "search", Q, "--namespace", scope, "--limit", "20"]),
            true,
        ),
        (
            s(&["--json", "list", "--namespace", scope, "--limit", "50"]),
            true,
        ),
        (
            s(&[
                "--json",
                "list",
                "--tag",
                "shared",
                "--namespace",
                scope,
                "--limit",
                "50",
            ]),
            true,
        ),
        (s(&["--json", "export", "-", "--namespace", scope]), true),
        (s(&["--json", "room", "recall", &room]), true),
        (
            s(&[
                "--json",
                "orphans",
                "--threshold",
                "2",
                "--namespace",
                scope,
            ]),
            true,
        ),
        (s(&["--json", "tags", "list", "--namespace", scope]), false),
        (s(&["--json", "stats", "--namespace", scope]), false),
        (s(&["--json", "room", "list", "--namespace", scope]), false),
        (
            s(&["--json", "contradictions", "list", "--namespace", scope]),
            false,
        ),
    ]
}

#[test]
fn every_scoped_read_command_returns_only_its_own_namespace() {
    let store = Store::new("read");
    store.seed();
    for scope in SCOPES {
        let foreign = if scope == "alpha" { "beta" } else { "alpha" };
        for (args, expects_hits) in surfaces(scope) {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = store.run(&refs);
            let shown: String = out.chars().take(300).collect();
            assert!(
                !out.contains(&format!("of {foreign}")),
                "uteke {args:?} scoped to {scope} returned {foreign} records: {shown}"
            );
            assert!(
                !out.contains(&format!("project:{foreign}")),
                "uteke {args:?} scoped to {scope} exposed the {foreign} project tag: {shown}"
            );
            if expects_hits {
                assert!(
                    out.contains(&format!("of {scope}")),
                    "uteke {args:?} found no {scope} record, the check would be vacuous: {shown}"
                );
            }
        }
    }
}
