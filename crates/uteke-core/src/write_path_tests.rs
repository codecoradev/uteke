//! The steps shared by every way of writing a memory (#1514): `remember` and
//! `remember_with_contradiction` embed the same text, retry the same way,
//! validate the type, and leave the same side effects.

use crate::test_support::{RecordingEmbedder, Seen, open_keyword, open_with_embedder};
use crate::{Error, Uteke};

/// A store whose embedder fails its first `fail` calls, and what it was asked.
fn store_with(fail: usize) -> (Uteke, Seen) {
    let (embedder, seen) = RecordingEmbedder::failing_first(fail);
    (open_with_embedder(Box::new(embedder)), seen)
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

const IMPORT_LINE: &str = r#"{"content":"alpha beta gamma delta epsilon zeta eta theta iota","tags":["t"],"namespace":"imp"}"#;

#[test]
fn an_import_leaves_the_same_side_effects_as_remember() {
    let u = open_keyword();
    let remembered = u
        .remember_detailed(
            "alpha beta gamma delta epsilon zeta eta theta",
            &[],
            None,
            Some("imp"),
        )
        .unwrap()
        .id;
    let result = u.import(IMPORT_LINE, None).unwrap();
    assert_eq!(result.imported, 1);
    let imported = u
        .list(None, 10, 0, Some("imp"))
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .find(|id| *id != remembered)
        .expect("the imported row");

    for id in [&remembered, &imported] {
        let events = u.timeline(id, 20).unwrap();
        assert!(
            events.iter().any(|e| e.event_type == "created"),
            "{id}: {events:?}"
        );
        let report = u.provenance(id).unwrap().expect("provenance");
        assert!(report.source_hash.is_some(), "{id}: source hash recorded");
        assert!(
            !u.get_by_id(id).unwrap().unwrap().embedding.is_empty(),
            "{id}: vector stored"
        );
    }
    // The two rows are close but not duplicates: the cosine link connects them.
    let edges = u.edges_for(&imported).unwrap();
    assert!(
        edges.outgoing.iter().any(|e| e.target_id == remembered)
            || edges.incoming.iter().any(|e| e.source_id == remembered),
        "{edges:?}"
    );
}

#[test]
fn a_recall_cached_before_an_import_is_not_stale_after_it() {
    let u = open_keyword();
    u.remember("alpha first note", &[], None, Some("imp"))
        .unwrap();
    let recall = || {
        u.recall_hybrid(
            "alpha",
            10,
            None,
            Some("imp"),
            crate::RecallStrategy::Hybrid,
            0.0,
        )
        .unwrap()
    };
    assert_eq!(recall().len(), 1);

    u.import(IMPORT_LINE, None).unwrap();
    assert_eq!(
        recall().len(),
        2,
        "the import invalidated the cached recall"
    );
}

#[test]
fn an_update_keeps_the_id_and_creation_time_and_is_not_a_remember() {
    let (u, seen) = store_with(0);
    let id = u
        .remember_detailed("header: status open", &[], None, Some("up"))
        .unwrap()
        .id;
    let other = u
        .remember_detailed("header: status closed", &[], None, Some("up"))
        .unwrap()
        .id;
    let before = u.get_by_id(&id).unwrap().unwrap();

    // Update to exactly the text of another memory: it is stored, the dedup
    // check of `remember` is not applied to an update.
    assert!(
        u.update_memory(
            &id,
            Some("header: status closed"),
            None,
            None,
            None,
            None,
            None
        )
        .unwrap()
    );
    let after = u.get_by_id(&id).unwrap().unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.content, "header: status closed");
    assert_eq!(u.get_by_id(&other).unwrap().unwrap().content, after.content);

    // The same text again is not a content change: no embedding call.
    let calls = seen.lock().unwrap().len();
    assert!(
        u.update_memory(
            &id,
            Some("header: status closed"),
            None,
            None,
            Some(0.9),
            None,
            None
        )
        .unwrap()
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        calls,
        "no re-embed for the same text"
    );
    assert!((u.get_by_id(&id).unwrap().unwrap().importance - 0.9).abs() < 1e-9);
}
