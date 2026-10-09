//! Room semantic segmentation (PoC for #1088, inspired by LycheeMemory V2,
//! arXiv:2608.12990).
//!
//! Groups a room's memories (chronologically ordered) into coherent semantic
//! segments by measuring embedding similarity between adjacent memories.
//! A segment boundary is placed when adjacent similarity drops below a
//! threshold, or when a maximum segment size is reached.
//!
//! This is intentionally LLM-free: it only uses the embeddings that already
//! exist on every memory. The resulting segments are the batching unit for
//! future segment-level LLM consolidation — one LLM call per segment instead
//! of one per memory.

use crate::error::Error;
use crate::memory::types::Memory;
use serde::{Deserialize, Serialize};

/// Default adjacent-similarity threshold below which a boundary is placed.
///
/// Embedding cosine similarity of adjacent same-topic memories is typically
/// well above 0.5 for the built-in 768-d model; unrelated memories land far
/// lower. 0.45 is a conservative default — tune per deployment.
pub const DEFAULT_BOUNDARY_THRESHOLD: f32 = 0.45;

/// Default maximum memories per segment. Mirrors the segment-batching idea:
/// large enough to amortize LLM calls, small enough to stay coherent.
pub const DEFAULT_MAX_SEGMENT_SIZE: usize = 12;

