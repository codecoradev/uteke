//! Recall and search: vector, hybrid, FTS5 and fusion strategies, boosts, the context recall.

use super::*;

impl crate::Uteke {
    #[allow(clippy::too_many_arguments)]
    /// Recall memories relevant to a query using vector similarity.
    ///
    /// Optionally filter by tags and namespace.
    ///
    /// Embedding computation is performed outside the index lock to avoid
    /// blocking concurrent reads (RwLock allows shared read access).
    pub fn recall(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        min_score: f32,
        entity_filter: Option<&str>,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, Error> {
        let results = self.recall_inner(
            query,
            limit,
            tags_filter,
            namespace,
            min_score,
            entity_filter,
            category_filter,
            false,
        )?;
        self.touch_recalled(&results);
        Ok(results)
    }

    /// Count the results of ONE user-level recall/search as recall hits
    /// (#1337, #1407). The strategy arms never touch on their own: they are
    /// built from each other (fusion runs vector + hybrid, hybrid runs vector),
    /// and the arms work on a `boost_window` of candidates, so touching inside
    /// them counted one recall several times and bumped memories that were
    /// never returned.
    fn touch_recalled(&self, results: &[SearchResult]) {
        let ids: Vec<&str> = results.iter().map(|r| r.memory.id.as_str()).collect();
        self.store.touch_recall_batch(&ids).ok();
    }

    /// Same as `recall` but optionally keeps deprecated memories so temporal
    /// filters (#1086) can decide their fate based on `deprecated_at`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn recall_inner(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        min_score: f32,
        entity_filter: Option<&str>,
        category_filter: Option<&str>,
        include_deprecated: bool,
    ) -> Result<Vec<SearchResult>, Error> {
        // Embed query outside any lock — CPU-intensive (~50ms), no shared state needed.
        // Only the embedder Mutex is held here, allowing concurrent index reads.
        let query_embedding = self.embed(query)?;
        // Embedder lock dropped here — other threads can embed or recall concurrently.

        // Search usearch index with retry: if post-filtering removes too many
        // results, increase k and try again (up to 3 attempts).
        // RwLock read lock — multiple concurrent recalls can search simultaneously.
        let index = self
            .index
            .read()
            .map_err(|_| Error::lock("index read lock during recall"))?;
        let index_len = index.len();
        let mut results = Vec::new();
        let mut attempt = 0;
        let mut multiplier = 3usize;

        while results.len() < limit && attempt < 3 {
            let k = (limit * multiplier).min(index_len).max(1);
            let ef = (limit * multiplier * 4).max(50);
            let candidates = index.search(&query_embedding, k, ef);

            results.clear();

            // Batch-fetch all candidate memories in one query (eliminates N+1).
            let candidate_ids: Vec<&str> = candidates.iter().map(|(id, _)| id.as_str()).collect();
            let fetched = self.store.get_by_ids(&candidate_ids)?;
            let mem_map: std::collections::HashMap<&str, &crate::memory::types::Memory> =
                fetched.iter().map(|m| (m.id.as_str(), m)).collect();

            for (memory_id, distance) in &candidates {
                if results.len() >= limit {
                    break;
                }

                let memory = match mem_map.get(memory_id.as_str()) {
                    Some(m) => m,
                    None => continue,
                };

                // Apply namespace filter (None = search ALL namespaces, #448)
                if let Some(ns) = namespace {
                    if memory.namespace != ns {
                        continue;
                    }
                }

                // Apply tag filter
                if let Some(filter_tags) = tags_filter {
                    let has_tag = filter_tags
                        .iter()
                        .any(|ft| memory.tags.iter().any(|t| t == ft));
                    if !has_tag {
                        continue;
                    }
                }

                // Apply entity metadata filter
                if let Some(ent) = entity_filter {
                    let matches = memory
                        .metadata
                        .get("entity")
                        .and_then(|v| v.as_str())
                        .is_some_and(|e| e == ent);
                    if !matches {
                        continue;
                    }
                }

                // Apply category metadata filter
                if let Some(cat) = category_filter {
                    let matches = memory
                        .metadata
                        .get("category")
                        .and_then(|v| v.as_str())
                        .is_some_and(|c| c == cat);
                    if !matches {
                        continue;
                    }
                }

                // Filter deprecated memories (#748) — unless the caller
                // needs them for point-in-time filtering (#1086).
                if memory.deprecated && !include_deprecated {
                    continue;
                }

                let score = cosine_distance_to_similarity(*distance);

                // Boost hot memories (configurable boost)
                let tier = MemoryTier::from_last_accessed(
                    memory.last_accessed,
                    self.tier_config.hot_days,
                    self.tier_config.warm_days,
                );
                let boosted_score = match tier {
                    MemoryTier::Hot => (score + self.tier_config.hot_boost as f32).min(1.0),
                    _ => score,
                };

                results.push(SearchResult {
                    memory: (*memory).clone(),
                    score: boosted_score,
                });
            }

            // If we have enough results or searched the entire index, stop
            if results.len() >= limit || k >= index_len {
                break;
            }

            // Increase search scope for next attempt
            attempt += 1;
            multiplier *= 3;
        }

        // Sort by score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Filter by minimum similarity score
        if min_score > 0.0 {
            results.retain(|r| r.score >= min_score);
        }

        Ok(results)
    }

