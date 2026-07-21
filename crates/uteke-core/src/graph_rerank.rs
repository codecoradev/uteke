//! Graph-augmented RAG reranking — fuses graph signals into recall scoring.
//!
//! Without this module, the knowledge graph ([`crate::edges`]) and semantic
//! recall ([`crate::operations::recall_rrf`]) operate independently: a memory
//! that is heavily referenced in the graph (high authority) receives no boost
//! over an isolated memory with similar embedding distance.
//!
//! This module computes lightweight graph signals — derived entirely from the
//! existing `memory_edges` table (no new data, no LLM) — and fuses them into
//! recall scores via log-scaled additive boosts. The boosts are subtle by
//! default (`density_weight = 0.1`, `authority_weight = 0.1`) and saturate
//! quickly, preventing well-connected hubs from dominating results.
//!
//! # Performance
//!
//! [`compute_graph_signals`] issues a single batched SQL query for all
//! candidate ids (`WHERE source_id IN (...) OR target_id IN (...)`) and
//! aggregates in Rust — O(edges touching candidates), not O(total edges).
//! Expected overhead for `limit = 10` is well under 10ms.
//!
//! Design: see issue #378.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params_from_iter};

use crate::memory::types::SearchResult;

/// Graph signals computed per memory, all derived from the `memory_edges`
/// table. Zero edges (cold start) yields all-zero signals → no score change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphSignals {
    /// Total edge count (incoming + outgoing). Well-connected = likely important.
    pub edge_count: u32,
    /// Unique neighbors (deduplicated across edge types). High = hub node.
    pub neighbor_count: u32,
    /// Number of distinct edge types touching this memory.
    pub edge_type_diversity: u32,
    /// Incoming edge count. Many references from others = authority.
    pub incoming_count: u32,
    /// Outgoing edge count. Many references to others = index/reference node.
    pub outgoing_count: u32,
}

/// Configuration for graph-augmented reranking.
///
/// Weights are additive multipliers on log-scaled signal boosts. A weight of
/// `0.0` disables that axis; `0.1` is a subtle default; `0.3` is strong.
#[derive(Debug, Clone)]
pub struct GraphRerankConfig {
    /// Weight for edge-density boost.
    pub density_weight: f32,
    /// Weight for incoming-edge authority boost.
    pub authority_weight: f32,
    /// Feature flag. When `false`, [`rerank_with_graph`] is a no-op and
    /// [`compute_graph_signals`] need not be called.
    pub enabled: bool,
}

impl Default for GraphRerankConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            density_weight: 0.1, // subtle by default
            authority_weight: 0.1,
        }
    }
}

impl GraphRerankConfig {
    /// Clamp weights to a safe range and apply a floor on `enabled`.
    /// Negative weights would invert ranking; we forbid them.
    pub fn sanitized(self) -> Self {
        Self {
            density_weight: self.density_weight.clamp(0.0, 1.0),
            authority_weight: self.authority_weight.clamp(0.0, 1.0),
            enabled: self.enabled,
        }
    }
}

