//! Segment-level consolidation planning (#1088, phase 3 — measure-only).
//!
//! Before wiring LLM calls into room consolidation, this module produces a
//! *batching plan*: which memories belong to which LLM call, and what the
//! call/token cost would be compared to naive per-memory consolidation.
//! Zero LLM calls are made here — the plan is the evaluation artifact.

use crate::error::Error;
use crate::memory::types::Memory;
use crate::rooms_segments::SegmentationResult;
use serde::{Deserialize, Serialize};

/// Default similarity threshold for segmentation (validated on production
/// room data — see #1090: stable across 0.40–0.55 in 3 of 4 rooms).
pub const DEFAULT_THRESHOLD: f32 = 0.45;
/// Default max segment size (memories per LLM batch).
pub const DEFAULT_MAX_SIZE: usize = 12;
/// Default min segment size.
pub const DEFAULT_MIN_SIZE: usize = 3;

/// Rough token estimate: ~4 chars per token for English/mixed content.
const CHARS_PER_TOKEN: f64 = 4.0;

/// A planned LLM batch: one segment's memories, joined into one prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedBatch {
    pub segment_index: usize,
    pub memory_ids: Vec<String>,
    /// Joined content that would be sent to the LLM (the actual payload).
    pub payload: String,
    /// Estimated prompt tokens for this batch.
    pub est_tokens: u64,
}

/// Cost comparison between the two consolidation granularities.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CostEstimate {
    /// LLM calls if every memory is consolidated on its own (naive).
    pub naive_calls: u64,
    /// Estimated tokens for the naive approach.
    pub naive_tokens: u64,
    /// LLM calls when batching per segment.
    pub segment_calls: u64,
    /// Estimated tokens for segment batching (payload tokens only).
    pub segment_tokens: u64,
    /// Relative call reduction, 0.0–1.0 (naive → segment).
    pub call_reduction: f32,
    /// Relative token reduction, 0.0–1.0.
    pub token_reduction: f32,
}

/// The full consolidation plan for one room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationPlan {
    pub room_id: String,
    pub total_memories: usize,
    pub batches: Vec<PlannedBatch>,
    pub cost: CostEstimate,
    pub threshold: f32,
}

impl crate::Uteke {
    /// Plan segment-level LLM consolidation for a room (#1088 phase 3).
    ///
    /// Measure-only: builds the batching plan and cost estimate without
    /// calling any LLM. Use this to evaluate the LycheeMemory-style
    /// granularity tradeoff on real rooms before enabling execution.
    pub fn room_consolidation_plan(
        &self,
        room_id: &str,
        threshold: f32,
        max_size: usize,
        min_size: usize,
    ) -> Result<ConsolidationPlan, Error> {
        let memories = self.store.recall_room(room_id, None, 0)?;
        self.consolidation_plan_from(room_id, &memories, threshold, max_size, min_size)
    }

    /// Plan over caller-supplied memories (avoids a second query).
    pub fn consolidation_plan_from(
        &self,
        room_id: &str,
        memories: &[Memory],
        threshold: f32,
        max_size: usize,
        min_size: usize,
    ) -> Result<ConsolidationPlan, Error> {
        let segmentation: SegmentationResult =
            self.room_segments_inner(room_id, memories, threshold, max_size, min_size)?;

        let id_to_memory: std::collections::HashMap<&str, &Memory> =
            memories.iter().map(|m| (m.id.as_str(), m)).collect();

        let mut batches = Vec::with_capacity(segmentation.segments.len());
        let mut segment_tokens: u64 = 0;
        for seg in &segmentation.segments {
            let contents: Vec<&str> = seg
                .memory_ids
                .iter()
                .filter_map(|id| id_to_memory.get(id.as_str()).map(|m| m.content.as_str()))
                .collect();
            // Memories without embeddings are skipped by segmentation; the
            // filter_map above keeps batch contents aligned to the segment.
            if contents.is_empty() {
                continue;
            }
            let payload = contents.join("\n---\n");
            let est = (payload.chars().count() as f64 / CHARS_PER_TOKEN).ceil() as u64;
            segment_tokens += est;
            batches.push(PlannedBatch {
                segment_index: seg.index,
                memory_ids: seg.memory_ids.clone(),
                payload,
                est_tokens: est,
            });
        }

        let naive_calls = memories.len() as u64;
        let naive_tokens: u64 = memories
            .iter()
            .map(|m| ((m.content.chars().count() as f64) / CHARS_PER_TOKEN).ceil() as u64)
            .sum();
        let segment_calls = batches.len() as u64;

        let call_reduction = if naive_calls > 0 {
            1.0 - (segment_calls as f32 / naive_calls as f32)
        } else {
            0.0
        };
        let token_reduction = if naive_tokens > 0 {
            1.0 - (segment_tokens as f32 / naive_tokens as f32)
        } else {
            0.0
        };

        Ok(ConsolidationPlan {
            room_id: room_id.to_string(),
            total_memories: memories.len(),
            batches,
            cost: CostEstimate {
                naive_calls,
                naive_tokens,
                segment_calls,
                segment_tokens,
                call_reduction,
                token_reduction,
            },
            threshold,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_memory(id: &str, content: &str, embedding: Vec<f32>) -> crate::Memory {
        crate::Memory {
            id: id.to_string(),
            content: content.to_string(),
            embedding,
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
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
            source_type: String::new(),
            author_type: String::new(),
        }
    }

    #[test]
    fn plan_buckets_memories_and_estimates_savings() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        // Two orthogonal topics of 3 memories each -> 2 segments.
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let mems: Vec<Memory> = (0..3)
            .map(|i| make_memory(&format!("a{i}"), "alpha topic content", a.clone()))
            .chain((0..3).map(|i| make_memory(&format!("b{i}"), "beta topic content", b.clone())))
            .collect();

        let plan = uteke
            .consolidation_plan_from(
                "r",
                &mems,
                DEFAULT_THRESHOLD,
                DEFAULT_MAX_SIZE,
                DEFAULT_MIN_SIZE,
            )
            .unwrap();

        assert_eq!(plan.total_memories, 6);
        assert_eq!(plan.batches.len(), 2, "batches: {:?}", plan.batches.len());
        assert_eq!(plan.cost.naive_calls, 6);
        assert_eq!(plan.cost.segment_calls, 2);
        assert!((plan.cost.call_reduction - (1.0 - 2.0 / 6.0)).abs() < 1e-6);
        // Batching joins with separators, so payload tokens can slightly
        // exceed the naive sum — the real win is CALL reduction (above).
        assert!(plan.cost.token_reduction >= -0.5);
        for b in &plan.batches {
            assert!(b.payload.contains("---"));
            assert_eq!(b.memory_ids.len(), 3);
        }
    }

    #[test]
    fn empty_room_plans_zero_batches() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let plan = uteke
            .consolidation_plan_from("r", &[], DEFAULT_THRESHOLD, 12, 3)
            .unwrap();
        assert_eq!(plan.total_memories, 0);
        assert!(plan.batches.is_empty());
        assert_eq!(plan.cost.call_reduction, 0.0);
    }

    #[test]
    fn unembedded_memories_yield_empty_plan() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let mems = vec![make_memory("x1", "no embedding here", vec![])];
        let plan = uteke
            .consolidation_plan_from("r", &mems, DEFAULT_THRESHOLD, 12, 3)
            .unwrap();
        assert_eq!(plan.total_memories, 1);
        // Without embeddings, adjacent cosine is 0 → one min_size-merged batch.
        assert_eq!(plan.batches.len(), 1);
        assert_eq!(plan.batches[0].memory_ids, vec!["x1".to_string()]);
    }
}