    /// Recall memories using hybrid search: vector + FTS5 merged via RRF.
    ///
    /// Falls back to vector-only if FTS5 is not available.
    pub fn recall_hybrid(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        strategy: RecallStrategy,
        min_score: f32,
    ) -> Result<Vec<SearchResult>, Error> {
        // Check recall cache first — avoids redundant embedding (~50ms).
        // min_score is NOT in the cache key: cached results store the full set
        // and the caller re-applies threshold, ensuring correctness regardless
        // of what threshold a previous caller used.
        let cache_ns = namespace;
        // Boost window: salience/recency boosts can reorder results across the
        // limit boundary, so we cache a larger candidate set and re-apply
        // boosts + truncate on every read (#1037). Without the window, a
        // memory outside the raw top-N could never enter warm results even
        // though boosts would lift it there on a cold call.
        let boost_window = (limit.saturating_mul(4)).saturating_add(16);

        if let Some(cached) = self
            .recall_cache
            .get(query, cache_ns, limit, tags_filter, strategy)
        {
            let mut results = cached;
            // Cache stores RAW (pre-boost) scores — re-apply salience/recency
            // boosts on every read so warm-cache results match cold-compute
            // results exactly (#1037). Boosts are time-dependent, so applying
            // them at cache-write time would freeze staleness into the cache.
            self.apply_salience_recency_boosts(&mut results, limit);
            if min_score > 0.0 {
                results.retain(|r| r.score >= min_score);
            }
            results.truncate(limit);
            self.touch_recalled(&results);
            return Ok(results);
        }

        // Compute against the boost window so the cached candidate set is
        // large enough for boosts to reorder into the final top-N (#1037).
        // min_score is passed as 0.0 to the underlying paths: thresholding
        // happens AFTER boosts (on both cache-miss and cache-hit reads) so
        // cold and warm calls filter identical boosted score sets.
        let results = self.compute_recall(
            strategy,
            query,
            boost_window,
            tags_filter,
            namespace,
            min_score,
        )?;

        // Cache results for future queries (without min_score filtering,
        // so cached results are reusable for any threshold). The cached set
        // is the boost_window candidate set — truncated to `limit` on every
        // read after boost re-application (#1037).
        //
        // Post-process in place (identical to the cache-hit read path):
        // boost → sort → truncate → min_score. No put-then-get round-trip —
        // a cache eviction or lock failure between put and get must never
        // turn a successful computation into an empty result (cora finding).
        let mut results = results;
        results.truncate(boost_window);
        let raw = results.clone();
        self.recall_cache
            .put(query, cache_ns, limit, tags_filter, strategy, raw);

        self.apply_salience_recency_boosts(&mut results, limit);
        if min_score > 0.0 {
            results.retain(|r| r.score >= min_score);
        }
        results.truncate(limit);
        self.touch_recalled(&results);
        Ok(results)
    }

