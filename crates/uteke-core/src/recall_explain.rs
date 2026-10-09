//! Recall explanation / debug mode (#1160).
//!
//! `Uteke::recall_explained` runs the SAME building blocks as the normal
//! recall path (vector channel, FTS5 channel, weighted-RRF, Jaccard boost,
//! salience/recency boosts, graph rerank) but instruments every stage, so
//! each returned result carries the signals that placed it.
//!
//! Deliberate trade-offs (documented in the issue):
//! - Cold compute only — the recall cache is bypassed so the explanation
//!   always describes the results actually returned.
//! - One extra query embedding (~50ms) + one extra FTS5 query vs a normal
//!   cold call; no additional index searches or model loads.

use std::collections::HashMap;

use serde::Serialize;

use crate::Error;
use crate::Uteke;
use crate::memory::types::{Memory, RecallStrategy, SearchResult};
use crate::operations::{FUSION_RRF_K, FUSION_W_HYBRID, FUSION_W_VECTOR};

/// Per-result ranking signal breakdown (#1160).
#[derive(Debug, Clone, Serialize)]
pub struct RecallExplanation {
    /// Strategy that produced this result.
    pub strategy: String,
    /// The final score the result ranked on (post-boosts).
    pub final_score: f32,
    /// Score before salience/recency boosts (after RRF/jaccard/graph).
    pub base_score: f32,
    /// Raw cosine similarity between the query embedding and the memory
    /// embedding. `None` when the memory has no embedding.
    pub vector_similarity: Option<f32>,
    /// 1-based rank in the vector channel ranking. `None` when the memory
    /// was outside the vector channel's candidate window.
    pub vector_rank: Option<usize>,
    /// 1-based rank in the FTS5 channel ranking. `None` when absent.
    pub fts_rank: Option<usize>,
    /// Normalized RRF score (before jaccard/boosts) — hybrid and fusion.
    pub rrf_score: Option<f32>,
    /// Fusion only: weighted RRF contribution of the vector channel.
    pub fusion_vector_contribution: Option<f32>,
    /// Fusion only: weighted RRF contribution of the hybrid channel.
    pub fusion_hybrid_contribution: Option<f32>,
    /// Jaccard token-overlap boost added to the base score (hybrid/graph).
    pub jaccard_boost: Option<f32>,
    /// Salience boost delta included in the final score.
    pub salience_boost: Option<f32>,
    /// Recency boost delta included in the final score.
    pub recency_boost: Option<f32>,
    /// Graph-strategy only: total graph rerank delta.
    pub graph_boost: Option<f32>,
}

/// A recall result with its full ranking explanation (#1160).
///
/// Serialized with `result` flattened so JSON consumers see the familiar
/// `{memory, score}` shape plus an additive `explanation` object.
#[derive(Debug, Clone, Serialize)]
pub struct ExplainedRecall {
    #[serde(flatten)]
    pub result: SearchResult,
    pub explanation: RecallExplanation,
}

/// RRF rank→score contribution: `weight / (k + rank)` with a 1-based rank.
fn rrf_contrib(weight: f64, rank: usize) -> f64 {
    weight / (FUSION_RRF_K + rank as f64)
}

/// Raw cosine similarity between two vectors (0.0 for zero-norm inputs).
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na <= 0.0 || nb <= 0.0 {
        0.0
    } else {
        (dot / (na * nb)).clamp(0.0, 1.0)
    }
}

/// Rank of each memory id in a channel ranking (1-based, first occurrence).
fn rank_map<'a, I>(items: I) -> HashMap<String, usize>
where
    I: Iterator<Item = &'a SearchResult>,
{
    items
        .enumerate()
        .map(|(i, sr)| (sr.memory.id.clone(), i + 1))
        .collect()
}