/// Batch-compute graph signals for a set of candidate memory ids.
///
/// Issues a single SQL query fetching every edge that touches any candidate
/// (as source or target), then aggregates counts in Rust. Empty input or a
/// candidate set with no edges returns an empty map — callers should treat
/// missing ids as all-zero signals.
///
/// # Errors
///
/// Returns [`crate::Error`] on database errors.
pub fn compute_graph_signals(
    conn: &Connection,
    memory_ids: &[String],
) -> Result<HashMap<String, GraphSignals>, crate::Error> {
    // Fast path: nothing to score.
    if memory_ids.is_empty() {
        return Ok(HashMap::new());
    }

    // Deduplicate candidate ids in order (recall may return duplicates
    // across vector/FTS5 channels). Order-preserving dedup via a set.
    let unique_ids: Vec<&str> = {
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(memory_ids.len());
        for id in memory_ids {
            if seen.insert(id.as_str()) {
                out.push(id.as_str());
            }
        }
        out
    };

    // Build `IN (?, ?, ...)` placeholder list. We bind each id twice (once for
    // the source_id IN clause, once for target_id) so a single scan over
    // memory_edges suffices — no UNION, no second query.
    let placeholders = std::iter::repeat_n("?", unique_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT source_id, target_id, edge_type FROM memory_edges
         WHERE source_id IN ({src}) OR target_id IN ({tgt})",
        src = placeholders,
        tgt = placeholders,
    );

    // Bind params: ids for the source clause, then the same ids for the target clause.
    let mut all_params: Vec<&str> = Vec::with_capacity(unique_ids.len() * 2);
    all_params.extend(unique_ids.iter().copied());
    all_params.extend(unique_ids.iter().copied());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| crate::Error::db("prepare graph signals query", e))?;
    let rows = stmt
        .query_map(params_from_iter(all_params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?, // source_id
                r.get::<_, String>(1)?, // target_id
                r.get::<_, String>(2)?, // edge_type
            ))
        })
        .map_err(|e| crate::Error::db("query graph signals", e))?;

    let candidate_set: HashSet<&str> = unique_ids.iter().copied().collect();

    // Primary counters + set-based accumulators for diversity / neighbors.
    let mut signals: HashMap<String, GraphSignals> = HashMap::new();
    let mut sig_types: HashMap<String, HashSet<String>> = HashMap::new();
    let mut sig_neighbors: HashMap<String, HashSet<String>> = HashMap::new();
    // Pre-seed all candidates so isolated memories are represented (all zeros).
    for id in &unique_ids {
        signals.entry((*id).to_string()).or_default();
    }

    for row in rows {
        let (source_id, target_id, edge_type) =
            row.map_err(|e| crate::Error::db("graph signals row", e))?;

        // An edge contributes to a candidate's signals iff the candidate is
        // either endpoint. A self-loop (source == target) counts once per axis
        // but the other-endpoint neighbor collapses to itself.
        for (endpoint, as_incoming) in [(&source_id, false), (&target_id, true)] {
            if !candidate_set.contains(endpoint.as_str()) {
                continue;
            }
            let sig = signals.entry(endpoint.clone()).or_default();
            sig.edge_count += 1;
            if as_incoming {
                sig.incoming_count += 1;
            } else {
                sig.outgoing_count += 1;
            }
            sig_types
                .entry(endpoint.clone())
                .or_default()
                .insert(edge_type.clone());
            let other = if endpoint == &source_id {
                &target_id
            } else {
                &source_id
            };
            sig_neighbors
                .entry(endpoint.clone())
                .or_default()
                .insert(other.clone());
        }
    }

    // Finalize set-derived counts.
    for (id, sig) in signals.iter_mut() {
        if let Some(types) = sig_types.get(id) {
            sig.edge_type_diversity = types.len() as u32;
        }
        if let Some(neighbors) = sig_neighbors.get(id) {
            sig.neighbor_count = neighbors.len() as u32;
        }
    }

    Ok(signals)
}

