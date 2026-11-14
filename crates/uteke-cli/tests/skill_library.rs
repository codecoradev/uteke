//! `uteke skill` on a local store (#1731).
//!
//! The library is filled through the core with the keyword test embedder (no
//! ONNX runtime needed, CI has none), then read by the real binary: list,
//! show, search, similar, lint, approve and record embed nothing.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

const SKILLS: [(&str, &str, &str); 3] = [
    (
        "deploy",
        "Ship a release",
        "quartz meridian orchid ledger harbour",
    ),
    (
        "deploy-copy",
        "Ship a release",
        "quartz meridian orchid ledger harbour",
    ),
    (
        "cooking",
        "Cook dinner",
        "zebra violin tractor glacier lantern",
    ),
];

struct Store {
    dir: PathBuf,
    skills: PathBuf,
}

impl Store {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("uteke_skill_library_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        // Server routing off, so every command runs against the local store.
        std::fs::write(dir.join("home/uteke.toml"), "[server]\nenabled = false\n").unwrap();
        let skills = dir.join("skills");
        Self { dir, skills }
    }

    /// The store with the skills of `skills_dir()` indexed through the core.
    fn seeded(name: &str) -> Self {
        let store = Self::new(name);
        std::fs::create_dir_all(&store.skills).unwrap();
        for (slug, desc, body) in SKILLS {
            let d = store.skills.join(slug);
            std::fs::create_dir_all(d.join("references")).unwrap();
            std::fs::write(
                d.join("SKILL.md"),
                format!("---\nname: {slug}\ndescription: {desc}\n---\n{body}\n"),
            )
            .unwrap();
            std::fs::write(d.join("references/notes.md"), format!("notes for {slug}")).unwrap();
        }
        {
            let u = uteke_core::test_support::open_keyword_at(store.dir.join("store"));
            u.skill_index(&store.skills, Some("me"), None).unwrap();
        }
        store
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uteke"))
            .arg("--store")
            .arg(self.dir.join("store"))
            .args(args)
            .env("UTEKE_HOME", self.dir.join("home"))
            .env("UTEKE_NO_UPDATE_CHECK", "1")
            // No embedder: only the commands that embed fail.
            .env("ORT_LIB_PATH", "/nonexistent/uteke-test/libonnxruntime.so")
            // The store holds 64-dimension test vectors, the binary's model has 768:
            // only the open-time dimension check objects, reading is unaffected.
            .env("UTEKE_ALLOW_DIM_MISMATCH", "1")
            .env_remove("UTEKE_AUTH_TOKEN")
            .output()
            .expect("run uteke")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "uteke {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let text = self.ok(args);
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("uteke {args:?} printed no JSON ({e}): {text}"))
    }

    fn fails(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(!out.status.success(), "uteke {args:?} should fail");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn full_hash(store: &Store, slug: &str) -> String {
    store.json(&["--json", "skill", "show", slug])["current"]["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn list_and_show_report_the_indexed_library() {
    let store = Store::seeded("show");
    let v = store.json(&["--json", "skill", "list"]);
    let rows = v.as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["slug"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["cooking", "deploy", "deploy-copy"]
    );
    assert_eq!(rows[0]["status"], "draft");
    assert_eq!(rows[0]["owner"], "me");
    assert_eq!(rows[0]["description"], "Cook dinner");

    let d = store.json(&["--json", "skill", "show", "deploy"]);
    assert_eq!(d["slug"], "deploy");
    assert_eq!(d["current"]["status"], "draft");
    assert_eq!(d["current"]["file_count"], 2);
    let hash = d["current"]["hash"].as_str().unwrap();
    assert_eq!(hash.len(), 71, "sha256: + 64 hex digits");
    assert!(hash.starts_with("sha256:"), "{hash}");
    assert_eq!(d["approved_hash"], Value::Null);
    assert_eq!(d["ledger"][0]["action"], "created");
    let text = store.ok(&["skill", "show", "deploy"]);
    assert!(text.starts_with("deploy (draft)\n  hash:     "), "{text}");
    assert!(text.contains("references/notes.md  "), "{text}");

    let err = store.fails(&["skill", "show", "nope"]);
    assert!(err.contains("skill not found: nope"), "{err}");
    let err = store.fails(&["skill", "list", "--status", "stale"]);
    assert!(err.contains("status must be"), "{err}");
}

#[test]
fn search_finds_skills_by_keyword_and_degrades_without_the_model() {
    let store = Store::seeded("search");
    for mode in ["fts", "hybrid"] {
        let v = store.json(&[
            "--json",
            "skill",
            "search",
            "quartz meridian",
            "--mode",
            mode,
        ]);
        let slugs: Vec<_> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["slug"].as_str().unwrap())
            .collect();
        assert_eq!(slugs, ["deploy", "deploy-copy"], "{mode}");
        assert_eq!(v[0]["file"], "SKILL.md");
        assert_eq!(v[0]["status"], "draft");
    }
    let v = store.json(&[
        "--json", "skill", "search", "quartz", "--limit", "1", "--mode", "fts",
    ]);
    assert_eq!(v.as_array().unwrap().len(), 1);
    // Semantic needs the model, which the test environment lacks.
    store.fails(&["skill", "search", "quartz", "--mode", "semantic"]);
    assert!(
        store
            .fails(&["skill", "search", "  "])
            .contains("must not be empty")
    );
    assert!(
        store
            .fails(&["skill", "search", "x", "--mode", "fuzzy"])
            .contains("mode must be")
    );
    let text = store.ok(&["skill", "search", "zebra", "--mode", "fts"]);
    assert!(text.starts_with("1. cooking [draft]"), "{text}");
    assert_eq!(
        store.ok(&["skill", "search", "nomatchword", "--mode", "fts"]),
        "No skills found.\n"
    );
}

#[test]
fn skill_files_do_not_leak_into_doc_commands() {
    let store = Store::seeded("hidden");
    let v = store.json(&["--json", "doc", "list"]);
    assert_eq!(v.as_array().map(Vec::len), Some(0), "{v}");
    let v = store.json(&["--json", "doc", "search", "quartz", "--mode", "fts"]);
    assert_eq!(v.as_array().map(Vec::len), Some(0), "{v}");
}

#[test]
fn similar_reads_the_skill_vectors() {
    let store = Store::seeded("similar");
    let v = store.json(&["--json", "skill", "similar", "deploy"]);
    assert_eq!(v["source"], "deploy");
    assert_eq!(v["matches"], 1);
    assert_eq!(v["hits"][0]["slug"], "deploy-copy");
    assert_eq!(v["hits"][0]["relation"], "possible_duplicate");
    // Same text, but `name: deploy` against `name: deploy-copy`.
    let score = v["hits"][0]["score"].as_f64().unwrap();
    assert!((score - 0.968_245_86).abs() < 1e-6, "{score}");
    let v = store.json(&[
        "--json",
        "skill",
        "similar",
        "deploy",
        "--threshold",
        "0",
        "--limit",
        "1",
    ]);
    assert_eq!(v["matches"], 2);
    assert_eq!(v["hits"].as_array().unwrap().len(), 1);
    let text = store.ok(&["skill", "similar", "deploy"]);
    assert!(
        text.contains("possible_duplicate 0.968  deploy-copy"),
        "{text}"
    );
    assert!(
        store
            .fails(&["skill", "similar"])
            .contains("Provide a skill slug")
    );
    assert!(
        store
            .fails(&["skill", "similar", "deploy", "--text", "x"])
            .contains("cannot be used with")
    );
    assert!(
        store
            .fails(&["skill", "similar", "nope"])
            .contains("skill not found")
    );
    assert!(
        store
            .fails(&["skill", "similar", "deploy", "--threshold", "2"])
            .contains("threshold")
    );
    // A draft text needs the model.
    store.fails(&["skill", "similar", "--text", "quartz meridian"]);
}

#[test]
fn approve_records_the_hash_and_checks_it() {
    let store = Store::seeded("approve");
    let h = full_hash(&store, "cooking");
    let hex = h.strip_prefix("sha256:").unwrap().to_string();
    let out = store.ok(&[
        "skill",
        "approve",
        &format!("cooking@{h}"),
        "--actor",
        "alice",
    ]);
    assert_eq!(out, format!("Approved cooking@{h}\n"));
    let d = store.json(&["--json", "skill", "show", "cooking"]);
    assert_eq!(d["current"]["status"], "approved");
    assert_eq!(d["approved_hash"], h.as_str());
    assert_eq!(d["current"]["approved_by"], "alice");
    assert_eq!(d["ledger"][0]["action"], "approved");
    // Again: a no-op.
    let out = store.ok(&["skill", "approve", &format!("cooking@{}", &hex[..12])]);
    assert_eq!(out, format!("cooking@{h} was already approved\n"));
    // The same hash as a bare hex digest, and as `sha256:` plus a prefix.
    let out = store.ok(&["skill", "approve", &format!("cooking@{hex}")]);
    assert_eq!(out, format!("cooking@{h} was already approved\n"));
    let out = store.ok(&[
        "skill",
        "approve",
        &format!("cooking@sha256:{}", &hex[..16]),
    ]);
    assert_eq!(out, format!("cooking@{h} was already approved\n"));
    let v = store.json(&["--json", "skill", "approve", &format!("cooking@{h}")]);
    assert_eq!(v["changed"], false);
    assert_eq!(
        store
            .json(&["--json", "skill", "list", "--status", "approved"])
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let wrong = "0".repeat(64);
    assert!(
        store
            .fails(&["skill", "approve", &format!("deploy@{wrong}")])
            .contains("no version with hash")
    );
    assert!(
        store
            .fails(&["skill", "approve", &format!("deploy@{}", &hex[..11])])
            .contains("hash must be")
    );
    assert!(
        store
            .fails(&["skill", "approve", "deploy"])
            .contains("expected <slug>@<hash>")
    );
    assert!(
        store
            .fails(&["skill", "approve", &format!("nope@{h}")])
            .contains("skill not found")
    );
}

#[test]
fn record_counts_usage() {
    let store = Store::seeded("record");
    store.ok(&["skill", "record", "deploy", "recalled"]);
    let v = store.json(&["--json", "skill", "record", "deploy", "loaded"]);
    assert_eq!(
        (v["recalled"].as_i64(), v["loaded"].as_i64()),
        (Some(1), Some(1))
    );
    assert!(
        store
            .fails(&["skill", "record", "deploy", "exploded"])
            .contains("event must be")
    );
    assert!(
        store
            .fails(&["skill", "record", "nope", "loaded"])
            .contains("skill not found")
    );
}

#[test]
fn lint_a_directory_needs_no_model_and_fails_on_invalid_skills() {
    let store = Store::seeded("lint");
    let v = store.json(&["--json", "skill", "lint", store.skills.to_str().unwrap()]);
    assert_eq!(v["skills"].as_array().unwrap().len(), 3);
    assert!(
        v["skills"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["valid"] == true)
    );
    // The indexed library lints clean too.
    store.ok(&["skill", "lint"]);

    let bad = store.skills.join("broken");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("SKILL.md"), "no frontmatter here\n").unwrap();
    let out = store.run(&["skill", "lint", store.skills.to_str().unwrap()]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("INVALID broken"), "{stdout}");
    assert!(
        stdout.contains("3 skill(s)") || stdout.contains("4 skill(s), 1 invalid"),
        "{stdout}"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 skill(s) invalid or unreadable"));
    assert!(
        store
            .fails(&["skill", "lint", store.dir.join("missing").to_str().unwrap()])
            .contains("Failed to lint")
    );
}
