//! Orphan detection — find disconnected memories (#351).
//!
//! A memory is "orphaned" when it has no graph edges (incoming or outgoing),
//! has never been recalled, is not pinned, and its importance is below a
//! configurable threshold. Such memories are low-value: they can't be reached
//! via graph traversal and dilute recall quality.
//!
//! Detection is a single SQL pass over `memories` + `memory_edges` — no O(n²)
//! scan. The result is a list of [`OrphanMemory`] entries with an
//! `orphan_score` (0.0..=1.0) for ranking.

use crate::error::Error;
use crate::memory::store::Store;
use crate::memory::types::Memory;
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// Default importance threshold below which a memory can be considered an
/// orphan candidate (#351).
pub const DEFAULT_ORPHAN_THRESHOLD: f64 = 0.3;

/// A memory flagged as an orphan candidate (#351).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrphanMemory {
    /// The orphaned memory.
    pub memory: Memory,
    /// 0.0..=1.0 — higher means stronger orphan signal.
    pub orphan_score: f32,
    /// Number of outgoing edges (always 0 for a true orphan).
    pub outgoing_edges: usize,
    /// Number of incoming edges (always 0 for a true orphan).
    pub incoming_edges: usize,
}

impl Store {
    /// Detect orphan memories in a namespace (#351).
    ///
    /// A memory is an orphan when ALL of:
    /// 1. No outgoing edges (forward or backlink types).
    /// 2. No incoming edges.
    /// 3. `access_count == 0`.
    /// 4. Not pinned.
    /// 5. `importance < threshold` (default 0.3).
    ///
    /// Returns orphans sorted by `orphan_score` descending (strongest first).
    /// Set `limit = 0` for no cap.
    pub fn find_orphans(
        &self,
        namespace: Option<&str>,
        threshold: f64,
        limit: usize,
    ) -> Result<Vec<OrphanMemory>, Error> {
        // Single SQL pass: LEFT JOIN against the edge table twice (for
        // outgoing and incoming), filtering memories with zero matches on
        // both sides plus the remaining criteria.
        let sql = r#"
            SELECT m.id, m.content, m.embedding, m.tags, m.metadata,
                   m.created_at, m.updated_at, m.namespace, m.access_count,
                   m.last_accessed, m.deprecated, m.valid_from, m.valid_until,
                   m.memory_type, m.importance, m.pinned, m.content_type, m.slug
            FROM memories m
            LEFT JOIN memory_edges out_e ON out_e.source_id = m.id
            LEFT JOIN memory_edges in_e  ON in_e.target_id  = m.id
            WHERE out_e.id IS NULL
              AND in_e.id IS NULL
              AND m.access_count = 0
              AND m.pinned = 0
              AND m.importance < ?1
              AND (?2 IS NULL OR m.namespace = ?2)
            GROUP BY m.id
            ORDER BY m.importance ASC, m.created_at ASC
        "#;

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("prepare find_orphans", e))?;
        let ns_param: Option<&str> = namespace;
        let rows = stmt
            .query_map(params![threshold, ns_param], |row| {
                crate::memory::store::row_to_memory(row)
            })
            .map_err(|e| Error::db("find_orphans query", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::db("find_orphans row iter", e))?;

        // Compute orphan_score for every candidate first, then sort +
        // truncate. CodeCora #389: the previous version broke out of the
        // loop as soon as `orphans.len() >= limit`, but the subsequent
        // sort_by could reorder those rows — a higher-score orphan that
        // appeared later in the SQL result set would be dropped. Fetching
        // all candidates first guarantees the truncate picks the true
        // top-N by orphan_score.
        //
        // Orphan result sets are typically small (each match has zero
        // edges and zero accesses), so the full fetch is cheap.
        let mut orphans: Vec<OrphanMemory> = rows
            .into_iter()
            .map(|memory| {
                let orphan_score = compute_orphan_score(&memory, 0, 0);
                OrphanMemory {
                    memory,
                    orphan_score,
                    outgoing_edges: 0,
                    incoming_edges: 0,
                }
            })
            .collect();

        // Sort by orphan_score descending (strongest orphan signal first).
        orphans.sort_by(|a, b| {
            b.orphan_score
                .partial_cmp(&a.orphan_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if limit > 0 {
            orphans.truncate(limit);
        }
        Ok(orphans)
    }
}

/// Compute the orphan score for a memory (0.0..=1.0, higher = more orphaned).
///
/// Weighted blend of:
/// - `(1 - edge_density)` × 0.4 — fully disconnected = 1.0
/// - `(1 - access_frequency_normalized)` × 0.3 — never accessed = 1.0
/// - `(1 - importance)` × 0.3 — low importance = 1.0
pub fn compute_orphan_score(memory: &Memory, outgoing: usize, incoming: usize) -> f32 {
    let edge_density = ((outgoing + incoming).min(10) as f32) / 10.0;
    let access_freq = ((memory.access_count.max(1) as f32).log10() / 3.0).clamp(0.0, 1.0);
    let importance = memory.importance as f32;

    let score = (1.0 - edge_density) * 0.4 + (1.0 - access_freq) * 0.3 + (1.0 - importance) * 0.3;
    score.clamp(0.0, 1.0)
}

impl crate::Uteke {
    /// Find orphan memories (#351). See [`Store::find_orphans`].
    pub fn find_orphans(
        &self,
        namespace: Option<&str>,
        threshold: f64,
        limit: usize,
    ) -> Result<Vec<OrphanMemory>, Error> {
        self.store.find_orphans(namespace, threshold, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(content: &str, importance: f64) -> Memory {
        Memory {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.to_string(),
            embedding: vec![],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
        }
    }

    #[test]
    fn orphan_score_disconnected_low_importance() {
        let m = mem("orphan", 0.1);
        let s = compute_orphan_score(&m, 0, 0);
        assert!(s > 0.8, "strong orphan expected, got {s}");
    }

    #[test]
    fn orphan_score_connected_high_importance() {
        let mut m = mem("well-connected", 0.9);
        m.access_count = 100; // frequently accessed
        let s = compute_orphan_score(&m, 5, 3);
        assert!(s < 0.3, "weak orphan expected, got {s}");
    }

    #[test]
    fn find_orphans_returns_isolated_low_importance() {
        let store = Store::open(":memory:").unwrap();
        let orphan = mem("orphan candidate", 0.1);
        let connected = mem("well-connected", 0.1);
        let target = mem("target", 0.1);
        store.insert(&orphan).unwrap();
        store.insert(&connected).unwrap();
        store.insert(&target).unwrap();
        // Connect "connected" to "target" so it's no longer an orphan.
        store
            .add_memory_edge(&connected.id, &target.id, "references")
            .unwrap();

        let orphans = store.find_orphans(None, 0.3, 0).unwrap();
        let ids: Vec<_> = orphans.iter().map(|o| o.memory.id.as_str()).collect();
        // `target` has an incoming edge (referenced_by from #350), so it's NOT an orphan.
        // `connected` has outgoing edge, NOT an orphan.
        // Only the true orphan should appear.
        assert!(ids.contains(&orphan.id.as_str()), "orphan must be detected");
        assert!(
            !ids.contains(&connected.id.as_str()),
            "connected must not be flagged"
        );
        assert!(
            !ids.contains(&target.id.as_str()),
            "target must not be flagged (has incoming edge)"
        );
    }

    #[test]
    fn find_orphans_respects_threshold() {
        let store = Store::open(":memory:").unwrap();
        let low = mem("low importance", 0.1);
        let high = mem("high importance", 0.5);
        store.insert(&low).unwrap();
        store.insert(&high).unwrap();

        // Threshold 0.3 → only low is flagged.
        let orphans = store.find_orphans(None, 0.3, 0).unwrap();
        let ids: Vec<_> = orphans.iter().map(|o| o.memory.id.as_str()).collect();
        assert!(ids.contains(&low.id.as_str()));
        assert!(!ids.contains(&high.id.as_str()));

        // Threshold 0.6 → both are flagged.
        let orphans = store.find_orphans(None, 0.6, 0).unwrap();
        assert_eq!(orphans.len(), 2);
    }

    #[test]
    fn find_orphans_excludes_pinned_and_accessed() {
        let store = Store::open(":memory:").unwrap();
        let mut pinned = mem("pinned", 0.1);
        pinned.pinned = true;
        let mut accessed = mem("accessed", 0.1);
        accessed.access_count = 5;
        store.insert(&pinned).unwrap();
        store.insert(&accessed).unwrap();

        let orphans = store.find_orphans(None, 0.3, 0).unwrap();
        let ids: Vec<_> = orphans.iter().map(|o| o.memory.id.as_str()).collect();
        assert!(
            !ids.contains(&pinned.id.as_str()),
            "pinned must not be orphan"
        );
        assert!(
            !ids.contains(&accessed.id.as_str()),
            "accessed must not be orphan"
        );
    }

    #[test]
    fn find_orphans_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        let mut a = mem("ns-a orphan", 0.1);
        a.namespace = "ns-a".to_string();
        let mut b = mem("ns-b orphan", 0.1);
        b.namespace = "ns-b".to_string();
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        let ns_a = store.find_orphans(Some("ns-a"), 0.3, 0).unwrap();
        assert_eq!(ns_a.len(), 1);
        assert_eq!(ns_a[0].memory.id, a.id);
    }

    #[test]
    fn find_orphans_limit() {
        let store = Store::open(":memory:").unwrap();
        for _ in 0..10 {
            let m = mem("orphan", 0.1);
            store.insert(&m).unwrap();
        }
        let limited = store.find_orphans(None, 0.3, 3).unwrap();
        assert_eq!(limited.len(), 3);
    }
}
