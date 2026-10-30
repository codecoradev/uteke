//! High-level consolidation API wiring the executor to a real `Uteke` store.
//!
//! Implements `ConsolidationStore` for `Uteke` (SQLite + lazy embedder) and
//! exposes a single entry point [`consolidate_room`] used by the CLI and the
//! HTTP server. The executor itself stays store-agnostic and testable with
//! a mock store (see `consolidation_exec`).
//!
//! All operations are opt-in: nothing in the dream pipeline calls this
//! automatically. Rooms are consolidated only when explicitly requested
//! via CLI (`uteke consolidate`) or HTTP (`POST /room/{id}/consolidate`).

use crate::Error;
use crate::Uteke;
use crate::consolidation_exec::{self, ConsolidationExecution, ConsolidationStore};
use crate::consolidation_plan::{self, ConsolidationPlan};
use crate::extraction::ExtractionConfig;
use crate::memory::types::Memory;

/// System author tag for consolidated records.
const CONSOLIDATOR_AUTHOR: &str = "uteke-consolidator";

impl ConsolidationStore for Uteke {
    fn room_memories(&self, room_id: &str) -> Result<Vec<Memory>, Error> {
        // author=None → cross-namespace; limit=0 → all; active only (#784).
        self.store().recall_room(room_id, None, 0)
    }

    fn insert_memory(&self, memory: &mut Memory) -> Result<(), Error> {
        // Privacy filter (#1464): the consolidated text comes from an LLM and
        // is stored like any other write. The vector was computed from the
        // filtered text too (see `embed_content`), so nothing unmasked reached
        // the embedder.
        let tag_refs: Vec<&str> = memory.tags.iter().map(String::as_str).collect();
        let prepared = self.privacy_config().prepare_with_tags(
            &memory.content,
            &tag_refs,
            Some(memory.metadata.clone()),
        )?;
        memory.content = prepared.content;
        memory.tags = prepared.tags;
        if let Some(m) = prepared.metadata {
            memory.metadata = m;
        }
        let redactions = prepared.redactions;

        // `room_id`, with the old `room` key as a fallback for a record built
        // by a caller that still uses it (#1594).
        let room_id = ["room_id", "room"]
            .iter()
            .find_map(|key| memory.metadata.get(*key).and_then(|v| v.as_str()))
            .map(str::to_string);

        // The same tail as `remember` (#1514): the row, the source hash, the
        // `Created` event, edge wiring, recall-cache invalidation, the vector and
        // the cosine links. A vector that cannot be written leaves the row
        // FTS5-only with a warning, as for `remember` (`uteke repair` backfills it).
        let outcome = self.commit_memory(memory, None)?;
        if let Some(w) = outcome.warning {
            tracing::warn!("Consolidated record {} is FTS5-only: {w}", memory.id);
        }
        self.record_redaction(&memory.id, &redactions);

        // Keep the record linked to its room so recall_room returns it;
        // roll back the row + index entry on failure (no room links to a
        // memory the room cannot see).
        if let Some(room_id) = room_id {
            if let Err(e) = self.store().link_memory_to_room(
                &room_id,
                &memory.id,
                CONSOLIDATOR_AUTHOR,
                "consolidated",
            ) {
                self.remove_from_index(&memory.id);
                let _ = self.store().delete(&memory.id);
                return Err(e);
            }
        }
        Ok(())
    }

    fn deprecate_memory(&self, id: &str, reason: &str) -> Result<(), Error> {
        self.store().deprecate_with_reason(id, reason)
    }

    fn embed_content(&self, content: &str) -> Result<Vec<f32>, Error> {
        // The privacy filter runs BEFORE the embedder, as on every other write
        // path: with a remote backend the text leaves the process, and the
        // vector must match the text that will be stored. Strict mode refuses
        // here, so a refused record is never embedded.
        let prepared = self.privacy_config().prepare(content, None)?;
        self.embed_text(&prepared.content)
    }
}

/// Outcome of a dry-run (plan only, no LLM calls, no writes).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConsolidationDryRun {
    pub plan: ConsolidationPlan,
}

/// Plan consolidation for a room without any side effects.
///
/// Costs nothing: no LLM call, no store write. The returned plan carries
/// per-batch payloads and cost estimates the caller can inspect first.
pub fn plan_room(uteke: &Uteke, room_id: &str) -> Result<ConsolidationDryRun, Error> {
    let plan = uteke.room_consolidation_plan(
        room_id,
        consolidation_plan::DEFAULT_THRESHOLD,
        consolidation_plan::DEFAULT_MAX_SIZE,
        consolidation_plan::DEFAULT_MIN_SIZE,
    )?;
    Ok(ConsolidationDryRun { plan })
}

