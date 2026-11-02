//! Timeline events written by update, bulk deprecation and contradiction
//! (#1595), and the history a prune leaves behind.

use crate::Uteke;
use crate::provenance::content_hash;
use crate::test_support::open_keyword;
use crate::timeline::TimelineEvent;

fn events_of(u: &Uteke, id: &str, kind: &str) -> Vec<TimelineEvent> {
    u.timeline(id, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == kind)
        .collect()
}

fn data(e: &TimelineEvent) -> serde_json::Value {
    serde_json::from_str(e.event_data.as_deref().expect("event data")).unwrap()
}

#[test]
fn a_content_update_refreshes_the_hash_and_links_it_in_an_updated_event() {
    let u = open_keyword();
    let id = u
        .remember("the deploy window is 09:00", &[], None, None)
        .unwrap();
    let first = content_hash("the deploy window is 09:00");

    u.update_memory(
        &id,
        Some("the deploy window is 10:00"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let second = content_hash("the deploy window is 10:00");
    let report = u.provenance(&id).unwrap().unwrap();
    assert_eq!(report.source_hash.as_deref(), Some(second.as_str()));
    assert_eq!(
        report.content_hash_now, second,
        "no mismatch after an API update"
    );

    // The same shape as uteke-cloud #354.
    let updated = events_of(&u, &id, "updated");
    assert_eq!(updated.len(), 1, "{updated:?}");
    assert_eq!(
        data(&updated[0]),
        serde_json::json!({"fields": ["content"], "previous_hash": first, "source_hash": second})
    );
    assert_eq!(
        updated[0].evidence,
        Some(serde_json::json!([{"memory": id, "relation": "subject"}]))
    );
    assert_eq!(updated[0].actor.as_deref(), Some("system:edit"));

    // A second update chains from the first.
    u.update_memory(
        &id,
        Some("the deploy window is 11:00"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let newest = &events_of(&u, &id, "updated")[0];
    assert_eq!(data(newest)["previous_hash"], serde_json::json!(second));
    assert_eq!(
        data(newest)["source_hash"],
        serde_json::json!(content_hash("the deploy window is 11:00"))
    );
    // The write-time hash stays in history: the oldest link starts from it.
    assert_eq!(events_of(&u, &id, "updated").len(), 2);
}

#[test]
fn an_update_without_a_content_change_leaves_hash_and_timeline_alone() {
    let u = open_keyword();
    let id = u
        .remember("tags only change here", &[], None, None)
        .unwrap();
    let before = u.provenance(&id).unwrap().unwrap().source_hash;

    u.update_memory(
        &id,
        None,
        Some(&["x".to_string()]),
        None,
        Some(0.8),
        None,
        None,
    )
    .unwrap();
    // The same text is not a content change either.
    u.update_memory(
        &id,
        Some("tags only change here"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(u.provenance(&id).unwrap().unwrap().source_hash, before);
    assert!(events_of(&u, &id, "updated").is_empty());
}

#[test]
fn a_direct_sql_edit_still_shows_as_a_mismatch() {
    let u = open_keyword();
    let id = u.remember("audited content", &[], None, None).unwrap();
    u.update_memory(
        &id,
        Some("audited content v2"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    u.store
        .conn
        .execute(
            "UPDATE memories SET content = 'tampered' WHERE id = ?1",
            rusqlite::params![id],
        )
        .unwrap();
    let report = u.provenance(&id).unwrap().unwrap();
    assert_eq!(
        report.source_hash.as_deref(),
        Some(content_hash("audited content v2").as_str())
    );
    assert_ne!(
        report.source_hash.as_deref(),
        Some(report.content_hash_now.as_str())
    );
}

#[test]
fn a_bulk_deprecation_records_one_event_per_memory_it_deprecates() {
    let u = open_keyword();
    let a = u
        .remember("aging candidate alpha", &[], None, None)
        .unwrap();
    let b = u.remember("aging candidate beta", &[], None, None).unwrap();
    u.store.deprecate_with_reason(&b, "earlier").unwrap();

    let n = u
        .store
        .deprecate_by_ids(&[a.clone(), b.clone()], "auto-aging: test")
        .unwrap();
    assert_eq!(n, 1, "the already deprecated one is skipped");

    let a_events = events_of(&u, &a, "deprecated");
    assert_eq!(a_events.len(), 1);
    assert_eq!(
        data(&a_events[0]),
        serde_json::json!({"reason": "auto-aging: test"})
    );
    // No duplicate for the memory that was already deprecated.
    let b_events = events_of(&u, &b, "deprecated");
    assert_eq!(b_events.len(), 1);
    assert_eq!(data(&b_events[0]), serde_json::json!({"reason": "earlier"}));
}

#[test]
fn aging_cleanup_leaves_a_deprecated_event() {
    let u = open_keyword();
    let id = u
        .remember("an old and unused note", &[], None, None)
        .unwrap();
    let old = (chrono::Utc::now() - chrono::Duration::days(400)).to_rfc3339();
    u.store
        .conn
        .execute(
            "UPDATE memories SET created_at = ?1, last_accessed = NULL WHERE id = ?2",
            rusqlite::params![old, id],
        )
        .unwrap();

    u.aging_cleanup(90, 0, None).unwrap();

    assert!(u.get_by_id(&id).unwrap().unwrap().deprecated);
    let events = events_of(&u, &id, "deprecated");
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(
        data(&events[0])["reason"]
            .as_str()
            .unwrap()
            .starts_with("auto-aging"),
        "{events:?}"
    );
}

#[test]
fn a_contradiction_on_write_leaves_a_deprecated_event_on_the_old_memory() {
    let u = open_keyword();
    let (old, _) = u
        .remember_with_contradiction(
            "deploy window daily morning",
            &[],
            None,
            Some("ops"),
            None,
            true,
            0.5,
        )
        .unwrap();
    let (new, contradiction) = u
        .remember_with_contradiction(
            "deploy window daily evening",
            &[],
            None,
            Some("ops"),
            None,
            true,
            0.5,
        )
        .unwrap();
    assert!(contradiction.contradicted, "{contradiction:?}");
    let old = old.id;

    assert!(u.get_by_id(&old).unwrap().unwrap().deprecated);
    let events = events_of(&u, &old, "deprecated");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(
        data(&events[0])["reason"],
        serde_json::json!(format!("contradiction: replaced by {}", new.id))
    );
    assert_eq!(events_of(&u, &old, "consolidated").len(), 1);
}

#[test]
fn a_prune_keeps_the_history_of_the_memories_it_deletes() {
    let u = open_keyword();
    let id = u.remember("soon pruned", &[], None, None).unwrap();
    u.update_memory(
        &id,
        Some("soon pruned, edited"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    u.store.deprecate_with_reason(&id, "obsolete").unwrap();
    let old = (chrono::Utc::now() - chrono::Duration::days(400)).to_rfc3339();
    u.store
        .conn
        .execute(
            "UPDATE memories SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![old, id],
        )
        .unwrap();
    let before = u.timeline(&id, 0).unwrap().len();

    let result = u.prune(30, None, false).unwrap();
    assert_eq!(result.pruned, 1);
    assert!(u.get_by_id(&id).unwrap().is_none());
    // The rows stay as audit tombstones (#1280, schema v20).
    let kinds: Vec<String> = u
        .timeline(&id, 0)
        .unwrap()
        .into_iter()
        .map(|e| e.event_type)
        .collect();
    assert_eq!(kinds.len(), before, "{kinds:?}");
    for kind in ["created", "updated", "deprecated"] {
        assert!(kinds.iter().any(|k| k == kind), "{kinds:?}");
    }
}