impl Uteke {
    /// Recall with a per-result ranking explanation (#1160).
    ///
    /// Mirrors `recall_hybrid` stage-for-stage (same channel depths, same
    /// RRF constants, same boost order) but bypasses the recall cache so the
    /// explanation always matches the returned results.
    pub fn recall_explained(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        strategy: RecallStrategy,
        min_score: f32,
    ) -> Result<Vec<ExplainedRecall>, Error> {
        // Fts5 needs no query embedding — keep it usable without an embedder
        // (CI-safe, same contract as the fts5 strategy itself). All other
        // strategies embed the query once, exactly like the real path.
        let query_embedding: Option<Vec<f32>> = if strategy == RecallStrategy::Fts5 {
            None
        } else {
            self.ensure_embedder()?;
            Some(
                self.embedder
                    .lock()
                    .map_err(|_| Error::lock("embedder lock during recall_explained"))?
                    .as_ref()
                    .ok_or_else(|| Error::embed_msg("no embedder configured"))?
                    .embed(query)?,
            )
        };

        let boost_window = limit.saturating_mul(4).saturating_add(16);

        let strategy_name = match strategy {
            RecallStrategy::Vector => "vector",
            RecallStrategy::Fts5 => "fts5",
            RecallStrategy::Hybrid => "hybrid",
            RecallStrategy::Graph => "graph",
            RecallStrategy::Fusion => "fusion",
        };

        // Cosine similarity helper over the query embedding (None when the
        // strategy skips embedding or the memory has no vector).
        let vec_sim_of = |m: &Memory| -> Option<f32> {
            let qe = query_embedding.as_ref()?;
            if m.embedding.is_empty() {
                return None;
            }
            Some(cosine_similarity(qe, &m.embedding))
        };

        /// Intermediate signals captured while the strategy pipeline is
        /// reconstructed from the same building blocks.
        struct Sig {
            base: f32,
            vector_similarity: Option<f32>,
            vector_rank: Option<usize>,
            fts_rank: Option<usize>,
            rrf_score: Option<f32>,
            fusion_vector_contribution: Option<f32>,
            fusion_hybrid_contribution: Option<f32>,
            jaccard_boost: Option<f32>,
            graph_boost: Option<f32>,
        }
        let empty_sig = |base: f32| Sig {
            base,
            vector_similarity: None,
            vector_rank: None,
            fts_rank: None,
            rrf_score: None,
            fusion_vector_contribution: None,
            fusion_hybrid_contribution: None,
            jaccard_boost: None,
            graph_boost: None,
        };

        // (ordered results with raw base scores, signals per id)
        let (mut results, mut signals): (Vec<SearchResult>, HashMap<String, Sig>) = match strategy {
            RecallStrategy::Vector => {
                // The real vector arm via compute_recall (same entry point
                // the dispatcher uses — keeps the cache/boost contract of
                // this function consistent across strategies).
                let results = self.compute_recall(
                    RecallStrategy::Vector,
                    query,
                    limit,
                    tags_filter,
                    namespace,
                    0.0,
                )?;
                let ranks = rank_map(results.iter());
                let mut sigs: HashMap<String, Sig> = HashMap::new();
                for sr in &results {
                    let mut s = empty_sig(sr.score);
                    s.vector_similarity = vec_sim_of(&sr.memory);
                    s.vector_rank = ranks.get(&sr.memory.id).copied();
                    sigs.insert(sr.memory.id.clone(), s);
                }
                (results, sigs)
            }
            RecallStrategy::Fts5 => {
                // The real fts5 arm (compute_recall::Fts5 → recall_fts5_only).
                let results = self.compute_recall(
                    RecallStrategy::Fts5,
                    query,
                    limit,
                    tags_filter,
                    namespace,
                    0.0,
                )?;
                let mut sigs: HashMap<String, Sig> = HashMap::new();
                for (i, sr) in results.iter().enumerate() {
                    let mut s = empty_sig(sr.score);
                    s.vector_similarity = vec_sim_of(&sr.memory);
                    s.fts_rank = Some(i + 1);
                    sigs.insert(sr.memory.id.clone(), s);
                }
                (results, sigs)
            }
            RecallStrategy::Hybrid | RecallStrategy::Graph => {
                // recall_rrf(window): sub-channels run at window*3 depth.
                let depth = boost_window.saturating_mul(3);
                let vec_list = self.compute_recall(
                    RecallStrategy::Vector,
                    query,
                    depth,
                    tags_filter,
                    namespace,
                    0.0,
                )?;
                let fts_list: Vec<(Memory, f64)> = {
                    let fts = match self.store.search_fts5(query, namespace, depth) {
                        Ok(r) if !r.is_empty() => r,
                        Ok(_) => self.store.search_fts5_tokens(query, namespace, depth)?,
                        Err(e) => return Err(e),
                    };
                    fts.into_iter()
                        .filter(|(memory, _)| {
                            if let Some(ns) = namespace {
                                if memory.namespace != ns {
                                    return false;
                                }
                            }
                            if let Some(filter_tags) = tags_filter {
                                if !filter_tags
                                    .iter()
                                    .any(|ft| memory.tags.iter().any(|t| t == ft))
                                {
                                    return false;
                                }
                            }
                            true
                        })
                        .collect()
                };

                let vec_rank: HashMap<String, usize> = vec_list
                    .iter()
                    .enumerate()
                    .map(|(i, sr)| (sr.memory.id.clone(), i + 1))
                    .collect();
                let fts_rank: HashMap<String, usize> = fts_list
                    .iter()
                    .enumerate()
                    .map(|(i, (m, _))| (m.id.clone(), i + 1))
                    .collect();

                // RRF merge — identical math to recall_rrf (k = 60).
                let max_rrf = 2.0 / (FUSION_RRF_K + 1.0);
                let mut rrf_scores: HashMap<String, f64> = HashMap::new();
                for (id, r) in &vec_rank {
                    *rrf_scores.entry(id.clone()).or_default() += 1.0 / (FUSION_RRF_K + *r as f64);
                }
                for (id, r) in &fts_rank {
                    *rrf_scores.entry(id.clone()).or_default() += 1.0 / (FUSION_RRF_K + *r as f64);
                }
                let mut scored: Vec<(String, f64)> = rrf_scores.into_iter().collect();
                scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                // Payloads from the channel lists (first occurrence wins,
                // vector channel first — same order as recall_rrf).
                let mut payloads: HashMap<String, Memory> = HashMap::new();
                for sr in &vec_list {
                    payloads
                        .entry(sr.memory.id.clone())
                        .or_insert_with(|| sr.memory.clone());
                }
                for (m, _) in &fts_list {
                    payloads.entry(m.id.clone()).or_insert_with(|| m.clone());
                }

                // take(boost_window) — matches recall_rrf's truncation.
                let ordered: Vec<(String, f64)> = scored.into_iter().take(boost_window).collect();
                // Jaccard boost — identical math to recall_rrf (#719).
                let jaccard_of = |m: &Memory| -> Option<f32> {
                    if self.jaccard_weight <= 0.0 {
                        return None;
                    }
                    let qt = crate::jaccard::tokenize(query);
                    if qt.is_empty() {
                        return None;
                    }
                    let mut ct = crate::jaccard::tokenize(&m.content);
                    for tag in &m.tags {
                        ct.insert(tag.to_ascii_lowercase());
                    }
                    Some(crate::jaccard::jaccard_similarity(&qt, &ct) * self.jaccard_weight)
                };

                let mut results: Vec<SearchResult> = Vec::with_capacity(ordered.len());
                let mut sigs: HashMap<String, Sig> = HashMap::new();
                for (id, raw_rrf) in &ordered {
                    let memory = match payloads.get(id) {
                        Some(m) => m.clone(),
                        None => continue,
                    };
                    let normalized = ((*raw_rrf / max_rrf).clamp(0.0, 1.0)) as f32;
                    let jb = jaccard_of(&memory);
                    let base = normalized + jb.unwrap_or(0.0);
                    let mut s = empty_sig(base);
                    s.vector_similarity = vec_sim_of(&memory);
                    s.vector_rank = vec_rank.get(id).copied();
                    s.fts_rank = fts_rank.get(id).copied();
                    s.rrf_score = Some(normalized);
                    s.jaccard_boost = jb;
                    sigs.insert(id.clone(), s);
                    results.push(SearchResult {
                        memory,
                        score: base,
                    });
                }
                results.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                // Graph arm: hybrid RRF + graph-signal rerank delta (#378).
                if strategy == RecallStrategy::Graph
                    && self.graph_rerank_config.enabled
                    && !results.is_empty()
                {
                    let ids: Vec<String> = results.iter().map(|r| r.memory.id.clone()).collect();
                    let g_signals =
                        crate::graph_rerank::compute_graph_signals(&self.store.conn, &ids)?;
                    let before: HashMap<String, f32> = results
                        .iter()
                        .map(|r| (r.memory.id.clone(), r.score))
                        .collect();
                    results = crate::graph_rerank::rerank_with_graph(
                        results,
                        &g_signals,
                        &self.graph_rerank_config,
                    );
                    for r in &results {
                        if let Some(s) = sigs.get_mut(&r.memory.id) {
                            let prev = before.get(&r.memory.id).copied().unwrap_or(r.score);
                            s.graph_boost = Some(r.score - prev);
                            s.base = r.score;
                        }
                    }
                }
                (results, sigs)
            }
            RecallStrategy::Fusion => {
                // Fusion (#1123): two sub-rankings at window depth, then
                // weighted RRF — the exact calls compute_recall makes.
                let vec_res = self.compute_recall(
                    RecallStrategy::Vector,
                    query,
                    boost_window,
                    tags_filter,
                    namespace,
                    0.0,
                )?;
                let hyb_res = self.compute_recall(
                    RecallStrategy::Hybrid,
                    query,
                    boost_window,
                    tags_filter,
                    namespace,
                    0.0,
                )?;

                let vec_rank = rank_map(vec_res.iter());
                let hyb_rank = rank_map(hyb_res.iter());

                let fused = crate::operations::rrf_fuse_weighted(
                    vec_res.clone(),
                    hyb_res.clone(),
                    FUSION_W_VECTOR,
                    FUSION_W_HYBRID,
                );

                let mut sigs: HashMap<String, Sig> = HashMap::new();
                for sr in &fused {
                    let id = &sr.memory.id;
                    let vr = vec_rank.get(id).copied();
                    let hr = hyb_rank.get(id).copied();
                    let mut s = empty_sig(sr.score);
                    s.vector_similarity = vec_sim_of(&sr.memory);
                    s.vector_rank = vr;
                    s.fts_rank = None; // hybrid channel exposes no FTS ranks
                    s.rrf_score = Some(sr.score);
                    s.fusion_vector_contribution =
                        vr.map(|r| rrf_contrib(FUSION_W_VECTOR, r) as f32);
                    s.fusion_hybrid_contribution =
                        hr.map(|r| rrf_contrib(FUSION_W_HYBRID, r) as f32);
                    sigs.insert(id.clone(), s);
                }
                (fused, sigs)
            }
        };

        // Salience/recency boosts — identical math to
        // apply_salience_recency_boosts, with per-axis delta capture.
        let cfg = self.salience_recency_config;
        if !cfg.is_noop() {
            let now = chrono::Utc::now();
            for sr in results.iter_mut() {
                sr.score = crate::salience_recency::apply_boosts(sr.score, &sr.memory, now, cfg);
            }
        }
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        if min_score > 0.0 {
            results.retain(|r| r.score >= min_score);
        }

        // Assemble explained results; boost deltas computed per axis.
        let now = chrono::Utc::now();
        let mut out = Vec::with_capacity(results.len());
        for sr in results {
            let sig = signals.remove(&sr.memory.id);
            let (sal, rec) = if cfg.is_noop() {
                (None, None)
            } else {
                (
                    Some(crate::salience_recency::salience_score(&sr.memory) * cfg.salience_weight),
                    Some(
                        crate::salience_recency::recency_score(&sr.memory, now)
                            * cfg.recency_weight,
                    ),
                )
            };
            let explanation = match sig {
                Some(s) => RecallExplanation {
                    strategy: strategy_name.to_string(),
                    final_score: sr.score,
                    base_score: s.base,
                    vector_similarity: s.vector_similarity,
                    vector_rank: s.vector_rank,
                    fts_rank: s.fts_rank,
                    rrf_score: s.rrf_score,
                    fusion_vector_contribution: s.fusion_vector_contribution,
                    fusion_hybrid_contribution: s.fusion_hybrid_contribution,
                    jaccard_boost: s.jaccard_boost,
                    salience_boost: sal,
                    recency_boost: rec,
                    graph_boost: s.graph_boost,
                },
                None => RecallExplanation {
                    strategy: strategy_name.to_string(),
                    final_score: sr.score,
                    base_score: sr.score,
                    vector_similarity: vec_sim_of(&sr.memory),
                    vector_rank: None,
                    fts_rank: None,
                    rrf_score: None,
                    fusion_vector_contribution: None,
                    fusion_hybrid_contribution: None,
                    jaccard_boost: None,
                    salience_boost: sal,
                    recency_boost: rec,
                    graph_boost: None,
                },
            };
            out.push(ExplainedRecall {
                result: sr,
                explanation,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod explain_tests {
    use crate::Uteke;
    use crate::memory::types::RecallStrategy;

    fn scratch() -> (Uteke, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let uteke = Uteke::open(dir.path().join("t.db")).unwrap();
        (uteke, dir)
    }

    fn seed_fts(uteke: &Uteke) {
        let now = chrono::Utc::now();
        let mk = |id: &str, content: &str| crate::memory::types::Memory {
            id: id.to_string(),
            content: content.to_string(),
            embedding: vec![0.0; 768],
            tags: vec![],
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
            namespace: "explain-ns".to_string(),
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
            source_type: "user".to_string(),
            author_type: "agent".to_string(),
        };
        uteke
            .store
            .insert(&mk(
                "explain-fts-target",
                "The quick brown fox jumps over the lazy dog",
            ))
            .unwrap();
        uteke
            .store
            .insert(&mk(
                "explain-fts-noise",
                "Completely unrelated content about gardening tools",
            ))
            .unwrap();
    }

    /// #1160: explained recall matches the real fts5 arm — same memory, and
    /// the explanation carries the fts rank signal. Runs without an embedder.
    #[test]
    fn explain_fts5_matches_real_arm() {
        let (uteke, dir) = scratch();
        seed_fts(&uteke);

        let plain = uteke
            .recall_hybrid(
                "quick brown fox",
                5,
                None,
                Some("explain-ns"),
                RecallStrategy::Fts5,
                0.0,
            )
            .unwrap();
        let explained = uteke
            .recall_explained(
                "quick brown fox",
                5,
                None,
                Some("explain-ns"),
                RecallStrategy::Fts5,
                0.0,
            )
            .unwrap();

        assert!(!plain.is_empty(), "plain fts5 must find the fox");
        assert_eq!(plain[0].memory.id, "explain-fts-target");
        assert_eq!(explained.len(), plain.len(), "same result count");
        assert_eq!(explained[0].result.memory.id, plain[0].memory.id);
        assert_eq!(
            explained[0].result.memory.id, "explain-fts-target",
            "explained fts5 must find the fox too"
        );

        let e = &explained[0].explanation;
        assert_eq!(e.strategy, "fts5");
        assert_eq!(
            e.fts_rank,
            Some(1),
            "target must be rank 1 in the fts channel"
        );
        assert!(
            (e.final_score - plain[0].score).abs() < 1e-6,
            "explained score must equal the plain score: {} vs {}",
            e.final_score,
            plain[0].score
        );

        // Noise must not appear above the target.
        assert!(
            !explained
                .iter()
                .take(1)
                .any(|r| r.result.memory.id == "explain-fts-noise")
        );
        drop(uteke);
        drop(dir);
    }

    /// #1160 (requires ONNX model — ignored in CI, run locally): the
    /// explanation for the default fusion strategy must carry vector ranks,
    /// fusion contributions, and reproduce the plain recall scores.
    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn explain_fusion_reproduces_plain_recall() {
        let (mut uteke, dir) = scratch();
        // Real embeddings (remember → model embed) so vector_similarity is a
        // genuine cosine; synthetic fixtures would make it arbitrary.
        uteke
            .remember(
                "quarterly revenue projections for the board meeting",
                &["finance"],
                None,
                Some("explain-ns"),
            )
            .unwrap();
        for i in 0..3 {
            uteke
                .remember(
                    &format!("unrelated filler note {i} about gardening"),
                    &["misc"],
                    None,
                    Some("explain-ns"),
                )
                .unwrap();
        }

        let query = "quarterly revenue projections";

        // Boosts disabled → explained must EXACTLY reproduce the plain call.
        // (With boosts on, touch_access side effects legitimately make
        // salience drift between separate pipeline runs.)
        uteke.set_salience_recency_config(crate::salience_recency::SalienceRecencyConfig {
            salience_weight: 0.0,
            recency_weight: 0.0,
        });

        let plain = uteke
            .recall_hybrid(
                query,
                3,
                None,
                Some("explain-ns"),
                RecallStrategy::Fusion,
                0.0,
            )
            .unwrap();
        let explained = uteke
            .recall_explained(
                query,
                3,
                None,
                Some("explain-ns"),
                RecallStrategy::Fusion,
                0.0,
            )
            .unwrap();

        assert!(!plain.is_empty());
        assert_eq!(plain.len(), explained.len());
        assert_eq!(
            plain[0].memory.id, explained[0].result.memory.id,
            "explained fusion must keep the same top-1 as plain fusion"
        );
        assert!(
            plain[0].memory.content.contains("quarterly revenue"),
            "on-topic memory must rank first: {}",
            plain[0].memory.content
        );

        for (p, e) in plain.iter().zip(explained.iter()) {
            assert_eq!(p.memory.id, e.result.memory.id, "same ranking order");
            assert!(
                (p.score - e.result.score).abs() < 1e-6,
                "explained score must equal plain score: {} vs {}",
                e.result.score,
                p.score
            );
        }

        let e = &explained[0].explanation;
        assert_eq!(e.strategy, "fusion");
        assert!(
            e.vector_similarity.unwrap_or(0.0) > 0.3,
            "on-topic memory must have high vector similarity: {e:?}"
        );
        assert_eq!(e.vector_rank, Some(1), "on-topic must be vector rank 1");
        let vc = e.fusion_vector_contribution.expect("vector contribution");
        assert!(vc > 0.0, "fusion contribution must be positive");
        // Contribution math: 1.7 / (60 + 1) for rank 1.
        assert!(
            (vc - 1.7 / 61.0).abs() < 1e-6,
            "vector contribution for rank 1 must be 1.7/61: {vc}"
        );

        // ── Phase 2: default boosts on → explanation must be internally
        // consistent: final == base + salience + recency.
        uteke.set_salience_recency_config(crate::salience_recency::SalienceRecencyConfig {
            salience_weight: 0.1,
            recency_weight: 0.1,
        });
        let explained_boosted = uteke
            .recall_explained(
                query,
                3,
                None,
                Some("explain-ns"),
                RecallStrategy::Fusion,
                0.0,
            )
            .unwrap();
        let eb = &explained_boosted[0].explanation;
        let expected =
            eb.base_score + eb.salience_boost.unwrap_or(0.0) + eb.recency_boost.unwrap_or(0.0);
        assert!(
            (eb.final_score - expected).abs() < 1e-4,
            "final must equal base + boosts: {} vs {}",
            eb.final_score,
            expected
        );
        let sal = eb.salience_boost.expect("salience delta");
        let rec = eb.recency_boost.expect("recency delta");
        assert!(sal > 0.0, "fresh important memory must gain salience");
        assert!(
            (rec - 0.1).abs() < 0.02,
            "brand-new memory must gain ~full 0.1 recency: {rec}"
        );

        drop(uteke);
        drop(dir);
    }
}