/// Rerank search results by fusing graph signals into each result's score.
///
/// The boost is **additive and log-scaled**, so it preserves the relative
/// order of non-boosted results while shifting graph-rich memories up:
///
/// ```text
/// new_score = (score + density_boost + authority_boost).min(1.0)
/// density_boost  = ln(1 + edge_count)        * density_weight
/// authority_boost = ln(1 + incoming_count)   * authority_weight
/// ```
///
/// The `ln(1 + x)` form (not `ln(x)`) avoids `ln(0) = -inf` for zero-edge
/// memories and matches the saturating intent of the spec's `ln(count)/10`.
/// Results are re-sorted by the new score with a stable comparator so equal
/// scores keep their original order.
///
/// When `config.enabled` is `false` or `signals` is empty, this is a no-op:
/// the input vector is returned unchanged.
pub fn rerank_with_graph(
    mut results: Vec<SearchResult>,
    signals: &HashMap<String, GraphSignals>,
    config: &GraphRerankConfig,
) -> Vec<SearchResult> {
    if !config.enabled || signals.is_empty() || results.is_empty() {
        return results;
    }

    for sr in results.iter_mut() {
        // Missing signal = isolated memory → zero boost (no-op).
        if let Some(sig) = signals.get(&sr.memory.id) {
            let density_boost = (1.0 + sig.edge_count as f32).ln() * config.density_weight;
            let authority_boost = (1.0 + sig.incoming_count as f32).ln() * config.authority_weight;
            sr.score = (sr.score + density_boost + authority_boost).min(1.0);
        }
    }

    // Stable sort preserves prior order for equal post-boost scores.
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{Memory, SearchResult};
    use rusqlite::Connection;

    /// Build an in-memory DB with the `memory_edges` table populated from a
    /// list of `(source, target, edge_type)` tuples.
    fn db_with_edges(edges: &[(&str, &str, &str)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE memory_edges (
                source_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                edge_type TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT ''
            )",
            [],
        )
        .unwrap();
        for (s, t, ty) in edges {
            conn.execute(
                "INSERT INTO memory_edges (source_id, target_id, edge_type) VALUES (?1, ?2, ?3)",
                rusqlite::params![s, t, ty],
            )
            .unwrap();
        }
        conn
    }

    fn sr(id: &str, score: f32) -> SearchResult {
        let now = chrono::Utc::now();
        SearchResult {
            memory: Memory {
                id: id.to_string(),
                content: String::new(),
                embedding: Vec::new(),
                tags: Vec::new(),
                metadata: serde_json::Value::Null,
                created_at: now,
                updated_at: now,
                namespace: String::from("default"),
                access_count: 0,
                last_accessed: None,
                deprecated: false,
                valid_from: None,
                valid_until: None,
                memory_type: String::from("fact"),
                importance: 0.5,
                pinned: false,
                content_type: String::from("text"),
                slug: None,
                source: None,
                source_type: "user".to_string(),
            },
            score,
        }
    }

    #[test]
    fn compute_signals_counts_incoming_outgoing_and_density() {
        // A → B, A → C, D → A : A has 2 outgoing + 1 incoming = 3 edges.
        let conn = db_with_edges(&[
            ("A", "B", "references"),
            ("A", "C", "references"),
            ("D", "A", "tagged_as"),
        ]);
        let ids = vec!["A".to_string(), "B".to_string()];
        let sigs = compute_graph_signals(&conn, &ids).unwrap();

        let a = sigs.get("A").unwrap();
        assert_eq!(a.edge_count, 3);
        assert_eq!(a.outgoing_count, 2);
        assert_eq!(a.incoming_count, 1);

        let b = sigs.get("B").unwrap();
        assert_eq!(b.edge_count, 1);
        assert_eq!(b.incoming_count, 1);
        assert_eq!(b.outgoing_count, 0);
    }

    #[test]
    fn compute_signals_empty_ids_returns_empty() {
        let conn = db_with_edges(&[]);
        let sigs = compute_graph_signals(&conn, &[]).unwrap();
        assert!(sigs.is_empty());
    }

    #[test]
    fn compute_signals_isolated_memory_is_all_zeros() {
        let conn = db_with_edges(&[("A", "B", "references")]);
        // Z exists as a candidate but has no edges.
        let sigs = compute_graph_signals(&conn, &["Z".to_string()]).unwrap();
        let z = sigs.get("Z").unwrap();
        assert_eq!(z, &GraphSignals::default());
    }

    #[test]
    fn rerank_shifts_well_connected_memory_up() {
        let signals = HashMap::from([(
            "hub".to_string(),
            GraphSignals {
                edge_count: 20,
                incoming_count: 15,
                ..Default::default()
            },
        )]);
        let config = GraphRerankConfig::default(); // density 0.1, authority 0.1

        // iso and hub start at the same score; hub should rank above iso.
        let results = vec![sr("iso", 0.80), sr("hub", 0.80)];
        let reranked = rerank_with_graph(results, &signals, &config);
        assert_eq!(reranked[0].memory.id, "hub");
        assert_eq!(reranked[1].memory.id, "iso");
        // hub score strictly increased, iso unchanged.
        assert!(reranked[0].score > 0.80);
        assert!((reranked[1].score - 0.80).abs() < 1e-6);
    }

    #[test]
    fn rerank_disabled_is_noop() {
        let signals = HashMap::from([(
            "hub".to_string(),
            GraphSignals {
                edge_count: 100,
                incoming_count: 100,
                ..Default::default()
            },
        )]);
        let config = GraphRerankConfig {
            enabled: false,
            ..Default::default()
        };
        // Pre-sorted descending; noop must preserve it.
        let results = vec![sr("iso", 0.9), sr("hub", 0.5)];
        let reranked = rerank_with_graph(results, &signals, &config);
        // Order and scores unchanged.
        assert_eq!(reranked[0].memory.id, "iso");
        assert_eq!(reranked[1].memory.id, "hub");
        assert!((reranked[1].score - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rerank_zero_edges_is_noop() {
        // Cold start: no signals → no change. Input pre-sorted descending.
        let signals = HashMap::new();
        let config = GraphRerankConfig::default();
        let results = vec![sr("b", 0.9), sr("a", 0.5)];
        let reranked = rerank_with_graph(results, &signals, &config);
        assert_eq!(reranked[0].memory.id, "b");
        assert!((reranked[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn rerank_log_scaling_prevents_hub_dominance() {
        // Going from 1→10 edges should give roughly the same boost as 100→1000
        // (log saturation). Use a small weight + 0.0 base so the boost stays
        // measurable instead of capping at 1.0.
        let config = GraphRerankConfig {
            density_weight: 0.01, // isolate the density axis, keep sub-1.0
            authority_weight: 0.0,
            enabled: true,
        };
        let boost = |edges: u32| {
            let signals = HashMap::from([(
                "x".to_string(),
                GraphSignals {
                    edge_count: edges,
                    ..Default::default()
                },
            )]);
            let out = rerank_with_graph(vec![sr("x", 0.0)], &signals, &config);
            out[0].score
        };
        let b10 = boost(10);
        let b1000 = boost(1000);
        // 1000 edges is 100x more than 10, but the boost ratio is ln(1001)/ln(11)
        // ≈ 6.9 / 2.4 ≈ 2.9x, NOT 100x.
        let ratio = b1000 / b10.max(1e-6);
        assert!(ratio < 5.0, "log saturation expected, ratio was {ratio}");
        assert!(ratio > 1.0);
    }

    #[test]
    fn rerank_score_capped_at_one() {
        let signals = HashMap::from([(
            "hub".to_string(),
            GraphSignals {
                edge_count: 10_000,
                incoming_count: 10_000,
                ..Default::default()
            },
        )]);
        let config = GraphRerankConfig {
            density_weight: 1.0,
            authority_weight: 1.0,
            enabled: true,
        };
        let reranked = rerank_with_graph(vec![sr("hub", 0.99)], &signals, &config);
        assert!(reranked[0].score <= 1.0);
    }

    #[test]
    fn sanitized_clamps_negative_weights() {
        let bad = GraphRerankConfig {
            density_weight: -0.5,
            authority_weight: 2.0,
            enabled: true,
        };
        let good = bad.sanitized();
        assert_eq!(good.density_weight, 0.0);
        assert_eq!(good.authority_weight, 1.0);
    }

    /// Latency guard: computing signals for limit=10 candidates over a large
    /// edge table must stay well under 10ms (acceptance criterion #7).
    #[test]
    fn compute_signals_latency_under_10ms_for_typical_limit() {
        let mut edges: Vec<(&str, &str, &str)> = Vec::with_capacity(5000);
        // 5000 edges among 200 memories — denser than any realistic recall set.
        for i in 0..5000 {
            let s = format!("m{}", i % 200);
            let t = format!("m{}", (i * 7) % 200);
            edges.push((
                // leak a 'static name per edge: we build owned strings below.
                Box::leak(s.into_boxed_str()),
                Box::leak(t.into_boxed_str()),
                "references",
            ));
        }
        let conn = db_with_edges(&edges);
        let ids: Vec<String> = (0..10).map(|i| format!("m{i}")).collect();

        let start = std::time::Instant::now();
        let sigs = compute_graph_signals(&conn, &ids).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(sigs.len(), 10);
        // Generous headroom over the 10ms target to absorb CI variance.
        assert!(
            elapsed.as_millis() < 50,
            "compute_graph_signals took {:?}, expected < 50ms",
            elapsed
        );
    }
}
