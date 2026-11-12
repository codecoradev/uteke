//! `uteke doc similar` and `uteke doc duplicates` on a local store (#1730).
//!
//! The store is filled through the core with the keyword test embedder (no
//! ONNX runtime needed, CI has none), then read by the real binary: both
//! commands read the document vectors from SQLite and embed nothing, except
//! `--text`, which needs the model and fails cleanly without it.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

struct Store {
    dir: PathBuf,
}

const ALPHA: &str = "# Alpha\n\nquartz meridian orchid ledger harbour";
const OTHER: &str = "# Other\n\nzebra violin tractor glacier lantern";

impl Store {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("uteke_doc_similar_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        // Server routing off, so every command runs against the local store.
        std::fs::write(dir.join("home/uteke.toml"), "[server]\nenabled = false\n").unwrap();
        Self { dir }
    }

    /// The store with: `alpha` and `alpha-copy` (same text), `other` (unrelated),
    /// and `alpha-child`, a child of `alpha` with the same text again.
    fn seeded(name: &str) -> Self {
        let store = Self::new(name);
        {
            let u = uteke_core::test_support::open_keyword_at(store.dir.join("store"));
            u.doc_upsert("alpha", "Alpha", ALPHA, &[], None).unwrap();
            u.doc_upsert("alpha-copy", "Alpha copy", ALPHA, &[], None)
                .unwrap();
            u.doc_upsert("other", "Other", OTHER, &[], None).unwrap();
            u.doc_upsert_with_parent(
                "alpha-child",
                "Alpha child",
                ALPHA,
                &[],
                None,
                Some("alpha"),
            )
            .unwrap();
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

fn slugs(hits: &Value) -> Vec<&str> {
    hits.as_array()
        .unwrap()
        .iter()
        .map(|h| h["document"]["slug"].as_str().unwrap())
        .collect()
}

#[test]
fn similar_json_lists_the_copies_with_exact_scores_and_relations() {
    let store = Store::seeded("json");
    let v = store.json(&["--json", "doc", "similar", "alpha"]);
    assert_eq!(v["source"]["slug"], "alpha");
    assert_eq!(v["threshold"], 0.8);
    assert_eq!(v["similar_threshold"], 0.8);
    assert_eq!(v["duplicate_threshold"], 0.92);
    assert_eq!(v["source_embedded"], true);
    assert_eq!(v["documents_without_vector"], 0);
    assert_eq!(v["matches"], 2);
    assert_eq!(slugs(&v["hits"]), ["alpha-child", "alpha-copy"]);
    for hit in v["hits"].as_array().unwrap() {
        assert_eq!(hit["score"], 1.0);
        assert_eq!(hit["relation"], "possible_duplicate");
    }
}

#[test]
fn similar_honours_limit_threshold_and_namespace() {
    let store = Store::seeded("opts");
    let v = store.json(&["--json", "doc", "similar", "alpha", "--limit", "1"]);
    assert_eq!(v["matches"], 2);
    assert_eq!(slugs(&v["hits"]), ["alpha-child"]);
    // At the top of the scale only exact copies pass; `other` shares no word.
    let v = store.json(&["--json", "doc", "similar", "alpha", "--threshold", "1"]);
    assert_eq!(v["threshold"], 1.0);
    assert_eq!(v["matches"], 2);
    // Everything, `other` included, with a zero floor.
    let v = store.json(&["--json", "doc", "similar", "alpha", "--threshold", "0"]);
    assert_eq!(v["matches"], 3);
    assert_eq!(slugs(&v["hits"]).last(), Some(&"other"));
    // Documents have no namespace: a namespace scope finds none.
    let v = store.json(&["--json", "doc", "similar", "alpha", "--namespace", "team-x"]);
    assert_eq!(v["matches"], 0);
    assert_eq!(v["hits"], serde_json::json!([]));
}

#[test]
fn similar_text_output_names_relation_score_and_slug() {
    let store = Store::seeded("text");
    let out = store.ok(&["doc", "similar", "other", "--threshold", "0"]);
    assert!(
        out.starts_with("Documents similar to 'other' (threshold 0.00;"),
        "{out}"
    );
    let none = store.ok(&["doc", "similar", "other"]);
    assert!(none.contains("  none\n"), "{none}");
    assert!(none.ends_with("0 match(es)\n"), "{none}");
    let dup = store.ok(&["doc", "similar", "alpha"]);
    assert!(
        dup.contains("possible_duplicate 1.000  alpha-child"),
        "{dup}"
    );
    assert!(
        dup.contains("possible_duplicate 1.000  alpha-copy"),
        "{dup}"
    );
    assert!(dup.ends_with("2 match(es)\n"), "{dup}");
}

#[test]
fn similar_errors_are_clear() {
    let store = Store::seeded("errors");
    let e = store.fails(&["doc", "similar", "ghost"]);
    assert!(e.contains("document not found: ghost"), "{e}");
    let e = store.fails(&["doc", "similar", "alpha", "--threshold", "1.5"]);
    assert!(
        e.contains("threshold must be a number between 0 and 1"),
        "{e}"
    );
    let e = store.fails(&["doc", "similar"]);
    assert!(e.contains("Provide a document slug, or --text"), "{e}");
    // clap refuses a slug together with --text.
    let out = store.run(&["doc", "similar", "alpha", "--text", "x"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be used with"));
}

#[test]
fn similar_text_needs_the_model_and_stores_nothing() {
    let store = Store::seeded("draft");
    let e = store.fails(&["doc", "similar", "--text", "quartz meridian"]);
    assert!(e.contains("Failed to find similar documents"), "{e}");
    let list = store.json(&["--json", "doc", "list"]);
    assert_eq!(
        list.as_array().unwrap().len(),
        4,
        "nothing was stored: {list}"
    );
}

#[test]
fn duplicates_reports_pairs_and_groups() {
    let store = Store::seeded("dups");
    let v = store.json(&["--json", "doc", "duplicates"]);
    assert_eq!(v["threshold"], 0.92);
    assert_eq!(v["scanned"], 4);
    assert_eq!(v["documents_without_vector"], 0);
    assert_eq!(v["total_pairs"], 3);
    let pairs: Vec<(String, String, f64)> = v["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["a"]["slug"].as_str().unwrap().to_string(),
                p["b"]["slug"].as_str().unwrap().to_string(),
                p["score"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [
            ("alpha".to_string(), "alpha-child".to_string(), 1.0),
            ("alpha".to_string(), "alpha-copy".to_string(), 1.0),
            ("alpha-child".to_string(), "alpha-copy".to_string(), 1.0),
        ]
    );
    assert_eq!(
        v["groups"],
        serde_json::json!([["alpha", "alpha-child", "alpha-copy"]])
    );

    let text = store.ok(&["doc", "duplicates"]);
    assert!(text.contains("  1.000  alpha  ~  alpha-child"), "{text}");
    assert!(text.contains("3 pair(s)"), "{text}");
    assert!(
        text.contains("Group 1: alpha, alpha-child, alpha-copy"),
        "{text}"
    );
}

#[test]
fn duplicates_subtree_limit_and_threshold() {
    let store = Store::seeded("scope");
    let v = store.json(&["--json", "doc", "duplicates", "--subtree", "alpha"]);
    assert_eq!(v["scanned"], 2, "alpha and its child");
    assert_eq!(v["total_pairs"], 1);
    assert_eq!(v["groups"], serde_json::json!([["alpha", "alpha-child"]]));
    let v = store.json(&["--json", "doc", "duplicates", "--limit", "1"]);
    assert_eq!(v["total_pairs"], 3);
    assert_eq!(v["pairs"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["groups"][0].as_array().unwrap().len(),
        3,
        "groups ignore the limit"
    );
    let v = store.json(&["--json", "doc", "duplicates", "--threshold", "0"]);
    assert_eq!(v["total_pairs"], 6, "every pair of four documents");
    let e = store.fails(&["doc", "duplicates", "--subtree", "ghost"]);
    assert!(e.contains("document not found: ghost"), "{e}");
    let e = store.fails(&["doc", "duplicates", "--threshold", "-0.5"]);
    assert!(e.contains("threshold"), "{e}");
}

#[test]
fn an_empty_library_has_nothing_to_report() {
    let store = Store::new("empty");
    let v = store.json(&["--json", "doc", "duplicates"]);
    assert_eq!(v["scanned"], 0);
    assert_eq!(v["total_pairs"], 0);
    assert_eq!(v["pairs"], serde_json::json!([]));
    assert_eq!(v["groups"], serde_json::json!([]));
    let text = store.ok(&["doc", "duplicates"]);
    assert!(text.contains("0 document(s) scanned"), "{text}");
    assert!(text.ends_with("0 pair(s)\n"), "{text}");
}
