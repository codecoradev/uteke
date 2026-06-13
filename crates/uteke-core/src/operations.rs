//! Core memory operations: remember, recall, search, forget, list, get, tags.

use crate::error::Error;
use crate::memory::types::{
    BulkDeleteResult, Memory, MemoryTier, RecallStrategy, SearchResult, TagInfo, DEFAULT_NAMESPACE,
};
use crate::memory::vector::cosine_distance_to_similarity;

impl crate::Uteke {
    /// Store a new memory.
    ///
    /// Returns the UUID of the created memory.
    pub fn remember(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
    ) -> Result<String, Error> {
        self.remember_typed(content, tags, metadata, namespace, "fact")
    }

    /// Store a new memory with explicit type.
    ///
    /// Returns the UUID of the created memory.
    pub fn remember_typed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
    ) -> Result<String, Error> {
        crate::validate_input(content, tags)?;
        // Validate memory_type against known variants
        crate::memory::types::MemoryType::from_str_opt(memory_type).ok_or_else(|| {
            Error::Validation(format!(
                "Unknown memory type '{memory_type}'. Valid types: fact, procedure, preference, decision, context"
            ))
        })?;
        // Lazy-load embedder on first use
        self.ensure_embedder()?;
        let embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during remember"))?
            .as_ref()
            .expect("embedder ensured above")
            .embed(content)?;
        self.remember_precomputed(content, tags, metadata, namespace, memory_type, &embedding)
    }

    /// Store a new memory with a pre-computed embedding.
    ///
    /// Use when the embedding has already been computed (e.g., contradiction check).
    /// Returns the UUID of the created memory.
    pub(crate) fn remember_precomputed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
        embedding: &[f32],
    ) -> Result<String, Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        let memory = Memory {
            id: id.clone(),
            content: content.to_string(),
            embedding: embedding.to_vec(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            metadata: metadata.unwrap_or(serde_json::Value::Null),
            created_at: now,
            updated_at: now,
            namespace: namespace.unwrap_or(DEFAULT_NAMESPACE).to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: Some(now),
            valid_until: None,
            memory_type: memory_type.to_string(),
            importance: 0.5,
            pinned: false,
        };

        // Acquire index write lock BEFORE any writes so lock failures are detected early.
        // If SQLite commit fails after index insert, the orphan index entry is harmless
        // and will be cleaned up by verify/repair.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during remember"))?;

        self.store.insert(&memory)?;

        // Invalidate recall cache — new memory may affect future queries
        self.recall_cache.invalidate_namespace(&memory.namespace);

        index.insert(&id, embedding)?;
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after remember for id={id}: {e}. \
                 Index entry can be rebuilt via `uteke repair`."
            );
        }

        Ok(id)
    }

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
    ) -> Result<Vec<SearchResult>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);

        // Embed query outside any lock — CPU-intensive (~50ms), no shared state needed.
        // Only the embedder Mutex is held here, allowing concurrent index reads.
        // Lazy-load embedder on first use.
        self.ensure_embedder()?;
        let query_embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during recall"))?
            .as_ref()
            .expect("embedder ensured above")
            .embed(query)?;
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
            for (memory_id, distance) in &candidates {
                if results.len() >= limit {
                    break;
                }

                let memory = match self.store.get_by_id(memory_id)? {
                    Some(m) => m,
                    None => continue,
                };

                // Apply namespace filter
                if memory.namespace != ns {
                    continue;
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
                    memory,
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

        // Touch access for returned results
        for r in &results {
            self.store.touch_access(&r.memory.id).ok();
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
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);

        // Check recall cache first — avoids redundant embedding (~50ms).
        // min_score is NOT in the cache key: cached results store the full set
        // and the caller re-applies threshold, ensuring correctness regardless
        // of what threshold a previous caller used.
        if let Some(cached) = self
            .recall_cache
            .get(query, ns, limit, tags_filter, strategy)
        {
            let mut results = cached;
            if min_score > 0.0 {
                results.retain(|r| r.score >= min_score);
            }
            results.truncate(limit);
            return Ok(results);
        }

        let results = match strategy {
            RecallStrategy::Vector => {
                self.recall(query, limit, tags_filter, namespace, min_score)?
            }
            RecallStrategy::Fts5 => {
                self.recall_fts5_only(query, limit, tags_filter, namespace, min_score)?
            }
            // Hybrid (RRF): min_score is passed but not used for filtering.
            // RRF scores are rank-based, not cosine similarity. Applying a
            // cosine threshold to RRF scores would incorrectly filter results.
            RecallStrategy::Hybrid => {
                self.recall_rrf(query, limit, tags_filter, namespace, min_score)?
            }
        };

        // Cache results for future queries (without min_score filtering,
        // so cached results are reusable for any threshold)
        self.recall_cache
            .put(query, ns, limit, tags_filter, strategy, results.clone());

        Ok(results)
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
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);

        // Try phrase search first, fall back to token search
        let fts_results = match self.store.search_fts5(query, namespace, limit * 3) {
            Ok(r) if !r.is_empty() => r,
            Ok(_) => self.store.search_fts5_tokens(query, namespace, limit * 3)?,
            Err(e) => {
                tracing::warn!("FTS5 search failed, falling back to vector: {e}");
                return self.recall(query, limit, tags_filter, namespace, min_score);
            }
        };

        let results: Vec<SearchResult> = fts_results
            .into_iter()
            .filter(|(memory, _)| {
                // Namespace filter (FTS5 may return cross-namespace if not filtered)
                if memory.namespace != ns {
                    return false;
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
            .map(|(memory, _rank)| {
                // Convert FTS5 BM25 rank to 0..1 score.
                // BM25 returns negative values (more negative = worse relevance).
                // We use rank-based scoring instead of raw BM25 since
                // BM25 magnitudes vary by query and aren't normalized.
                // Score is assigned by position in the result list.
                let score = 1.0f32; // Placeholder — actual ranking done by RRF in hybrid
                SearchResult { memory, score }
            })
            .take(limit)
            .collect();

        // Filter by minimum score
        let mut results = results;
        if min_score > 0.0 {
            results.retain(|r| r.score >= min_score);
        }

        // Touch access for returned results
        for r in &results {
            self.store.touch_access(&r.memory.id).ok();
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
        let vector_results = match self.recall(query, limit * 3, tags_filter, namespace, 0.0) {
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
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        for (rank, (memory, _rank_val)) in fts_results.iter().enumerate() {
            // Apply namespace + tag filter
            if memory.namespace != ns {
                continue;
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

        // Sort by RRF score descending, take top `limit`
        let mut scored: Vec<(String, f64)> = rrf_scores.into_iter().collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<SearchResult> = scored
            .into_iter()
            .take(limit)
            .map(|(id, score)| {
                let memory = memories
                    .remove(&id)
                    .expect("RRF score references memory that should exist");
                // RRF score is sum of 1/(k+rank) from both channels.
                // Max possible: 2/(k+1) when rank=0 in both.
                // Normalize by dividing by that theoretical max.
                let max_rrf = 2.0 / (RRF_K as f64 + 1.0);
                let normalized = (score / max_rrf).clamp(0.0, 1.0);
                SearchResult {
                    memory,
                    score: normalized as f32,
                }
            })
            .collect();

        // NOTE: min_score is NOT applied here. RRF normalized scores are
        // rank-based (0..1) and not directly comparable to cosine similarity.
        // Applying a cosine threshold to RRF scores would incorrectly filter
        // out valid results. The caller (recall_hybrid) handles threshold
        // filtering at the appropriate level.

        // Touch access for returned results
        for r in &results {
            self.store.touch_access(&r.memory.id).ok();
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

        let results = memories
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

        Ok(results)
    }

    /// Delete a memory by ID. Incremental — no index rebuild.
    pub fn forget(&self, id: &str) -> Result<(), Error> {
        // Look up namespace before delete for targeted cache invalidation.
        // If lookup succeeds, invalidate only that namespace.
        // If the memory simply doesn't exist, no invalidation needed.
        // We intentionally do NOT clear the entire cache on lookup errors
        // to avoid cross-namespace regressions from transient failures.
        if let Some(memory) = self.store.get_by_id(id).ok().flatten() {
            self.recall_cache.invalidate_namespace(&memory.namespace);
        }

        // Acquire index write lock BEFORE SQLite delete to narrow inconsistency window.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during forget"))?;
        // SQLite delete (source of truth)
        self.store.delete(id)?;
        // Vector index remove — orphan is harmless if fails (verify/repair cleans up)
        if !index.remove(id) {
            tracing::warn!("Vector index entry not found during forget for id={id}");
        }
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after forget for id={id}: {e}. \
                 Orphan entry will be cleaned up by verify/repair."
            );
        }

        Ok(())
    }

    /// Bulk delete memories by tag. Also removes from index.
    pub fn bulk_forget_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<BulkDeleteResult, Error> {
        // Acquire index write lock BEFORE SQLite delete to narrow inconsistency window.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during bulk_forget_by_tag"))?;
        let ids = self.store.bulk_delete_by_tag(tag, namespace)?;
        for id in &ids {
            if !index.remove(id) {
                tracing::warn!(
                    "Vector index entry not found during bulk_forget_by_tag for id={id}"
                );
            }
        }
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after bulk_forget_by_tag: {e}. \
                 Orphan entries will be cleaned up by verify/repair."
            );
        }
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// Bulk delete all cold memories. Also removes from index.
    pub fn bulk_forget_cold(&self, namespace: Option<&str>) -> Result<BulkDeleteResult, Error> {
        // Acquire index write lock BEFORE SQLite delete to narrow inconsistency window.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during bulk_forget_cold"))?;
        let ids = self
            .store
            .bulk_delete_cold(namespace, self.tier_config.warm_days)?;
        for id in &ids {
            if !index.remove(id) {
                tracing::warn!("Vector index entry not found during bulk_forget_cold for id={id}");
            }
        }
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after bulk_forget_cold: {e}. \
                 Orphan entries will be cleaned up by verify/repair."
            );
        }
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// Bulk delete all memories in a namespace. Also removes from index.
    pub fn bulk_forget_all(&self, namespace: Option<&str>) -> Result<BulkDeleteResult, Error> {
        // Acquire index write lock BEFORE SQLite delete to narrow inconsistency window.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during bulk_forget_all"))?;
        let ids = self.store.bulk_delete_all(namespace)?;
        for id in &ids {
            if !index.remove(id) {
                tracing::warn!("Vector index entry not found during bulk_forget_all for id={id}");
            }
        }
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after bulk_forget_all: {e}. \
                 Orphan entries will be cleaned up by verify/repair."
            );
        }
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// List memories with optional tag filter and pagination.
    pub fn list(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
        namespace: Option<&str>,
    ) -> Result<Vec<Memory>, Error> {
        self.store.list(tag, namespace, limit, offset)
    }

    /// Get a single memory by ID.
    pub fn get(&self, id: &str) -> Result<Memory, Error> {
        let memory = self
            .store
            .get_by_id(id)?
            .ok_or_else(|| Error::db_msg(format!("Memory not found: {id}")))?;
        self.store.touch_access(id).ok();
        Ok(memory)
    }

    /// List all namespaces.
    pub fn list_namespaces(&self) -> Result<Vec<String>, Error> {
        self.store.list_namespaces()
    }

    /// List all tags with their usage counts.
    pub fn tags_with_counts(&self, namespace: Option<&str>) -> Result<Vec<TagInfo>, Error> {
        self.store.tags_with_counts(namespace)
    }

    /// Rename a tag across all memories in a namespace.
    pub fn rename_tag(
        &self,
        old: &str,
        new: &str,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        self.store.rename_tag(old, new, namespace)
    }

    /// Delete a tag from all memories in a namespace.
    pub fn delete_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.delete_tag(tag, namespace)
    }

    /// Count memories by tag in a namespace.
    pub fn count_by_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.count_by_tag(tag, namespace)
    }

    /// Count total memories, optionally filtered by namespace.
    pub fn count(&self, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.count(namespace)
    }

    /// Get a memory by ID (without touching access count — used for internal lookups).
    pub fn get_by_id(&self, id: &str) -> Result<Option<Memory>, Error> {
        self.store.get_by_id(id)
    }

    /// Recall memories that existed at a specific point in time.
    ///
    /// Runs a semantic recall to gather candidates, then post-filters by
    /// temporal validity at `point_in_time`:
    /// - `created_at <= point_in_time`
    /// - `valid_until IS NULL OR valid_until > point_in_time`
    /// - `valid_from IS NULL OR valid_from <= point_in_time`
    /// - `deprecated = false`
    pub fn recall_at_time(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        point_in_time: chrono::DateTime<chrono::Utc>,
        min_score: f32,
    ) -> Result<Vec<SearchResult>, Error> {
        // Retry loop: over-fetch with increasing multipliers to compensate
        // for temporal filtering removing candidates. If post-filtering
        // yields fewer than `limit` results, expand the search scope.
        let mut multiplier = 3usize;
        let mut attempt = 0;

        loop {
            let fetch_limit = (limit * multiplier).max(50);
            let candidates = self.recall(query, fetch_limit, tags_filter, namespace, min_score)?;
            let candidates_len = candidates.len();

            let mut results: Vec<SearchResult> = candidates
                .into_iter()
                .filter(|r| {
                    // Memory must have existed at this time
                    if r.memory.created_at > point_in_time {
                        return false;
                    }
                    // Memory must not have been invalidated before this time
                    if let Some(valid_until) = r.memory.valid_until {
                        if valid_until <= point_in_time {
                            return false;
                        }
                    }
                    // Memory should not be deprecated
                    if r.memory.deprecated {
                        return false;
                    }
                    // valid_from should be before point_in_time (if set)
                    if let Some(valid_from) = r.memory.valid_from {
                        if valid_from > point_in_time {
                            return false;
                        }
                    }
                    true
                })
                .collect();

            // Stop if we have enough results or exhausted retry budget.
            if results.len() >= limit || attempt >= 2 {
                results.truncate(limit);
                return Ok(results);
            }

            // If the fetch returned fewer candidates than fetch_limit, the
            // index is exhausted — expanding the search scope won't help.
            if candidates_len < fetch_limit {
                results.truncate(limit);
                return Ok(results);
            }

            attempt += 1;
            multiplier *= 3;
        }
    }

    /// List memories that existed at a specific point in time.
    ///
    /// Thin wrapper around the store-level temporal query.
    pub fn list_at_time(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
        namespace: Option<&str>,
        point_in_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Memory>, Error> {
        self.store
            .list_at_time(tag, namespace, limit, offset, point_in_time)
    }
}
