//! `uteke skill index` end to end with the real ONNX embedder (#1731).
//!
//! `#[ignore]`d: CI has no ONNX Runtime and the embedding model is a ~200 MB
//! download. Run it where both are available:
//!
//! ```text
//! ORT_LIB_PATH=/path/to/libonnxruntime.so \
//!   cargo test -p uteke-cli --test skill_index_onnx -- --ignored --nocapture
//! ```
//!
//! The model is downloaded into a throwaway `UTEKE_HOME` on the first run; set
//! `UTEKE_ONNX_TEST_HOME` to a directory to keep (and reuse) it between runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

struct Env {
    dir: PathBuf,
    home: PathBuf,
    skills: PathBuf,
    keep_home: bool,
}

impl Env {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("uteke_skill_onnx_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (home, keep_home) = match std::env::var_os("UTEKE_ONNX_TEST_HOME") {
            Some(h) => (PathBuf::from(h), true),
            None => (dir.join("home"), false),
        };
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("uteke.toml"), "[server]\nenabled = false\n").ok();
        let skills = dir.join("skills");
        for (slug, desc, body) in [
            (
                "pdf-extract",
                "Extract text and tables from PDF documents, including scanned pages via OCR.",
                "Pull text, tables and images out of PDF files. Run OCR on scanned pages.",
            ),
            (
                "pdf-reader",
                "Read PDF files and extract their text content and tables, with OCR for scanned pages.",
                "Read PDF files and pull out text, tables and images. Apply OCR to scanned pages.",
            ),
            (
                "cooking",
                "Plan weeknight dinners, scale recipes and suggest ingredient substitutions in the kitchen.",
                "Help decide what to make for dinner, convert measurements and swap ingredients.",
            ),
        ] {
            let d = skills.join(slug);
            std::fs::create_dir_all(d.join("references")).unwrap();
            std::fs::write(
                d.join("SKILL.md"),
                format!("---\nname: {slug}\ndescription: {desc}\n---\n# {slug}\n{body}\n"),
            )
            .unwrap();
            std::fs::write(
                d.join("references/notes.md"),
                format!("Reference for {slug}."),
            )
            .unwrap();
        }
        Self {
            dir,
            home,
            skills,
            keep_home,
        }
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_uteke"));
        cmd.arg("--store")
            .arg(self.dir.join("store"))
            .arg("--json")
            .args(args)
            .env("UTEKE_HOME", &self.home)
            .env("UTEKE_NO_UPDATE_CHECK", "1")
            .env_remove("UTEKE_AUTH_TOKEN");
        let out = cmd.output().expect("run uteke");
        assert!(
            out.status.success(),
            "uteke {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8(out.stdout).unwrap();
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}): {text}"))
    }

    fn checksums(&self) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(p: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut es: Vec<_> = std::fs::read_dir(p).unwrap().flatten().collect();
            es.sort_by_key(|e| e.path());
            for e in es {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push((path.clone(), std::fs::read(&path).unwrap()));
                }
            }
        }
        let mut v = Vec::new();
        walk(&self.skills, &mut v);
        v
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if self.keep_home {
            let _ = std::fs::remove_dir_all(self.dir.join("store"));
            let _ = std::fs::remove_dir_all(&self.skills);
        } else {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

fn top_slug(v: &Value) -> &str {
    v[0]["slug"].as_str().expect("a hit")
}

#[test]
#[ignore = "needs ONNX Runtime (ORT_LIB_PATH) and the embedding model (~200 MB download)"]
fn skill_index_with_the_real_onnx_embedder() {
    let env = Env::new();
    let skills = env.skills.to_str().unwrap().to_string();
    let before = env.checksums();

    // First index: three drafts, the two PDF skills flagged as near-duplicates.
    let r = env.json(&["skill", "index", &skills]);
    assert_eq!(r["created"], 3, "{r}");
    let dups = r["near_duplicates"].as_array().unwrap();
    assert!(
        dups.iter().any(|d| {
            let (a, b) = (d["slug"].as_str().unwrap(), d["other"].as_str().unwrap());
            [a, b].contains(&"pdf-extract") && [a, b].contains(&"pdf-reader")
        }),
        "{r}"
    );
    assert!(
        dups.iter()
            .all(|d| d["slug"] != "cooking" && d["other"] != "cooking"),
        "{r}"
    );

    // Search by meaning: no query word appears in the target skill.
    let hits = env.json(&[
        "skill",
        "search",
        "what should I make tonight with leftover chicken",
    ]);
    assert_eq!(top_slug(&hits), "cooking", "{hits}");
    let hits = env.json(&["skill", "search", "digitise scanned paperwork"]);
    assert!(top_slug(&hits).starts_with("pdf-"), "{hits}");

    // `similar`: the near-duplicate with a plausible score, the unrelated one not.
    let s = env.json(&["skill", "similar", "pdf-extract"]);
    let m = s["hits"].as_array().unwrap();
    assert_eq!(m.len(), 1, "{s}");
    assert_eq!(m[0]["slug"], "pdf-reader");
    let score = m[0]["score"].as_f64().unwrap();
    assert!((0.80..=1.0).contains(&score), "{s}");
    assert_eq!(env.json(&["skill", "similar", "cooking"])["matches"], 0);

    // Approve, then an unchanged re-index leaves the approval alone.
    let list = env.json(&["skill", "list"]);
    let hash = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["slug"] == "cooking")
        .unwrap()["hash"]
        .as_str()
        .unwrap()
        .to_string();
    let a = env.json(&["skill", "approve", &format!("cooking@{hash}")]);
    assert_eq!(a["status"], "approved", "{a}");
    let r = env.json(&["skill", "index", &skills]);
    assert_eq!(
        (r["created"].as_u64(), r["unchanged"].as_u64()),
        (Some(0), Some(3)),
        "{r}"
    );

    // An edited file makes a new draft version and the approval stale.
    let notes = env.skills.join("cooking/references/notes.md");
    std::fs::write(&notes, "Reference for cooking. Edited.").unwrap();
    let r = env.json(&["skill", "index", &skills]);
    assert_eq!(r["updated"], 1, "{r}");
    let c = r["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["slug"] == "cooking")
        .unwrap()
        .clone();
    assert_eq!(c["status"], "draft", "{c}");

    // Indexing never modified a source file (apart from our own edit).
    std::fs::write(&notes, "Reference for cooking.").unwrap();
    assert_eq!(env.checksums(), before);
}