/// Default minimum memories per segment; shorter runs are merged into the
/// previous segment to avoid tiny, low-value consolidation batches.
pub const DEFAULT_MIN_SEGMENT_SIZE: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWithSim {
    pub memory: Memory,
    /// Cosine similarity to the previous memory in the room (None for the
    /// first memory — it always starts a segment).
    pub sim_to_prev: Option<f32>,
    /// Index of the segment this memory belongs to.
    pub segment: usize,
    /// Whether this memory starts a new segment.
    pub starts_segment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub index: usize,
    pub memory_ids: Vec<String>,
    /// Index range [start, end) into the chronological memory list.
    pub range: (usize, usize),
    /// Mean adjacent similarity inside the segment (coherence signal).
    pub mean_sim: f32,
    /// Boundary reason for the segment that follows this one, if any.
    pub boundary_reason: Option<BoundaryReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryReason {
    /// Adjacent similarity dropped below the threshold.
    Semantic,
    /// Segment reached the maximum size.
    MaxSize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentationResult {
    pub room_id: String,
    pub total_memories: usize,
    pub segments: Vec<Segment>,
    /// Per-memory assignment with adjacent similarities, chronological order.
    pub memories: Vec<MemoryWithSim>,
    pub threshold: f32,
}

/// Cosine similarity for equal-length f32 vectors.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

impl crate::Uteke {
    /// Segment a room's memories by semantic boundaries (LLM-free, PoC #1088).
    ///
    /// Memories are ordered chronologically; a new segment starts when the
    /// cosine similarity to the previous memory drops below `threshold`, or
    /// when the current segment reaches `max_size`. Runs shorter than
    /// `min_size` are merged into the previous segment.
    pub fn room_segments(
        &self,
        room_id: &str,
        threshold: f32,
        max_size: usize,
        min_size: usize,
    ) -> Result<SegmentationResult, Error> {
        // Bulk-fetch room memories in one query, chronological order.
        let memories = self.store.recall_room(room_id, None, 0)?;
        self.room_segments_inner(room_id, &memories, threshold, max_size, min_size)
    }

    /// Segmentation over caller-supplied memories (already fetched — avoids
    /// a second query when the caller already holds the room's memories).
    pub fn room_segments_inner(
        &self,
        room_id: &str,
        memories: &[Memory],
        threshold: f32,
        max_size: usize,
        min_size: usize,
    ) -> Result<SegmentationResult, Error> {
        let mut memories: Vec<Memory> = memories.to_vec();
        if memories.is_empty() {
            return Ok(SegmentationResult {
                room_id: room_id.to_string(),
                total_memories: 0,
                segments: vec![],
                memories: vec![],
                threshold,
            });
        }

        memories.sort_by_key(|a| a.created_at);

        // Adjacent similarities.
        let mut sims: Vec<Option<f32>> = Vec::with_capacity(memories.len());
        sims.push(None);
        for w in memories.windows(2) {
            let s = cosine(&w[0].embedding, &w[1].embedding);
            sims.push(Some(s));
        }

        // First pass: boundaries.
        let mut assignments: Vec<(usize, bool, BoundaryReason)> =
            Vec::with_capacity(memories.len());
        assignments.push((0, true, BoundaryReason::Semantic)); // first memory starts segment 0
        let mut cur = 0usize;
        let mut cur_len = 1usize;
        for sim in sims.iter().skip(1) {
            let sim = sim.unwrap_or(0.0);
            let semantic_break = sim < threshold;
            let size_break = cur_len >= max_size;
            if semantic_break || size_break {
                cur += 1;
                cur_len = 1;
                assignments.push((
                    cur,
                    true,
                    if semantic_break {
                        BoundaryReason::Semantic
                    } else {
                        BoundaryReason::MaxSize
                    },
                ));
            } else {
                cur_len += 1;
                assignments.push((cur, false, BoundaryReason::Semantic));
            }
        }

        // Second pass: merge runs shorter than min_size into the previous
        // segment (keeps at most the first segment unmerged).
        let mut merged: Vec<usize> = assignments.iter().map(|a| a.0).collect();
        let mut i = 1usize;
        while i < merged.len() {
            let start = i;
            while i < merged.len() && merged[i] == merged[start] {
                i += 1;
            }
            let len = i - start;
            if len < min_size && merged[start] > 0 {
                // Merge into the *actual* preceding segment (which may itself
                // have been merged earlier in this pass) — not the raw index.
                let prev = merged[start - 1];
                for m in merged.iter_mut().skip(start).take(len) {
                    *m = prev;
                }
                // Re-scan from the start of the merged segment in case the
                // merge made the previous segment exceed max_size; we accept
                // this overshoot deliberately (coherence beats hard caps).
            }
        }
        // Compact segment indices.
        let mut remap: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        let mut next = 0usize;
        for m in merged.iter_mut() {
            let e = remap.entry(*m).or_insert_with(|| {
                let v = next;
                next += 1;
                v
            });
            *m = *e;
        }

        // Build per-memory view.
        let mut view: Vec<MemoryWithSim> = Vec::with_capacity(memories.len());
        for (idx, mem) in memories.into_iter().enumerate() {
            let seg = merged[idx];
            let starts = idx == 0 || merged[idx - 1] != seg;
            view.push(MemoryWithSim {
                memory: mem,
                sim_to_prev: sims[idx],
                segment: seg,
                starts_segment: starts,
            });
        }

        // Build segments with coherence stats and boundary reasons.
        let n_segments = next;
        let mut segments: Vec<Segment> = (0..n_segments)
            .map(|i| Segment {
                index: i,
                memory_ids: vec![],
                range: (usize::MAX, 0),
                mean_sim: 0.0,
                boundary_reason: None,
            })
            .collect();
        for (idx, m) in view.iter().enumerate() {
            let s = &mut segments[m.segment];
            if s.memory_ids.is_empty() {
                s.range.0 = idx;
            }
            s.range.1 = idx + 1;
            s.memory_ids.push(m.memory.id.clone());
            if let Some(sim) = m.sim_to_prev {
                // Only count within-segment adjacency for coherence.
                if idx > 0 && view[idx - 1].segment == m.segment {
                    s.mean_sim += sim;
                }
            }
        }
        for s in segments.iter_mut() {
            let n = s.memory_ids.len();
            let inner = n.saturating_sub(1);
            if inner > 0 {
                s.mean_sim /= inner as f32;
            } else {
                s.mean_sim = 1.0;
            }
        }
        // Boundary reason of the NEXT segment lives on the boundary memory.
        for idx in 1..view.len() {
            if view[idx].starts_segment {
                // Find original assignment reason (pre-merge index may differ;
                // reason only meaningful for semantic/max-size boundaries).
                let reason = sims[idx]
                    .map(|sim| {
                        if sim < threshold {
                            BoundaryReason::Semantic
                        } else {
                            BoundaryReason::MaxSize
                        }
                    })
                    .unwrap_or(BoundaryReason::Semantic);
                segments[view[idx].segment].boundary_reason = Some(reason);
            }
        }

        Ok(SegmentationResult {
            room_id: room_id.to_string(),
            total_memories: view.len(),
            segments,
            memories: view,
            threshold,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(seed: f32) -> Vec<f32> {
        // Simple 4-d embeddings: same seed → identical vector.
        vec![seed, seed * 0.5, 1.0 - seed, seed + 0.1]
    }

    #[test]
    fn cosine_basics() {
        assert!((cosine(&v(0.2), &v(0.2)) - 1.0).abs() < 1e-5);
        assert!(cosine(&v(0.0), &v(1.0)) < 0.5);
        assert_eq!(cosine(&[], &[]), 0.0);
    }

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
    fn segments_two_topic_clusters() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        uteke.store.create_room("r1", None, "default").unwrap();

        // Cluster A (similar embeddings), then cluster B (different).
        let a = v(0.1);
        let mut b = v(0.9);
        // ensure b differs strongly from a
        b[2] = 0.05;
        let ids = ["m1", "m2", "m3", "m4", "m5", "m6"];
        let embs = [
            a.clone(),
            a.clone(),
            a.clone(),
            b.clone(),
            b.clone(),
            b.clone(),
        ];
        for (id, e) in ids.iter().zip(embs.iter()) {
            let m = make_memory(id, id, e.clone());
            uteke.store.insert(&m).unwrap();
            uteke
                .store
                .link_memory_to_room("r1", id, "tester", "participant")
                .unwrap();
        }

        let res = uteke.room_segments("r1", 0.45, 12, 2).unwrap();
        assert_eq!(res.total_memories, 6);
        assert_eq!(
            res.segments.len(),
            2,
            "expected 2 segments: {:?}",
            res.segments
                .iter()
                .map(|s| s.memory_ids.len())
                .collect::<Vec<_>>()
        );
        assert_eq!(res.segments[0].memory_ids, vec!["m1", "m2", "m3"]);
        assert_eq!(res.segments[1].memory_ids, vec!["m4", "m5", "m6"]);
        assert!(res.segments[1].boundary_reason == Some(BoundaryReason::Semantic));
        // Within-cluster coherence should be high.
        assert!(res.segments[0].mean_sim > 0.99);
    }

    #[test]
    fn max_size_split_single_topic() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        uteke.store.create_room("r2", None, "default").unwrap();
        let e = v(0.3);
        for i in 0..7 {
            let id = format!("s{i}");
            let m = make_memory(&id, &id, e.clone());
            uteke.store.insert(&m).unwrap();
            uteke
                .store
                .link_memory_to_room("r2", &id, "tester", "participant")
                .unwrap();
        }
        // Same topic everywhere, but max_size=3 forces splits; the 1-item
        // tail run is below min_size=2 and merges into the previous segment.
        let res = uteke.room_segments("r2", 0.45, 3, 2).unwrap();
        assert_eq!(res.total_memories, 7);
        assert_eq!(res.segments.len(), 2);
        assert_eq!(res.segments[0].memory_ids.len(), 3);
        assert_eq!(res.segments[1].memory_ids.len(), 4);
        // The merge boundary was a max-size split, not semantic.
        assert!(res.segments[1].boundary_reason == Some(BoundaryReason::MaxSize));
    }

    #[test]
    fn empty_room_returns_empty() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let res = uteke.room_segments("nope", 0.45, 12, 3).unwrap();
        assert_eq!(res.total_memories, 0);
        assert!(res.segments.is_empty());
    }

    #[test]
    fn chains_of_short_runs_merge_progressively() {
        // Regression: merging a short run must target the *actual* preceding
        // segment, even when that segment was itself merged earlier in the
        // pass (previously `merged[start] - 1` produced orphan indices).
        // Layout: A(2) B(1) C(1) D(2) — adjacent B/C are dissimilar from
        // their neighbours, so first pass yields 4 segments of sizes 2,1,1,2.
        // With min_size=3, both B and C must merge into A (3 segments -> 2).
        let uteke = crate::Uteke::open(":memory:").unwrap();
        uteke.store.create_room("r3", None, "default").unwrap();
        // Orthogonal unit vectors -> pairwise cosine ≈ 0 (all < threshold).
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        let c = vec![0.0, 0.0, 1.0, 0.0];
        let d = vec![0.0, 0.0, 0.0, 1.0];
        let embs = [a.clone(), a.clone(), b, c, d.clone(), d.clone()];
        for (i, e) in embs.into_iter().enumerate() {
            let id = format!("m{i}");
            let m = make_memory(&id, &id, e);
            uteke.store.insert(&m).unwrap();
            uteke
                .store
                .link_memory_to_room("r3", &id, "tester", "participant")
                .unwrap();
        }
        let res = uteke.room_segments("r3", 0.45, 12, 3).unwrap();
        let sizes: Vec<usize> = res.segments.iter().map(|s| s.memory_ids.len()).collect();
        // B, C, D runs all merge progressively into the leading A run: [6].
        assert_eq!(sizes, vec![6], "sizes: {sizes:?}");
        // No tail segment below min_size may survive.
        assert!(
            sizes.iter().skip(1).all(|&s| s >= 3),
            "tail segments below min_size: {sizes:?}"
        );
    }

    #[test]
    fn room_summary_includes_segments_when_embeddings_exist() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        uteke.store.create_room("r4", None, "default").unwrap();
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        let embs = [
            a.clone(),
            a.clone(),
            a.clone(),
            b.clone(),
            b.clone(),
            b.clone(),
        ];
        for (i, e) in embs.into_iter().enumerate() {
            let id = format!("m{i}");
            let m = make_memory(&id, &id, e);
            uteke.store.insert(&m).unwrap();
            uteke
                .store
                .link_memory_to_room("r4", &id, "tester", "participant")
                .unwrap();
        }
        let summary = uteke.room_summary("r4").unwrap().expect("summary exists");
        let segs = summary.segments.expect("segments present");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].memory_ids.len(), 3);
        assert_eq!(segs[1].memory_ids.len(), 3);

        // No embeddings -> segments stay None.
        uteke.store.create_room("r5", None, "default").unwrap();
        let m = make_memory("plain", "plain", vec![]);
        uteke.store.insert(&m).unwrap();
        uteke
            .store
            .link_memory_to_room("r5", "plain", "tester", "participant")
            .unwrap();
        let s5 = uteke.room_summary("r5").unwrap().expect("summary exists");
        assert!(s5.segments.is_none());
    }
}