/// Execute consolidation for a room: plan → LLM batches → provenance gate →
/// write consolidated records and deprecate sources.
///
/// `max_llm_calls` caps the number of LLM requests for this run (failed
/// calls count against the budget too). Uses the shared extraction endpoint
/// setup — same model/key/base URL as import extraction ("1 setup, many uses").
pub fn consolidate_room(
    uteke: &Uteke,
    room_id: &str,
    config: &ExtractionConfig,
    max_llm_calls: usize,
) -> Result<ConsolidationExecution, Error> {
    let dry = plan_room(uteke, room_id)?;
    consolidation_exec::execute_plan(uteke, room_id, &dry.plan, config, max_llm_calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_makes_no_changes() {
        let uteke = Uteke::open(":memory:").unwrap();
        let dry = plan_room(&uteke, "r1").expect("plan empty room");
        assert!(dry.plan.batches.is_empty());
        assert_eq!(dry.plan.total_memories, 0);
    }

    #[test]
    fn trait_impl_reads_room() {
        // Trait dispatch through &Uteke — catches signature drift between
        // the trait and the store API at compile time.
        fn takes_store<S: ConsolidationStore>(_s: &S) {}
        let uteke = Uteke::open(":memory:").unwrap();
        takes_store(&uteke);
        let mems = ConsolidationStore::room_memories(&uteke, "nope").unwrap();
        assert!(mems.is_empty());
    }

    fn mem(id: &str, content: &str) -> crate::memory::types::Memory {
        use crate::memory::types::Memory;
        use chrono::Utc;
        Memory {
            id: id.to_string(),
            content: content.to_string(),
            embedding: vec![0.1; 768],
            tags: vec!["test".into()],
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            namespace: crate::memory::types::DEFAULT_NAMESPACE.to_string(),
            access_count: 0,
            recall_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "unknown".to_string(),
            author_type: "agent".to_string(),
            author: None,
        }
    }

    #[test]
    fn plan_room_batches_real_memories() {
        let uteke = Uteke::open(":memory:").unwrap();
        uteke.store.create_room("r1", None, "default").unwrap();
        for i in 0..6 {
            let id = format!("m{i}");
            uteke
                .store
                .insert(&mem(&id, &format!("Fact {i} about caching")))
                .unwrap();
            uteke
                .store
                .link_memory_to_room("r1", &id, "tester", "member")
                .unwrap();
        }
        let dry = plan_room(&uteke, "r1").unwrap();
        assert_eq!(dry.plan.total_memories, 6);
        assert!(
            !dry.plan.batches.is_empty(),
            "6 similar memories should form at least one batch"
        );
    }

    #[test]
    fn deprecate_and_insert_via_trait() {
        // Exercises the two write paths the executor relies on.
        let uteke = Uteke::open(":memory:").unwrap();
        uteke.store.create_room("r1", None, "default").unwrap();
        let mut m = mem("m1", "caching fact");
        ConsolidationStore::insert_memory(&uteke, &mut m).unwrap();
        assert!(!m.id.is_empty());
        uteke
            .store
            .link_memory_to_room("r1", &m.id, "tester", "member")
            .unwrap();
        ConsolidationStore::deprecate_memory(&uteke, &m.id, "consolidated").unwrap();
        let after = ConsolidationStore::room_memories(&uteke, "r1").unwrap();
        assert!(
            after.is_empty(),
            "deprecated memory must not appear in room recall"
        );
    }

    #[test]
    fn a_consolidated_memory_goes_through_the_privacy_filter() {
        let mut uteke = Uteke::open(":memory:").unwrap();
        uteke.set_privacy_config(crate::privacy::PrivacyConfig::default());
        let token = format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5");
        let mut m = mem("m1", &format!("the LLM wrote {token}"));
        m.metadata = serde_json::json!({"summary": format!("see {token}")});
        ConsolidationStore::insert_memory(&uteke, &mut m).unwrap();
        let stored = uteke.store.get_by_id("m1").unwrap().unwrap();
        assert!(
            !stored.content.contains("aB3dE5gH7jK9"),
            "{}",
            stored.content
        );
        assert!(!stored.metadata.to_string().contains("aB3dE5gH7jK9"));
        assert_eq!(stored.metadata["redacted"], true);

        let mut strict = Uteke::open(":memory:").unwrap();
        strict.set_privacy_config(crate::privacy::PrivacyConfig {
            mode: crate::privacy::PrivacyMode::Strict,
            allowlist: Vec::new(),
        });
        let mut m = mem("m2", &format!("the LLM wrote {token}"));
        assert!(ConsolidationStore::insert_memory(&strict, &mut m).is_err());
        assert!(strict.store.get_by_id("m2").unwrap().is_none());
    }

    #[test]
    fn a_consolidated_record_leaves_the_same_side_effects_as_remember() {
        use crate::embed::Embedder;
        let uteke = crate::test_support::open_keyword();
        uteke.store.create_room("r1", None, "default").unwrap();
        let recall = || {
            uteke
                .recall_hybrid(
                    "caching",
                    10,
                    None,
                    Some("default"),
                    crate::RecallStrategy::Hybrid,
                    0.0,
                )
                .unwrap()
        };
        assert!(recall().is_empty());

        let text = "caching keeps hot data close to the reader";
        let mut m = mem("c1", text);
        m.embedding = crate::test_support::KeywordEmbedder.embed(text).unwrap();
        m.metadata = serde_json::json!({"room_id": "r1"});
        ConsolidationStore::insert_memory(&uteke, &mut m).unwrap();

        let events = uteke.timeline("c1", 20).unwrap();
        assert!(
            events.iter().any(|e| e.event_type == "created"),
            "{events:?}"
        );
        assert!(
            uteke
                .provenance("c1")
                .unwrap()
                .unwrap()
                .source_hash
                .is_some()
        );
        assert_eq!(
            recall().len(),
            1,
            "the insert invalidated the cached recall"
        );
        assert_eq!(
            ConsolidationStore::room_memories(&uteke, "r1")
                .unwrap()
                .len(),
            1,
            "linked to its room"
        );
    }

    /// #1594: the executor wrote the room under `room`, the reader looked for
    /// `room_id`, so a consolidated record was never linked to its room.
    #[test]
    fn a_record_built_by_the_executor_is_linked_to_its_room() {
        let uteke = crate::test_support::open_keyword();
        uteke.store.create_room("r1", None, "default").unwrap();
        let mut m = mem("c1", "caching keeps hot data close to the reader");
        m.metadata =
            consolidation_exec::consolidation_metadata(&["s1".to_string(), "s2".to_string()], "r1");
        assert_eq!(m.metadata["room_id"], "r1");
        assert_eq!(
            m.metadata["consolidated_from"],
            serde_json::json!(["s1", "s2"])
        );
        ConsolidationStore::insert_memory(&uteke, &mut m).unwrap();
        let linked = ConsolidationStore::room_memories(&uteke, "r1").unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].id, "c1");
    }

    /// A record carrying only the old `room` key is still recognised.
    #[test]
    fn the_old_room_key_is_still_recognised() {
        let uteke = crate::test_support::open_keyword();
        uteke.store.create_room("r1", None, "default").unwrap();
        let mut m = mem("c2", "indexes trade write cost for read speed");
        m.metadata = serde_json::json!({"room": "r1"});
        ConsolidationStore::insert_memory(&uteke, &mut m).unwrap();
        let linked = ConsolidationStore::room_memories(&uteke, "r1").unwrap();
        assert_eq!(linked.len(), 1);
    }

    /// A secret in consolidated text is masked (or refused in strict mode)
    /// before the embedder sees it.
    #[test]
    fn consolidated_text_is_filtered_before_it_is_embedded() {
        use crate::privacy::{PrivacyConfig, PrivacyMode};
        use crate::test_support::{RecordingEmbedder, open_with_embedder};
        let token = format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5");
        let text = format!("the deploy token is {token}");

        let (e, seen) = RecordingEmbedder::new();
        let mut u = open_with_embedder(Box::new(e));
        u.set_privacy_config(PrivacyConfig::default());
        let v = u.embed_content(&text).unwrap();
        assert!(!v.is_empty());
        let sent = seen.lock().unwrap().join("\n");
        assert!(
            !sent.contains(&token),
            "the embedder received a secret: {sent}"
        );
        assert!(sent.contains("the deploy token is"));

        let (e, seen) = RecordingEmbedder::new();
        let mut u = open_with_embedder(Box::new(e));
        u.set_privacy_config(PrivacyConfig {
            mode: PrivacyMode::Strict,
            ..Default::default()
        });
        assert!(u.embed_content(&text).is_err(), "strict mode refuses");
        assert!(
            seen.lock().unwrap().is_empty(),
            "a refused text was embedded"
        );
    }
}