    /// Raw strategy computation below the cache layer. Shared by the
    /// dispatch path (cache-miss) and by Fusion, which runs two
    /// sub-strategies and fuses their rankings (#1123).
    ///
    /// Callers must NOT cache inside this method — the dispatcher owns the
    /// cache put and applies salience/recency boosts exactly once.
    /// `pub(crate)` for the #1160 explanation path, which replays the same
    /// building blocks stage-by-stage.
    pub(crate) fn compute_recall(
        &self,
        strategy: RecallStrategy,
        query: &str,
        boost_window: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        min_score: f32,
    ) -> Result<Vec<SearchResult>, Error> {
        match strategy {
            RecallStrategy::Vector => self.recall_inner(
                query,
                boost_window,
                tags_filter,
                namespace,
                0.0,
                None,
                None,
                false,
            ),
            RecallStrategy::Fts5 => {
                self.recall_fts5_only(query, boost_window, tags_filter, namespace, 0.0)
            }
            // Hybrid (RRF): min_score is passed but not used for filtering.
            // RRF scores are rank-based, not cosine similarity. Applying a
            // cosine threshold to RRF scores would incorrectly filter results.
            RecallStrategy::Hybrid => {
                self.recall_rrf(query, boost_window, tags_filter, namespace, min_score)
            }
            // Graph (#378): hybrid RRF, then fuse graph-signal boosts.
            // The boost is additive + log-scaled, so isolated memories are
            // untouched and well-connected memories drift upward. Reranking
            // happens *before* caching so cache entries store the final scores.
            RecallStrategy::Graph => {
                let rrf =
                    self.recall_rrf(query, boost_window, tags_filter, namespace, min_score)?;
                if self.graph_rerank_config.enabled && !rrf.is_empty() {
                    let ids: Vec<String> = rrf.iter().map(|r| r.memory.id.clone()).collect();
                    let signals =
                        crate::graph_rerank::compute_graph_signals(&self.store.conn, &ids)?;
                    Ok(crate::graph_rerank::rerank_with_graph(
                        rrf,
                        &signals,
                        &self.graph_rerank_config,
                    ))
                } else {
                    Ok(rrf)
                }
            }
            // Fusion (#1123): run vector and hybrid rankings at boost_window
            // depth, then weighted-RRF fuse them. Vector and hybrid fail on
            // disjoint question sets; the fused ranking captures both sides'
            // wins (fast50 R@5 0.9267 hybrid → 0.98 fusion). Sub-calls bypass
            // the cache/boost layer — THIS dispatcher applies boosts once and
            // caches the fused set, mirroring harness semantics.
            RecallStrategy::Fusion => {
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
                    min_score,
                )?;
                Ok(rrf_fuse_weighted(
                    vec_res,
                    hyb_res,
                    FUSION_W_VECTOR,
                    FUSION_W_HYBRID,
                ))
            }
        }
    }

    /// Apply salience/recency boosts in place, then re-sort and truncate.
    /// Shared by the cache-miss and cache-hit paths of `recall_hybrid` so
    /// both produce identical scores for the same query (#1037).
    fn apply_salience_recency_boosts(&self, results: &mut Vec<SearchResult>, limit: usize) {
        if self.salience_recency_config.is_noop() {
            return;
        }
        let now = chrono::Utc::now();
        for sr in results.iter_mut() {
            sr.score = crate::salience_recency::apply_boosts(
                sr.score,
                &sr.memory,
                now,
                self.salience_recency_config,
            );
        }
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
    }

    /// Recall memories and return a formatted context string for AI prompt injection.
    ///
    /// Returns a compact, structured summary optimized for LLM consumption.
    /// Each memory includes content, score, tags, and importance.
    ///
    /// Example output:
    /// ```text
    /// [Relevant Memories (3 results, 0.82 avg score)]
    /// 1. [0.91] Login timeout bug at 500ms [bug, login]
    /// 2. [0.85] Increase login timeout to 5s [fix]
    /// 3. [0.70] Users report timeout on slow connections [feedback]
    /// ```
    #[deprecated(note = "unused — candidate for removal in future version")]
    #[allow(dead_code)]
    pub fn recall_context(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        strategy: RecallStrategy,
        min_score: f32,
    ) -> Result<String, Error> {
        let results =
            self.recall_hybrid(query, limit, tags_filter, namespace, strategy, min_score)?;

        if results.is_empty() {
            return Ok(format!("[No relevant memories found for: {query}]"));
        }

        let avg_score: f32 = results.iter().map(|r| r.score).sum::<f32>() / results.len() as f32;
        let mut lines = vec![format!(
            "[Relevant Memories ({} results, {:.2} avg score)]",
            results.len(),
            avg_score
        )];

        for (i, sr) in results.iter().enumerate() {
            let tags = if sr.memory.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", sr.memory.tags.join(", "))
            };
            let importance = if sr.memory.pinned {
                " ★".to_string()
            } else if sr.memory.importance > 0.7 {
                " ↑".to_string()
            } else {
                String::new()
            };
            lines.push(format!(
                "{}. [{:.2}] {}{}{}",
                i + 1,
                sr.score,
                sr.memory.content,
                tags,
                importance
            ));
        }

        Ok(lines.join("\n"))
    }

    /// FTS5-only recall.
    fn recall_fts5_only(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        min_score: f32,
    ) -> Result<Vec<SearchResult>, Error> {
        // Try phrase search first, fall back to token search
        let fts_results = match self.store.search_fts5(query, namespace, limit * 3) {
            Ok(r) if !r.is_empty() => r,
            Ok(_) => self.store.search_fts5_tokens(query, namespace, limit * 3)?,
            Err(e) => {
                tracing::warn!("FTS5 search failed, falling back to vector: {e}");
                return self.recall_inner(
                    query,
                    limit,
                    tags_filter,
                    namespace,
                    min_score,
                    None,
                    None,
                    false,
                );
            }
        };

        // Filter first, then normalize — otherwise the range is computed
        // from items that get filtered out (e.g. namespace mismatch),
        // producing skewed scores (#854 cora review).
        let filtered: Vec<(Memory, f64)> = fts_results
            .into_iter()
            .filter(|(memory, _)| {
                // Namespace filter (None = ALL, #448)
                if let Some(ns) = namespace {
                    if memory.namespace != ns {
                        return false;
                    }
                }
                // Tag filter
                if let Some(filter_tags) = tags_filter {
                    let has_tag = filter_tags
                        .iter()
                        .any(|ft| memory.tags.iter().any(|t| t == ft));
                    if !has_tag {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Compute min-max from SURVIVING results only.
        // BM25 rank: negative values, more negative = more relevant.
        let best = filtered
            .iter()
            .map(|(_, r)| *r)
            .fold(f64::INFINITY, f64::min);
        let worst = filtered
            .iter()
            .map(|(_, r)| *r)
            .fold(f64::NEG_INFINITY, f64::max);
        let range = best - worst;

        let results: Vec<SearchResult> = filtered
            .into_iter()
            .map(|(memory, rank)| {
                let score = if range.abs() < f64::EPSILON {
                    1.0f32 // single result or all same rank
                } else {
                    (((rank - worst) / range).clamp(0.0, 1.0)) as f32
                };
                SearchResult { memory, score }
            })
            .take(limit)
            .collect();

        // Filter by minimum score
        let mut results = results;
        if min_score > 0.0 {
            results.retain(|r| r.score >= min_score);
        }

        Ok(results)
    }

    /// Hybrid recall using Reciprocal Rank Fusion (RRF).
    ///
    /// Runs both vector search and FTS5 search in sequence, then merges
    /// results using RRF: `score = 1/(k + rank_vector) + 1/(k + rank_fts5)`
    fn recall_rrf(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        min_score: f32,
    ) -> Result<Vec<SearchResult>, Error> {
        const RRF_K: u32 = 60;

        // Run vector search (pass 0.0 for min_score since RRF does its own filtering)
        let vector_results = match self.recall_inner(
            query,
            limit * 3,
            tags_filter,
            namespace,
            0.0,
            None,
            None,
            false,
        ) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Vector search failed in hybrid: {e}");
                return self.recall_fts5_only(query, limit, tags_filter, namespace, min_score);
            }
        };

        // Run FTS5 search
        let fts_results = match self.store.search_fts5(query, namespace, limit * 3) {
            Ok(r) if !r.is_empty() => r,
            Ok(_) => self.store.search_fts5_tokens(query, namespace, limit * 3)?,
            Err(e) => {
                tracing::warn!("FTS5 search failed in hybrid, using vector only: {e}");
                return Ok(vector_results.into_iter().take(limit).collect());
            }
        };

        // RRF merge
        let mut rrf_scores: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        let mut memories: std::collections::HashMap<String, Memory> =
            std::collections::HashMap::new();

        // Score vector results by rank
        for (rank, sr) in vector_results.iter().enumerate() {
            let rrf = 1.0 / (RRF_K as f64 + rank as f64 + 1.0);
            *rrf_scores.entry(sr.memory.id.clone()).or_default() += rrf;
            memories
                .entry(sr.memory.id.clone())
                .or_insert_with(|| sr.memory.clone());
        }

        // Score FTS5 results by rank
        for (rank, (memory, _rank_val)) in fts_results.iter().enumerate() {
            // Apply namespace + tag filter (None = ALL, #448)
            if let Some(ns) = namespace {
                if memory.namespace != ns {
                    continue;
                }
            }
            if let Some(filter_tags) = tags_filter {
                let has_tag = filter_tags
                    .iter()
                    .any(|ft| memory.tags.iter().any(|t| t == ft));
                if !has_tag {
                    continue;
                }
            }
            let rrf = 1.0 / (RRF_K as f64 + rank as f64 + 1.0);
            *rrf_scores.entry(memory.id.clone()).or_default() += rrf;
            memories
                .entry(memory.id.clone())
                .or_insert_with(|| memory.clone());
        }

        // NOTE: min_score is NOT applied here. RRF normalized scores are
        // rank-based (0..1) and not directly comparable to cosine similarity.
        // Applying a cosine threshold to RRF scores would incorrectly filter
        // out valid results. The caller (recall_hybrid) handles threshold
        // filtering at the appropriate level.

        // #719: Jaccard token reranking boost (post-RRF, additive).
        // Measures query-content token overlap as an orthogonal signal
        // to BM25 (IDF-weighted) and vector cosine (semantic).
        let mut scored: Vec<(String, f64)> = rrf_scores.into_iter().collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut results: Vec<SearchResult> = scored
            .into_iter()
            .take(limit)
            .filter_map(|(id, score)| {
                // Every RRF id is inserted into `memories` in the same loop,
                // so this cannot miss today. If that invariant ever breaks,
                // skip the id with a warning instead of panicking (the result
                // may then hold fewer than `limit` items; `take` stays first so
                // the normal path is unchanged).
                let Some(memory) = memories.remove(&id) else {
                    tracing::warn!(
                        id = %id,
                        "RRF score references a memory missing from the fused set; skipping"
                    );
                    return None;
                };
                // RRF score is sum of 1/(k+rank) from both channels.
                // Max possible: 2/(k+1) when rank=0 in both.
                // Normalize by dividing by that theoretical max.
                let max_rrf = 2.0 / (RRF_K as f64 + 1.0);
                let normalized = (score / max_rrf).clamp(0.0, 1.0);
                Some(SearchResult {
                    memory,
                    score: normalized as f32,
                })
            })
            .collect();

        if self.jaccard_weight > 0.0 {
            let query_tokens = crate::jaccard::tokenize(query);
            if !query_tokens.is_empty() {
                for sr in &mut results {
                    // Tokenize content + tags for a richer overlap signal
                    let mut content_tokens = crate::jaccard::tokenize(&sr.memory.content);
                    for tag in &sr.memory.tags {
                        content_tokens.insert(tag.to_ascii_lowercase());
                    }
                    let j = crate::jaccard::jaccard_similarity(&query_tokens, &content_tokens);
                    sr.score += j * self.jaccard_weight;
                }
                // Re-sort after Jaccard boost
                results.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }

        Ok(results)
    }

    /// Search memories by content text (LIKE-based for v2).
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
    ) -> Result<Vec<SearchResult>, Error> {
        let memories = self.store.search_content(query, namespace, limit)?;

        let results: Vec<SearchResult> = memories
            .into_iter()
            .filter(|memory| {
                if let Some(filter_tags) = tags_filter {
                    filter_tags
                        .iter()
                        .any(|ft| memory.tags.iter().any(|t| t == ft))
                } else {
                    true
                }
            })
            .map(|memory| SearchResult {
                memory,
                score: 1.0, // Text search doesn't have meaningful scores
            })
            .collect();

        // Touch access for returned results
        let touch_ids: Vec<&str> = results.iter().map(|r| r.memory.id.as_str()).collect();
        self.store.touch_recall_batch(&touch_ids).ok();

        Ok(results)
    }
}

#[cfg(test)]
mod recall_cache_parity_tests {
    use crate::RecallStrategy;

    /// #1037: cold (cache miss) and warm (cache hit) recall_hybrid calls must
    /// return identical scores. The cache-hit path used to skip salience/
    /// recency boosts, so warm results scored lower than cold results.
    #[test]
    fn test_recall_hybrid_cold_warm_score_parity() {
        let dir = std::env::temp_dir().join(format!("parity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut uteke = crate::test_support::open_keyword_at(dir.join("t.db"));

        // Non-noop boosts (default config is 0.1/0.1 — already non-noop, but be explicit)
        uteke.set_salience_recency_config(crate::salience_recency::SalienceRecencyConfig {
            salience_weight: 0.1,
            recency_weight: 0.1,
        });

        let embedding = vec![0.42_f32; 768];
        for i in 0..3 {
            uteke
                .remember_precomputed(
                    &format!("parity probe memory number {i} about pod scheduling cluster"),
                    &[],
                    None,
                    Some("parity-ns"),
                    "fact",
                    "text",
                    &embedding,
                )
                .unwrap();
        }

        let ns = Some("parity-ns");
        let cold = uteke
            .recall_hybrid(
                "pod scheduling cluster",
                10,
                None,
                ns,
                RecallStrategy::Hybrid,
                0.0,
            )
            .unwrap();
        assert!(!cold.is_empty(), "cold call must return results");

        let warm = uteke
            .recall_hybrid(
                "pod scheduling cluster",
                10,
                None,
                ns,
                RecallStrategy::Hybrid,
                0.0,
            )
            .unwrap();
        assert_eq!(cold.len(), warm.len(), "warm call must return same count");

        let max_delta = cold
            .iter()
            .zip(warm.iter())
            .map(|(c, w)| (c.score - w.score).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_delta < 0.001,
            "cold vs warm scores must match (max delta {max_delta}); cold={:?} warm={:?}",
            cold.iter().map(|r| r.score).collect::<Vec<_>>(),
            warm.iter().map(|r| r.score).collect::<Vec<_>>(),
        );

        // Cold scores must actually be boosted (not raw 1.0-RRF plateaus only):
        // sanity — identical ids, same order
        for (c, w) in cold.iter().zip(warm.iter()) {
            assert_eq!(c.memory.id, w.memory.id);
        }

        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// #1037 (noop case): with boosts disabled, warm cache hits must still
    /// respect `limit` even though the cache stores a boost_window-sized set.
    #[test]
    fn test_recall_hybrid_noop_respects_limit_on_warm() {
        let dir = std::env::temp_dir().join(format!("parity-n-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut uteke = crate::test_support::open_keyword_at(dir.join("t.db"));
        uteke.set_salience_recency_config(crate::salience_recency::SalienceRecencyConfig {
            salience_weight: 0.0,
            recency_weight: 0.0,
        });
        let embedding = vec![0.11_f32; 768];
        for i in 0..20 {
            uteke
                .remember_precomputed(
                    &format!("noop limit probe {i} alpha beta"),
                    &[],
                    None,
                    Some("noop-ns"),
                    "fact",
                    "text",
                    &embedding,
                )
                .unwrap();
        }
        let ns = Some("noop-ns");
        let cold = uteke
            .recall_hybrid("alpha beta", 3, None, ns, RecallStrategy::Hybrid, 0.0)
            .unwrap();
        let warm = uteke
            .recall_hybrid("alpha beta", 3, None, ns, RecallStrategy::Hybrid, 0.0)
            .unwrap();
        assert!(cold.len() <= 3, "cold ≤ limit, got {}", cold.len());
        assert!(
            warm.len() <= 3,
            "warm ≤ limit even with noop boosts + window cache, got {}",
            warm.len()
        );
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// #1037 (cora finding): boosts can lift a memory from outside the raw
    /// top-N into the boosted top-N. The cached candidate set must therefore
    /// be wider than `limit` (boost window), else warm calls could never
    /// surface the boosted-in memory even though cold calls do.
    #[test]
    fn test_recall_hybrid_boost_reorder_across_limit() {
        let dir = std::env::temp_dir().join(format!("parity-r-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut uteke = crate::test_support::open_keyword_at(dir.join("t.db"));
        uteke.set_salience_recency_config(crate::salience_recency::SalienceRecencyConfig {
            salience_weight: 0.4,
            recency_weight: 0.4,
        });

        // Strong boosts: heavy access_count on the LAST raw-scored memory
        // so boosts should lift it into the top-N.
        let embedding = vec![0.42_f32; 768];
        for i in 0..6 {
            let id = uteke
                .remember_precomputed(
                    &format!("reorder probe {i} queue scheduling topic"),
                    &[],
                    None,
                    Some("reorder-ns"),
                    "fact",
                    "text",
                    &embedding,
                )
                .unwrap();
            if i == 5 {
                // Simulate heavy access: last_accessed=now, access_count high
                // via direct store touch (recall path touches, but we want it
                // ranked dead-last raw yet boosted-top after boosts).
                uteke.store.touch_access_batch(&[id.as_str()]).unwrap_or(());
                for _ in 0..20 {
                    uteke.store.touch_access_batch(&[id.as_str()]).unwrap_or(());
                }
            }
        }

        let ns = Some("reorder-ns");
        let cold = uteke
            .recall_hybrid(
                "queue scheduling topic",
                3,
                None,
                ns,
                RecallStrategy::Hybrid,
                0.0,
            )
            .unwrap();
        let warm = uteke
            .recall_hybrid(
                "queue scheduling topic",
                3,
                None,
                ns,
                RecallStrategy::Hybrid,
                0.0,
            )
            .unwrap();

        assert_eq!(cold.len(), warm.len(), "same count cold vs warm");
        for (c, w) in cold.iter().zip(warm.iter()) {
            assert_eq!(c.memory.id, w.memory.id, "same ids in same order");
        }

        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Weighted RRF fuse of two complete SearchResult rankings (#1123).
///
/// Deduplicates by memory id (first occurrence keeps the Memory payload),
/// sorts by fused score descending, and rewrites each result's score to the
/// fused RRF score. No truncation — the caller truncates to its window.
/// `pub(crate)` for the #1160 explanation path, which replays the same fuse.
pub(crate) fn rrf_fuse_weighted(
    primary: Vec<SearchResult>,
    secondary: Vec<SearchResult>,
    w_primary: f64,
    w_secondary: f64,
) -> Vec<SearchResult> {
    use std::collections::HashMap;
    let mut scores: HashMap<String, f64> = HashMap::new();
    for (rank, r) in primary.iter().enumerate() {
        *scores.entry(r.memory.id.clone()).or_default() +=
            w_primary / (FUSION_RRF_K + rank as f64 + 1.0);
    }
    for (rank, r) in secondary.iter().enumerate() {
        *scores.entry(r.memory.id.clone()).or_default() +=
            w_secondary / (FUSION_RRF_K + rank as f64 + 1.0);
    }

    // Chain both rankings, dedup by id (first occurrence wins the payload),
    // then stable-sort by fused score descending.
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<SearchResult> = primary
        .into_iter()
        .chain(secondary)
        .filter(|r| seen.insert(r.memory.id.clone()))
        .collect();
    out.sort_by(|a, b| {
        let sb = scores.get(&b.memory.id).copied().unwrap_or(0.0);
        let sa = scores.get(&a.memory.id).copied().unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    for r in out.iter_mut() {
        r.score = scores.get(&r.memory.id).copied().unwrap_or(0.0) as f32;
    }
    out
}

#[cfg(test)]
mod rrf_fuse_weighted_tests {
    use super::rrf_fuse_weighted;
    use crate::memory::types::{Memory, SearchResult};

    fn sr(id: &str, score: f32) -> SearchResult {
        let now = chrono::Utc::now();
        let memory = Memory {
            id: id.to_string(),
            content: format!("content {id}"),
            embedding: vec![0.1_f32; 4],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
            namespace: "test".to_string(),
            access_count: 0,
            recall_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: Some(now),
            valid_until: None,
            memory_type: "note".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
            author_type: "agent".to_string(),
            author: None,
        };
        SearchResult { memory, score }
    }

    #[test]
    fn overlap_wins_and_ids_dedup() {
        // 'both' appears in BOTH rankings; 'gold' only in secondary.
        let primary = vec![sr("a", 0.9), sr("b", 0.8), sr("both", 0.7)];
        let secondary = vec![sr("gold", 0.95), sr("both", 0.85), sr("c", 0.6)];
        let fused = rrf_fuse_weighted(primary, secondary, 1.7, 1.0);
        // 'both' ranks 3rd primary + 2nd secondary → highest fused score.
        assert_eq!(fused[0].memory.id, "both");
        // No duplicate ids survive.
        let n = fused.len();
        let uniq = fused
            .iter()
            .map(|r| r.memory.id.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(n, uniq);
        // Scores are fused RRF scores (small positive), not the originals.
        assert!(fused[0].score > 0.0 && fused[0].score < 0.1);
    }

    #[test]
    fn primary_weight_orders_disagreeing_rankings() {
        let primary = vec![sr("x", 0.9), sr("y", 0.8)];
        let secondary = vec![sr("y", 0.95), sr("x", 0.85)];
        // x: 1st primary + 2nd secondary; y: mirrored. Higher primary weight
        // must lift x; higher secondary weight must lift y.
        let f1 = rrf_fuse_weighted(primary.clone(), secondary.clone(), 2.0, 1.0);
        assert_eq!(f1[0].memory.id, "x");
        let f2 = rrf_fuse_weighted(primary, secondary, 0.5, 2.0);
        assert_eq!(f2[0].memory.id, "y");
    }

    #[test]
    fn empty_inputs_yield_empty() {
        let fused = rrf_fuse_weighted(vec![], vec![], 1.7, 1.0);
        assert!(fused.is_empty());
        let fused = rrf_fuse_weighted(vec![sr("only", 0.9)], vec![], 1.7, 1.0);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].memory.id, "only");
    }
}

#[cfg(test)]
mod fusion_strategy_tests {
    use crate::memory::types::RecallStrategy;

    /// #1123: Fusion strategy end-to-end — seeds a store, runs recall with
    /// the explicit Fusion strategy, and asserts the on-topic memory surfaces
    /// in the top results. Also asserts cold/warm parity through the cache.
    #[test]
    fn fusion_recall_finds_on_topic_memory() {
        let dir = std::env::temp_dir().join(format!("fusion-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = crate::test_support::open_keyword_at(dir.join("t.db"));

        let base = vec![0.1_f32; 768];
        // On-topic memory: near-parallel to the probe direction.
        let mut on_topic = base.clone();
        for v in on_topic.iter_mut().take(384) {
            *v = 0.9;
        }
        // Off-topic memories: anti-parallel.
        let mut off = base.clone();
        for v in off.iter_mut().take(384) {
            *v = -0.9;
        }

        uteke
            .remember_precomputed(
                "quarterly revenue projections for the board meeting",
                &["finance"],
                None,
                Some("fusion-ns"),
                "fact",
                "text",
                &on_topic,
            )
            .unwrap();
        for i in 0..4 {
            uteke
                .remember_precomputed(
                    &format!("unrelated filler note {i} about gardening"),
                    &["misc"],
                    None,
                    Some("fusion-ns"),
                    "note",
                    "text",
                    &off,
                )
                .unwrap();
        }

        let ns = Some("fusion-ns");
        let cold = uteke
            .recall_hybrid(
                "board meeting revenue projections",
                3,
                None,
                ns,
                RecallStrategy::Fusion,
                0.0,
            )
            .unwrap();
        assert!(!cold.is_empty(), "fusion must return results");
        assert_eq!(
            cold[0].memory.content, "quarterly revenue projections for the board meeting",
            "on-topic memory must rank first under fusion"
        );
        assert!(cold.len() <= 3);

        // Cold/warm parity through the recall cache (#1037 invariant).
        let warm = uteke
            .recall_hybrid(
                "board meeting revenue projections",
                3,
                None,
                ns,
                RecallStrategy::Fusion,
                0.0,
            )
            .unwrap();
        let cold_ids: Vec<_> = cold.iter().map(|r| r.memory.id.clone()).collect();
        let warm_ids: Vec<_> = warm.iter().map(|r| r.memory.id.clone()).collect();
        assert_eq!(cold_ids, warm_ids, "cold and warm fusion must match");

        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Fusion must not silently degrade: an explicit Fusion call on an empty
    /// store returns empty results (both sub-rankings empty), NOT an error
    /// and NOT a fallback ranking.
    #[test]
    fn fusion_recall_empty_store_returns_empty() {
        let dir = std::env::temp_dir().join(format!("fusion-e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = crate::test_support::open_keyword_at(dir.join("t.db"));
        let res = uteke
            .recall_hybrid(
                "anything",
                5,
                None,
                Some("empty-ns"),
                RecallStrategy::Fusion,
                0.0,
            )
            .unwrap();
        assert!(res.is_empty());
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod recall_touch_tests {
    //! #1407 — one user-level recall counts each RETURNED memory once.
    use crate::RecallStrategy;
    use crate::test_support::open_keyword;

    /// Three memories; only the first two share words with the query, and
    /// `limit = 1` returns exactly one of them.
    fn seeded() -> (crate::Uteke, [String; 3]) {
        let u = open_keyword();
        let mut ids = Vec::new();
        for text in [
            "alpha beta gamma delta",
            "alpha epsilon zeta eta",
            "omega sigma tau upsilon",
        ] {
            ids.push(u.remember(text, &[], None, Some("ns")).unwrap());
        }
        let [a, b, c]: [String; 3] = ids.try_into().unwrap();
        (u, [a, b, c])
    }

    fn counts(u: &crate::Uteke, ids: &[String; 3]) -> [u32; 3] {
        let mut out = [0; 3];
        for (i, id) in ids.iter().enumerate() {
            out[i] = u.store().get_by_id(id).unwrap().unwrap().recall_count;
        }
        out
    }

    #[test]
    #[serial_test::serial]
    fn every_strategy_counts_the_returned_memory_exactly_once() {
        for strategy in [
            RecallStrategy::Vector,
            RecallStrategy::Fts5,
            RecallStrategy::Hybrid,
            RecallStrategy::Fusion,
        ] {
            let (u, ids) = seeded();
            let res = u
                .recall_hybrid("alpha beta", 1, None, Some("ns"), strategy, 0.0)
                .unwrap();
            assert_eq!(res.len(), 1, "{strategy:?}");
            let c = counts(&u, &ids);
            // exactly one memory counted, once; nothing else touched
            assert_eq!(c.iter().sum::<u32>(), 1, "{strategy:?}: {c:?}");
            let returned = ids.iter().position(|i| *i == res[0].memory.id).unwrap();
            assert_eq!(c[returned], 1, "{strategy:?}: {c:?}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn a_repeated_query_served_from_the_cache_still_counts_once_per_call() {
        let (u, ids) = seeded();
        for expected in 1..=3u32 {
            let res = u
                .recall_hybrid(
                    "alpha beta",
                    1,
                    None,
                    Some("ns"),
                    RecallStrategy::Fusion,
                    0.0,
                )
                .unwrap();
            let returned = ids.iter().position(|i| *i == res[0].memory.id).unwrap();
            assert_eq!(counts(&u, &ids)[returned], expected);
        }
    }

    #[test]
    #[serial_test::serial]
    fn the_public_vector_recall_counts_returned_results_once() {
        let (u, ids) = seeded();
        let res = u
            .recall("alpha beta", 1, None, Some("ns"), 0.0, None, None)
            .unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(counts(&u, &ids).iter().sum::<u32>(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn explain_is_a_read_only_diagnostic() {
        let (u, ids) = seeded();
        let _ = u
            .recall_explained(
                "alpha beta",
                2,
                None,
                Some("ns"),
                RecallStrategy::Fusion,
                0.0,
            )
            .unwrap();
        assert_eq!(counts(&u, &ids), [0, 0, 0]);
    }
}

#[cfg(test)]
mod recall_no_embedder_tests {
    #[cfg(feature = "onnx")]
    use crate::Uteke;

    /// A store whose embedder slot is empty and cannot be filled: the
    /// "custom" backend with no injected embedder. `ensure_embedder` fails,
    /// so vector recall must return an error, never panic. Gated on `onnx`
    /// because non-onnx builds reject "custom" at open().
    #[cfg(feature = "onnx")]
    #[test]
    fn recall_without_embedder_returns_error() {
        let (_db_str, store) = Uteke::open_store(":memory:").expect("open_store");
        let u = Uteke::finish_open_full(
            store,
            None,
            "custom".to_string(),
            crate::TierConfig::default(),
            crate::RecallConfig::default(),
            crate::EmbeddingSettings::default(),
            crate::graph_rerank::GraphRerankConfig::default(),
            None,
        )
        .expect("finish_open_full");

        let res = u.recall("anything", 5, None, None, 0.0, None, None);
        let err = res.expect_err("recall without an embedder must be an error");
        assert!(
            err.to_string()
                .contains("Custom embedder backend set but no embedder was provided"),
            "got: {err}"
        );
    }
}

/// Exact-value tests for `recall_rrf` that the 0.21.0 mutation gate missed:
/// the vector candidate window (`limit * 3`) and the score normalisation
/// (`score / max_rrf`). They call `recall_rrf` directly so the boost window
/// and salience/recency boosts of `recall_hybrid` stay out of the numbers.
/// The keyword embedder hashes words into 64 buckets; "zebra" and "aab",
/// "afk", "ajg", "ans", "aol" share bucket 15, which lets a memory be close
/// in vector space without matching the FTS query.
#[cfg(test)]
mod mutant_kill_tests_0_21 {
    use crate::test_support::open_keyword;

    const K: f64 = 60.0;

    fn rrf_at(rank: usize) -> f64 {
        1.0 / (K + rank as f64 + 1.0)
    }

    fn normalized(score: f64) -> f32 {
        (score / (2.0 / (K + 1.0))) as f32
    }

    /// Rank 0 in both channels normalises to exactly 1.0 (the theoretical
    /// max); a memory found by one channel only, at rank 0, to exactly 0.5.
    /// With `limit = 1` the vector window is 3; `limit / 3` would make it 0
    /// and leave the top memory with FTS credit only (0.5).
    #[test]
    fn rank_zero_in_both_channels_scores_exactly_one() {
        let u = open_keyword();
        let top = u.remember("zebra", &[], None, None).unwrap();
        u.remember("apple river stone", &[], None, None).unwrap();

        let got = u.recall_rrf("zebra", 1, None, None, 0.0).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].memory.id, top);
        assert_eq!(got[0].score, 1.0);
        assert_eq!(got[0].score, normalized(rrf_at(0) + rrf_at(0)));
    }

    #[test]
    fn rank_zero_in_one_channel_scores_exactly_half() {
        let u = open_keyword();
        // Vector neighbour of "zebra" (shared bucket) without the FTS word.
        let vec_only = u.remember("aab", &[], None, None).unwrap();
        u.remember("apple river stone", &[], None, None).unwrap();

        let got = u.recall_rrf("zebra", 1, None, None, 0.0).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].memory.id, vec_only);
        assert_eq!(got[0].score, 0.5);
    }

    /// With `limit = 2` the vector window is 6. The only FTS hit sits at
    /// vector rank 5 (the 6th), so it gets credit from both channels and
    /// ranks first. With a window of `limit + 3` = 5 it would get FTS credit
    /// only, tie at 0.5 with the vector-only leader and lose its score.
    #[test]
    fn vector_window_is_three_times_the_limit() {
        let u = open_keyword();
        // Cosine to "zebra" = 1/sqrt(2): vector ranks 0..=4, no FTS match.
        for c in [
            "aab apple",
            "afk stone",
            "ajg cloud",
            "ans maple",
            "aol tiger",
        ] {
            u.remember(c, &[], None, None).unwrap();
        }
        // Cosine 1/2: vector rank 5; FTS rank 0 (the only FTS hit).
        let target = u
            .remember("zebra piano lemon candle", &[], None, None)
            .unwrap();

        let got = u.recall_rrf("zebra", 2, None, None, 0.0).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].memory.id, target);
        assert_eq!(got[0].score, normalized(rrf_at(5) + rrf_at(0)));
        assert_eq!(got[1].score, 0.5);
    }
}
