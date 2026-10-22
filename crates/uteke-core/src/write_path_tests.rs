//! The steps shared by every way of writing a memory (#1514): `remember` and
//! `remember_with_contradiction` embed the same text, retry the same way,
//! validate the type, and leave the same side effects.

use std::sync::{Arc, Mutex};

use crate::embed::Embedder;
use crate::test_support::open_keyword;
use crate::{Error, Uteke};

/// Records every text it is asked to embed and can fail the first `fail` calls.
struct Recording {
    seen: Arc<Mutex<Vec<String>>>,
    fail: Mutex<usize>,
}

impl Embedder for Recording {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        self.seen.lock().unwrap().push(text.to_string());
        let mut fail = self.fail.lock().unwrap();
        if *fail > 0 {
            *fail -= 1;
            return Err(Error::embed_msg("transient embedding failure"));
        }
        let mut v = vec![0.0_f32; 8];
        v[text.len() % 8] = 1.0;
        Ok(v)
    }
    fn dims(&self) -> usize {
        8
    }
    fn max_seq_len(&self) -> usize {
        128
    }
    fn name(&self) -> &str {
        "recording-test-embedder"
    }
}

fn store_with(fail: usize) -> (Uteke, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (_db, store) = Uteke::open_store(":memory:").expect("open_store");
    let u = Uteke::finish_open_full(
        store,
        Some(Box::new(Recording {
            seen: seen.clone(),
            fail: Mutex::new(fail),
        })),
        "test-recording".to_string(),
        crate::TierConfig::default(),
        crate::RecallConfig::default(),
        crate::EmbeddingSettings::default(),
        crate::graph_rerank::GraphRerankConfig::default(),
        None,
    )
    .expect("finish_open_full");
    (u, seen)
}

const JSON_CONTENT: &str = r#"{"user_name":"Bob","tags":["x","y"],"n":{"deep":1}}"#;

fn contradiction(u: &Uteke, content: &str, ns: &str) -> Result<crate::RememberOutcome, Error> {
    u.remember_with_contradiction(content, &[], None, Some(ns), None, true, 0.65)
        .map(|(o, _)| o)
}

#[test]
fn the_contradiction_path_rejects_an_unknown_type_like_remember() {
    let u = open_keyword();
    let err = u
        .remember_with_contradiction("a memory", &[], None, None, Some("bogus"), true, 0.65)
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)), "{err}");
    let err = u
        .remember_typed_detailed("a memory", &[], None, None, "bogus")
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)), "{err}");
    assert!(u.list(None, 10, 0, None).unwrap().is_empty());
}

#[test]
fn both_paths_embed_the_same_text_for_json_content() {
    let (u, seen) = store_with(0);
    u.remember_detailed(JSON_CONTENT, &[], None, Some("a"))
        .unwrap();
    contradiction(&u, JSON_CONTENT, "b").unwrap();
    let seen = seen.lock().unwrap().clone();
    let flat = crate::memory::crud::flatten_json_for_embedding(JSON_CONTENT);
    assert_ne!(flat, JSON_CONTENT, "the flattening must change this input");
    assert_eq!(seen, vec![flat.clone(), flat], "{seen:?}");
}

#[test]
fn the_contradiction_path_retries_a_transient_embedding_failure() {
    let (u, seen) = store_with(1);
    let o = contradiction(&u, "retry me", "b").unwrap();
    assert!(o.embedding_written, "{:?}", o.warning);
    assert_eq!(seen.lock().unwrap().len(), 2, "one failure, one success");
}

#[test]
fn a_failing_embedder_stores_fts_only_with_a_warning_instead_of_failing_the_write() {
    // Three failures exhaust the retries.
    let (u, _) = store_with(3);
    let o = contradiction(&u, "never embedded", "b").unwrap();
    assert!(!o.embedding_written);
    assert!(
        o.warning
            .as_deref()
            .unwrap()
            .contains("embedding generation failed")
    );
    // The row is stored and keyword-searchable.
    let rows = u.list(None, 10, 0, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].content, "never embedded");
}

#[test]
fn both_paths_leave_the_same_side_effects() {
    let (u, _) = store_with(0);
    let a = u
        .remember_detailed("alpha note", &[], None, Some("a"))
        .unwrap()
        .id;
    let b = contradiction(&u, "beta note", "b").unwrap().id;
    for id in [&a, &b] {
        let events = u.timeline(id, 20).unwrap();
        assert!(
            events.iter().any(|e| e.event_type == "created"),
            "{id}: {events:?}"
        );
        let m = u.get_by_id(id).unwrap().unwrap();
        assert!(!m.embedding.is_empty(), "{id}: vector stored");
    }
}

#[test]
fn without_an_embedder_a_requested_contradiction_check_says_it_was_skipped() {
    let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
    let (o, c) = u
        .remember_with_contradiction("no vectors here", &[], None, None, None, true, 0.65)
        .unwrap();
    assert!(!o.embedding_written);
    assert!(!c.contradicted);
    assert!(
        o.warning
            .as_deref()
            .unwrap()
            .contains("contradiction check skipped"),
        "{:?}",
        o.warning
    );
    // Not asked for: no warning (same as plain remember without an embedder).
    let (o, _) = u
        .remember_with_contradiction("still no vectors", &[], None, None, None, false, 0.65)
        .unwrap();
    assert_eq!(o.warning, None);
}
