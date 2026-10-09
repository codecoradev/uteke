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
        let room_id = memory
            .metadata
            .get("room_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        // Persist (transactional insert, mirrors crud flow).
        self.store().insert(memory)?;

        // Insert into the vector index so recall finds it; roll back the row
        // if indexing fails so we never leave an un-searchable memory behind.
        if !memory.embedding.is_empty() {
            let key = memory.id.clone();
            let embedding = memory.embedding.clone();
            if let Err(e) = self.add_to_index(&key, &embedding) {
                let _ = self.store().delete(&memory.id);
                return Err(e);
            }
        }

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
        self.embed_text(content)
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
}
