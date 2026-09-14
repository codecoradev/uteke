//! Core memory operations: remember, recall, search, forget, list, get, tags.

use crate::error::Error;
use crate::memory::types::{
    BulkDeleteResult, DEFAULT_NAMESPACE, Memory, MemoryTier, NamespaceDeleteResult,
    NamespaceRenameResult, RecallStrategy, SearchResult, TagInfo,
};
use crate::memory::vector::cosine_distance_to_similarity;
use std::sync::Mutex;
use std::time::Duration;

/// Retry embedding generation with exponential backoff (#621).
///
/// Embedding failures silently drop vector entries, causing the vector
/// index to desync from SQLite. This helper retries up to 3 times with
/// 200ms, 400ms, then 800ms delays between attempts.
pub(crate) fn retry_embed(
    embedder: &Mutex<Option<Box<dyn crate::embed::Embedder>>>,
    text: &str,
) -> Result<Vec<f32>, Error> {
    const MAX_RETRIES: usize = 3;
    let mut delay = Duration::from_millis(200);

    for attempt in 0..MAX_RETRIES {
        let lock = embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during remember"))?;
        let embedder = lock.as_ref().expect("embedder ensured above");
        match embedder.embed(text) {
            Ok(embedding) => return Ok(embedding),
            Err(e) => {
                drop(lock); // Release lock before sleeping
                if attempt < MAX_RETRIES - 1 {
                    tracing::warn!(
                        "Embedding attempt {}/{} failed: {}. Retrying in {:?}...",
                        attempt + 1,
                        MAX_RETRIES,
                        e,
                        delay
                    );
                    std::thread::sleep(delay);
                    delay *= 2;
                } else {
                    tracing::error!(
                        "Embedding failed after {} attempts: {}. \
                         Memory will be stored WITHOUT vector embedding. \
                         Run `uteke repair` to rebuild index.",
                        MAX_RETRIES,
                        e
                    );
                    return Err(e);
                }
            }
        }
    }
    unreachable!()
}

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
        // Default path uses auto-inference (#349). Passing "fact" explicitly
        // would bypass inference — use remember_auto_infer(None) so content
        // signals drive the type.
        self.remember_auto_infer(content, tags, metadata, namespace, None)
    }

    /// Store a JSON-structured memory. Content must be valid JSON.
    ///
    /// This is a convenience wrapper that validates JSON before storing.
    /// The `remember()` method also auto-detects JSON content.
    #[deprecated(note = "unused — candidate for removal in future version")]
    #[allow(dead_code)]
    pub fn remember_json(
        &self,
        json_content: &str,
        tags: &[&str],
        namespace: Option<&str>,
    ) -> Result<String, Error> {
        serde_json::from_str::<serde_json::Value>(json_content)
            .map_err(|e| Error::Validation(format!("Invalid JSON content: {e}")))?;
        self.remember(json_content, tags, None, namespace)
    }

    /// Store a new memory with explicit type.
    ///
    /// The caller-chosen type is honored as-is — no auto-inference runs
    /// (CodeCora #386). Use [`Self::remember_auto_infer`] for the
    /// pattern-based auto-inference path.
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
        // Validate memory_type against known variants. The type is used
        // as-is — no inference, no override.
        crate::memory::types::MemoryType::from_str_opt(memory_type).ok_or_else(|| {
            Error::Validation(format!(
                "Unknown memory type '{memory_type}'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
            ))
        })?;
        self.remember_embed(content, tags, metadata, namespace, memory_type)
    }

    /// Store a new memory with auto-inferred type (#349).
    ///
    /// Runs pattern-based inference on the content. If the caller passes
    /// `Some(explicit_type)`, that type wins and inference is skipped. If
    /// `None`, the inference result is used (falling back to `Fact` when
    /// the content is ambiguous, preserving backward compatibility with
    /// pre-#349 callers).
    ///
    /// Returns the UUID of the created memory.
    pub fn remember_auto_infer(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        explicit_type: Option<&str>,
    ) -> Result<String, Error> {
        let effective_type = match explicit_type {
            Some(t) => {
                // Validate explicit type — same check as remember_typed
                // (CodeCora #386 r2).
                crate::memory::types::MemoryType::from_str_opt(t).ok_or_else(|| {
                    Error::Validation(format!(
                        "Unknown memory type '{t}'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
                    ))
                })?;
                t.to_string()
            }
            None => {
                let inferred = crate::memory::types::MemoryType::infer_from_content(content);
                if inferred == crate::memory::types::MemoryType::Note {
                    // Ambiguous content → keep Fact (backward compat).
                    "fact".to_string()
                } else {
                    inferred.as_str().to_string()
                }
            }
        };
        self.remember_embed(content, tags, metadata, namespace, &effective_type)
    }

    /// Embed-then-store shared by [`remember_typed`] and [`remember_auto_infer`].
    fn remember_embed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
    ) -> Result<String, Error> {
        crate::validate_input(content, tags)?;
        // Validate memory_type against known variants.
        crate::memory::types::MemoryType::from_str_opt(memory_type).ok_or_else(|| {
            Error::Validation(format!(
                "Unknown memory type '{memory_type}'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
            ))
        })?;
        // Detect JSON content and use flattened text for embedding
        let content_type = crate::memory::crud::detect_content_type(content);
        let embed_text = if content_type == "json" {
            crate::memory::crud::flatten_json_for_embedding(content)
        } else {
            content.to_string()
        };
        // Lazy-load embedder on first use. #1166: when no backend is
        // configured (backend == "" on a build without the onnx feature,
        // or open_with_backend(.., None)), skip embedding entirely — the
        // row is stored FTS5-only and stays keyword-searchable. Vector
        // search for this row becomes available once an embedding is
        // supplied via the injected-embedding path or `uteke repair`.
        let embedding = if self.embedder_backend.is_empty() {
            tracing::debug!("No embedding backend configured; storing memory FTS5-only (#1166)");
            Vec::new()
        } else {
            self.ensure_embedder()?;
            // Retry embedding generation up to 3 times with exponential backoff.
            // Embedding failures silently drop vector entries, causing desync (#621).
            self::retry_embed(&self.embedder, &embed_text)?
        };

        // Dedup check: if an existing memory has cosine >= 0.95, return it
        // instead of creating a duplicate (#442 enhancement).
        // #1166: skip when no embedding was produced (no embedder) — cosine
        // dedup is meaningless without vectors.
        if !embedding.is_empty() {
            if let Some(existing_id) = self.check_duplicate(&embedding, namespace)? {
                tracing::info!("Dedup: memory {existing_id} is nearly identical, skipping insert");
                return Ok(existing_id);
            }
        }

        self.remember_precomputed(
            content,
            tags,
            metadata,
            namespace,
            memory_type,
            content_type,
            &embedding,
        )
    }

    /// Store a new memory with a pre-computed embedding.
    ///
    /// Use when the embedding has already been computed (e.g., contradiction check).
    /// Returns the UUID of the created memory.
    #[allow(clippy::too_many_arguments)]
    /// Check if a near-duplicate memory already exists (#442 enhancement).
    ///
    /// Searches the vector index for cosine >= 0.95. If found, returns
    /// the existing memory ID so caller can skip the insert.
    /// Only checks within the same namespace.
    pub(crate) fn check_duplicate(
        &self,
        embedding: &[f32],
        namespace: Option<&str>,
    ) -> Result<Option<String>, Error> {
        const DEDUP_THRESHOLD: f32 = 0.95;

        let index = match self.index.try_read() {
            Ok(i) => i,
            Err(_) => return Ok(None), // Don't block if locked
        };

        if index.is_empty() {
            return Ok(None);
        }

        let results = index.search(embedding, 5, 50);
        drop(index);

        if results.is_empty() {
            return Ok(None);
        }

        // Filter by namespace if specified.
        let ns_set: Option<std::collections::HashSet<String>> = if let Some(ns) = namespace {
            match self.store.memories_in_namespace(ns) {
                Ok(ids) => Some(ids.into_iter().collect()),
                Err(_) => return Ok(None),
            }
        } else {
            None
        };

        for (id, dist) in &results {
            // Skip chunk: prefixed entries (document chunks).
            if id.starts_with("chunk:") {
                continue;
            }
            // Namespace filter.
            if let Some(ref set) = ns_set {
                if !set.contains(id) {
                    continue;
                }
            }
            let sim = (1.0 - dist).clamp(0.0, 1.0);
            if sim >= DEDUP_THRESHOLD {
                // Verify against SQLite before trusting the index hit (#1210).
                // A stale index can still hold vectors for rows that were
                // soft-deleted (deprecated) or hard-deleted; deduplicating
                // against them silently drops the write and can link rooms
                // to invisible ghost rows. Treat them as absent and keep
                // scanning remaining candidates.
                match self.store.get_by_id(id) {
                    // Live row — safe to dedup onto.
                    Ok(Some(m)) if !m.deprecated => return Ok(Some(id.clone())),
                    // Missing (hard-deleted) or deprecated row — stale index
                    // entry; skip this candidate and keep scanning (#1210).
                    Ok(_) => continue,
                    // Store read failed — never dedup on an unreliable read.
                    Err(_) => continue,
                }
            }
        }

        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remember_precomputed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
        content_type: &str,
        embedding: &[f32],
    ) -> Result<String, Error> {
        let id = uuid::Uuid::now_v7().to_string();
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
            deprecated_at: None,
            valid_from: Some(now),
            valid_until: None,
            memory_type: memory_type.to_string(),
            importance: 0.5,
            pinned: false,
            content_type: content_type.to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
            author_type: "agent".to_string(),
        };

        // Acquire index write lock BEFORE any writes so lock failures are detected early.
        // If SQLite commit fails after index insert, the orphan index entry is harmless
        // and will be cleaned up by verify/repair.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during remember"))?;

        self.store.insert(&memory)?;

        // Provenance: record the content hash at write time (#1172 Fase 1).
        // Best-effort — audits recompute this to detect post-write tampering.
        use sha2::Digest;
        let source_hash: String = sha2::Sha256::digest(content.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if let Err(e) = self.store.set_source_hash(&id, Some(source_hash.as_str())) {
            tracing::warn!("source_hash write failed for {id}: {e}");
        }

        // Timeline: record creation (#347). This hook lives in the single
        // shared creation path so every remember() / remember_typed() /
        // remember_precomputed() / consolidate() call records a Created
        // event. Best-effort, never fails the insert.
        self.try_timeline_event(&id, crate::timeline::TimelineEventType::Created, None);

        // Auto-wire edges for the new memory (v8, #346).
        // Pattern-based extraction — best-effort, never fails the insert.
        self.wire_edges(
            &id,
            content,
            tags,
            &memory.metadata,
            Some(memory.namespace.as_str()),
        );

        // Invalidate recall cache — new memory may affect future queries
        self.recall_cache.invalidate_namespace(&memory.namespace);

        // #1166: empty embedding = "no embedder configured" — store the row
        // (already committed to SQLite above) without a vector entry. The
        // row stays FTS5-searchable; `uteke repair` can backfill vectors
        // once an embedder is available.
        if !embedding.is_empty() {
            index.insert(&id, embedding)?;
        }
        // Retry index persistence up to 3 times (#621).
        // A failed save means the in-memory index has the entry but
        // on-disk doesn't → silent desync on next process launch.
        for attempt in 0..3 {
            match index.save() {
                Ok(()) => break,
                Err(e) => {
                    if attempt < 2 {
                        tracing::warn!(
                            "Index save attempt {}/3 failed after remember for id={id}: {e}. Retrying...",
                            attempt + 1
                        );
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    } else {
                        tracing::warn!(
                            "Failed to persist vector index after 3 attempts for remember id={id}: {e}. \
                             Index entry can be rebuilt via `uteke repair`."
                        );
                    }
                }
            }
        }
        // Drop the write lock BEFORE auto_link_cosine to prevent deadlock.
        // auto_link_cosine needs a read lock on the same index — holding
        // the write lock here would deadlock (#442).
        drop(index);

        // Cosine-similarity auto-linking (#401).
        // Must run AFTER index.insert() so the new memory is searchable.
        // Best-effort: errors logged, never fails remember().
        // #1166: cosine auto-linking needs a real embedding; skip when the
        // row was stored FTS5-only (no embedder configured).
        if !embedding.is_empty() {
            self.auto_link_cosine(&id, embedding, Some(memory.namespace.as_str()));
        }

        Ok(id)
    }

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
        self.recall_inner(
            query,
            limit,
            tags_filter,
            namespace,
            min_score,
            entity_filter,
            category_filter,
            false,
        )
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

        // Touch access for returned results
        let touch_ids: Vec<&str> = results.iter().map(|r| r.memory.id.as_str()).collect();
        self.store.touch_access_batch(&touch_ids).ok();

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
        let cache_ns = namespace.unwrap_or("all");
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
            RecallStrategy::Vector => {
                self.recall(query, boost_window, tags_filter, namespace, 0.0, None, None)
            }
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
                return self.recall(query, limit, tags_filter, namespace, min_score, None, None);
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

        // Touch access for returned results
        let touch_ids: Vec<&str> = results.iter().map(|r| r.memory.id.as_str()).collect();
        self.store.touch_access_batch(&touch_ids).ok();

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
        let vector_results =
            match self.recall(query, limit * 3, tags_filter, namespace, 0.0, None, None) {
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

        // Touch access for returned results
        let touch_ids: Vec<&str> = results.iter().map(|r| r.memory.id.as_str()).collect();
        self.store.touch_access_batch(&touch_ids).ok();

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
        self.store.touch_access_batch(&touch_ids).ok();

        Ok(results)
    }

    /// Delete a memory by ID. Incremental — no index rebuild.
    ///
    /// Holds the index write lock for the entire operation to prevent
    /// concurrent processes from reading a partially-updated index (#621).
    pub fn forget(&self, id: &str) -> Result<(), Error> {
        // When soft_delete_only is enabled (default), redirect to soft-delete (#932).
        if self.lifecycle_config.soft_delete_only {
            return self.soft_forget(id, "user forget() — soft-delete via lifecycle config");
        }

        // --- Legacy hard-delete path (only when soft_delete_only=false) ---
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
        // SQLite delete (source of truth).
        // Check the return value: Ok(false) means the ID was not found in the DB (#926).
        let deleted = self.store.delete(id)?;
        if !deleted {
            return Err(Error::db_msg(format!(
                "Memory with id='{id}' not found in store. Nothing was deleted."
            )));
        }
        // Vector index remove — orphan is harmless if fails (verify/repair cleans up)
        if !index.remove(id) {
            tracing::warn!("Vector index entry not found during forget for id={id}");
        }
        // Retry index persistence up to 3 times (#621).
        // A failed save leaves orphan entries that desync from SQLite.
        let mut last_err = None;
        for attempt in 0..3 {
            match index.save() {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 2 {
                        tracing::warn!(
                            "Index save attempt {}/3 failed after forget for id={id}: {}. Retrying...",
                            attempt + 1,
                            last_err.as_ref().unwrap()
                        );
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            }
        }

        // All retries exhausted: SQLite row is deleted but index persistence failed (#926).
        // Return Err so the caller knows the operation was only partially successful.
        // The DB is the source of truth; `repair` will resync the index.
        tracing::error!(
            "Failed to persist vector index after 3 attempts for forget id={id}. \
             SQLite row deleted but index is stale. Run `uteke repair` to resync."
        );
        Err(Error::embed_msg(format!(
            "Memory {id} was deleted from the database, but the vector index \
             could not be saved after 3 attempts: {}. \
             Run `uteke repair` to resync the index.",
            last_err.unwrap()
        )))
    }

    /// Soft-delete (deprecate) a memory with reason (#929).
    ///
    /// When `soft_delete_only` is enabled (default), this replaces `forget()`.
    /// The memory is marked deprecated but remains in the database,
    /// hidden from recall, and restorable via `promote()`.
    pub fn soft_forget(&self, id: &str, reason: &str) -> Result<(), Error> {
        self.store.deprecate_with_reason(id, reason)?;
        // Remove from vector index so it doesn't appear in recall.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during soft_forget"))?;
        if !index.remove(id) {
            tracing::debug!(
                "Vector index entry not found during soft_forget for id={id} (ok if never embedded)"
            );
        }
        // Persist vector index to disk so the removal survives restart.
        if let Err(e) = index.save() {
            tracing::warn!("Failed to persist vector index after soft_forget: {e}");
        }
        // Invalidate recall cache for this memory's namespace.
        if let Some(memory) = self.store.get_by_id(id).ok().flatten() {
            self.recall_cache.invalidate_namespace(&memory.namespace);
        }
        tracing::info!("Soft-deleted memory id={id}: {reason}");
        Ok(())
    }

    /// Restore a deprecated memory to active state (#929).
    ///
    /// Re-adds the memory to the vector index and clears the deprecated flag.
    /// Returns false if the memory was not deprecated or doesn't exist.
    pub fn promote(&self, id: &str) -> Result<bool, Error> {
        let restored = self.store.undeprecate(id)?;
        if !restored {
            return Ok(false);
        }
        // Re-add to vector index.
        if let Some(memory) = self.store.get_by_id(id).ok().flatten() {
            if !memory.embedding.is_empty() {
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during promote"))?;
                if let Err(e) = index.insert(&memory.id, &memory.embedding) {
                    tracing::warn!(
                        "Failed to re-insert memory id={} into vector index during promote: {e}",
                        memory.id
                    );
                }
                let _ = index.save();
            }
            self.recall_cache.invalidate_namespace(&memory.namespace);
        }
        tracing::info!("Promoted (restored) memory id={id}");
        Ok(true)
    }

    /// Bulk delete memories by tag. Hard or soft-delete based on lifecycle config (#932).
    pub fn bulk_forget_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<BulkDeleteResult, Error> {
        if self.lifecycle_config.soft_delete_only {
            let ids = self.store.find_ids_by_tag(tag, namespace)?;
            let reason = format!("bulk forget by tag='{tag}' — soft-delete via lifecycle config");
            let count = self.store.deprecate_by_ids(&ids, &reason)?;
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during bulk_forget_by_tag"))?;
            for id in &ids {
                index.remove(id);
            }
            persist_index_after_delete(&mut index, "bulk_forget_by_tag (soft)")?;
            if let Some(ns) = namespace {
                self.recall_cache.invalidate_namespace(ns);
            } else {
                self.recall_cache.clear();
            }
            return Ok(BulkDeleteResult {
                deleted: count,
                ids,
            });
        }

        // Legacy hard-delete path
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
        persist_index_after_delete(&mut index, "bulk_forget_by_tag")?;
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// Bulk delete all cold memories. Hard or soft-delete based on lifecycle config (#932).
    pub fn bulk_forget_cold(&self, namespace: Option<&str>) -> Result<BulkDeleteResult, Error> {
        if self.lifecycle_config.soft_delete_only {
            let ids = self
                .store
                .find_ids_cold(namespace, self.tier_config.warm_days)?;
            let reason = "bulk forget cold — soft-delete via lifecycle config".to_string();
            let count = self.store.deprecate_by_ids(&ids, &reason)?;
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during bulk_forget_cold"))?;
            for id in &ids {
                index.remove(id);
            }
            persist_index_after_delete(&mut index, "bulk_forget_cold (soft)")?;
            if let Some(ns) = namespace {
                self.recall_cache.invalidate_namespace(ns);
            } else {
                self.recall_cache.clear();
            }
            return Ok(BulkDeleteResult {
                deleted: count,
                ids,
            });
        }

        // Legacy hard-delete path
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
        persist_index_after_delete(&mut index, "bulk_forget_cold")?;
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// Bulk delete all memories in a namespace. Hard or soft-delete based on lifecycle config (#932).
    pub fn bulk_forget_all(&self, namespace: Option<&str>) -> Result<BulkDeleteResult, Error> {
        if self.lifecycle_config.soft_delete_only {
            let ids = self.store.find_ids_all(namespace)?;
            let reason = "bulk forget all — soft-delete via lifecycle config".to_string();
            let count = self.store.deprecate_by_ids(&ids, &reason)?;
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during bulk_forget_all"))?;
            for id in &ids {
                index.remove(id);
            }
            persist_index_after_delete(&mut index, "bulk_forget_all (soft)")?;
            if let Some(ns) = namespace {
                self.recall_cache.invalidate_namespace(ns);
            } else {
                self.recall_cache.clear();
            }
            return Ok(BulkDeleteResult {
                deleted: count,
                ids,
            });
        }

        // Legacy hard-delete path
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
        persist_index_after_delete(&mut index, "bulk_forget_all")?;
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

    /// List all namespaces with memory counts (#527).
    ///
    /// Returns `[(namespace, count)]` — e.g. `[("default", 432), ("cto", 28)]`.
    pub fn list_namespaces_with_counts(&self) -> Result<Vec<(String, usize)>, Error> {
        self.store.list_namespaces_with_counts()
    }

    /// List all namespaces with counts split by lifecycle state (#1181).
    ///
    /// Returns `[(namespace, active, deprecated)]` so clients can show honest
    /// breakdowns — deprecated-only "ghost" namespaces look identical to
    /// active ones in plain counts.
    pub fn list_namespaces_with_lifecycle_counts(
        &self,
    ) -> Result<Vec<(String, usize, usize)>, Error> {
        self.store.list_namespaces_with_lifecycle_counts()
    }

    /// Move a single memory to another namespace (#1181).
    ///
    /// Returns `Ok(false)` when the memory does not exist. Namespace is a
    /// plain column and embeddings are content-based, so no re-embed happens.
    pub fn move_memory(&self, id: &str, namespace: &str) -> Result<bool, Error> {
        Self::validate_namespace_name(namespace)?;
        let existing = match self.store.get_by_id(id)? {
            Some(memory) => memory,
            None => return Ok(false),
        };
        let moved =
            self.store
                .move_memory_namespace(id, namespace, &chrono::Utc::now().to_rfc3339())?;
        if moved {
            self.recall_cache.invalidate_namespace(&existing.namespace);
            self.recall_cache.invalidate_namespace(namespace);
        }
        Ok(moved)
    }

    /// Rename a namespace, merging into the target when it exists (#1181).
    ///
    /// Single atomic `UPDATE` — the old name vanishes naturally because
    /// namespaces are a derived view over `memories.namespace`.
    pub fn rename_namespace(&self, from: &str, to: &str) -> Result<NamespaceRenameResult, Error> {
        Self::validate_namespace_name(from)?;
        Self::validate_namespace_name(to)?;
        if from == to {
            return Err(Error::Validation(
                "Rename source and target namespaces are identical".to_string(),
            ));
        }
        let (active, deprecated, total) = self.store.namespace_lifecycle_counts(from)?;
        if total == 0 {
            return Err(Error::Validation(format!("Namespace not found: {from}")));
        }
        let target_existed = self.store.namespace_lifecycle_counts(to)?.2 > 0;
        let moved = self
            .store
            .rename_namespace(from, to, &chrono::Utc::now().to_rfc3339())?;
        self.recall_cache.invalidate_namespace(from);
        self.recall_cache.invalidate_namespace(to);
        tracing::info!(
            "Namespace rename/merge: '{from}' -> '{to}' moved {moved} memories \
             ({active} active, {deprecated} deprecated, target_existed={target_existed})"
        );
        Ok(NamespaceRenameResult {
            from: from.to_string(),
            to: to.to_string(),
            moved,
            target_existed,
        })
    }

    /// Delete a namespace with an explicit strategy for its memories (#1181).
    ///
    /// Namespaces are derived — the name only disappears once no memory
    /// references it, so deletion must first decide the fate of its memories:
    /// - `refuse`: fail while any memory (incl. deprecated) uses the name.
    /// - `merge`: move all memories into `target` (the name disappears).
    /// - `deprecate`: soft-delete all memories (recycle bin + TTL) — the name
    ///   stays visible as a deprecated-only ghost with honest counts.
    ///
    /// There is no hard-delete path, consistent with the lifecycle design.
    pub fn delete_namespace(
        &self,
        name: &str,
        strategy: &str,
        target: Option<&str>,
    ) -> Result<NamespaceDeleteResult, Error> {
        Self::validate_namespace_name(name)?;
        match strategy {
            "refuse" | "merge" | "deprecate" => {}
            other => {
                return Err(Error::Validation(format!(
                    "Unknown delete strategy '{other}'. Expected: refuse, merge, deprecate"
                )));
            }
        }
        let (active, deprecated, total) = self.store.namespace_lifecycle_counts(name)?;
        let no_op = |empty: bool| NamespaceDeleteResult {
            name: name.to_string(),
            strategy: strategy.to_string(),
            affected: 0,
            target: target.map(str::to_string),
            empty,
        };
        if total == 0 {
            // Namespace is empty / already gone — idempotent no-op.
            return Ok(no_op(true));
        }
        match strategy {
            "merge" => {
                let target = target.ok_or_else(|| {
                    Error::Validation("strategy=merge requires a 'target' namespace".to_string())
                })?;
                Self::validate_namespace_name(target)?;
                if target == name {
                    return Err(Error::Validation(
                        "Merge target must differ from the deleted namespace".to_string(),
                    ));
                }
                let moved =
                    self.store
                        .rename_namespace(name, target, &chrono::Utc::now().to_rfc3339())?;
                self.recall_cache.invalidate_namespace(name);
                self.recall_cache.invalidate_namespace(target);
                tracing::info!(
                    "Namespace delete (merge): '{name}' -> '{target}' moved {moved} memories"
                );
                Ok(NamespaceDeleteResult {
                    name: name.to_string(),
                    strategy: strategy.to_string(),
                    affected: moved,
                    target: Some(target.to_string()),
                    empty: true,
                })
            }
            "deprecate" => {
                let reason = format!("namespace delete (strategy=deprecate) of '{name}' (#1181)");
                let affected = self.store.deprecate_by_namespace(name, &reason)?;
                let ids = self.store.namespace_ids(name)?;
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during delete_namespace"))?;
                for id in &ids {
                    index.remove(id);
                }
                persist_index_after_delete(&mut index, "delete_namespace (deprecate)")?;
                self.recall_cache.invalidate_namespace(name);
                tracing::info!(
                    "Namespace delete (deprecate): '{name}' soft-deleted {affected} memories \
                     ({deprecated} were already deprecated)"
                );
                Ok(NamespaceDeleteResult {
                    name: name.to_string(),
                    strategy: strategy.to_string(),
                    affected,
                    target: None,
                    // The name survives as a deprecated-only ghost listing.
                    empty: false,
                })
            }
            // `refuse` — validated above; total > 0 always refuses.
            _ => Err(Error::Validation(format!(
                "Namespace '{name}' still holds {total} memory(ies) \
                 ({active} active, {deprecated} deprecated); refusing to delete. \
                 Use strategy=merge (with target) or strategy=deprecate."
            ))),
        }
    }

    /// Shared namespace name validation (#1181).
    fn validate_namespace_name(name: &str) -> Result<(), Error> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(Error::Validation(
                "Namespace name must not be empty".to_string(),
            ));
        }
        if trimmed.len() > 128 {
            return Err(Error::Validation(format!(
                "Namespace name too long ({} > 128 chars)",
                trimmed.len()
            )));
        }
        Ok(())
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
        let renamed = self.store.rename_tag(old, new, namespace)?;
        if renamed > 0 {
            // Invalidate recall cache — tag filters may reference the old
            // tag name, so cached results are now stale (#849).
            if let Some(ns) = namespace {
                self.recall_cache.invalidate_namespace(ns);
            } else {
                self.recall_cache.clear();
            }
        }
        Ok(renamed)
    }

    /// Delete a tag from all memories in a namespace.
    pub fn delete_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let deleted = self.store.delete_tag(tag, namespace)?;
        if deleted > 0 {
            // Invalidate recall cache to prevent stale search results (#844).
            // If namespace is specified, invalidate only that namespace.
            // Cross-namespace delete requires full cache flush.
            if let Some(ns) = namespace {
                self.recall_cache.invalidate_namespace(ns);
            } else {
                self.recall_cache.clear();
            }
        }
        Ok(deleted)
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

    /// Batch-fetch multiple memories by ID (eliminates N+1 in graph traversal).
    pub fn get_by_ids(&self, ids: &[&str]) -> Result<Vec<Memory>, Error> {
        self.store.get_by_ids(ids)
    }

    /// Resolve a short ID prefix (first 8 chars) to a full memory ID (#794).
    ///
    /// Returns `Ok(Some(id))` for exact match, `Ok(None)` if not found,
    /// or `Err(Validation)` if ambiguous.
    pub fn resolve_id_prefix(&self, prefix: &str) -> Result<Option<String>, Error> {
        self.store.resolve_id_prefix(prefix)
    }

    /// Update an existing memory with partial fields (#659).
    ///
    /// Only provided fields are changed. If `content` is changed, the
    /// embedding is regenerated and the vector index is updated.
    /// Returns `Ok(true)` if the memory was found and updated,
    /// `Ok(false)` if the memory ID doesn't exist.
    ///
    /// Acceptance criteria:
    /// - Partial update semantics (only provided fields changed)
    /// - Content update regenerates embedding
    /// - 404 if not found (caller checks return value)
    #[allow(clippy::too_many_arguments)]
    pub fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
        metadata: Option<&serde_json::Value>,
        importance: Option<f64>,
        pinned: Option<bool>,
        memory_type: Option<&str>,
    ) -> Result<bool, Error> {
        // Validate memory exists
        let existing = self
            .store
            .get_by_id(id)?
            .ok_or_else(|| Error::Validation(format!("Memory not found: {id}")))?;

        // Validate new content if provided
        if let Some(c) = content {
            crate::validate_input(c, &[] as &[&str])?;
        }

        // Validate new tags if provided
        if let Some(t) = tags {
            let tag_refs: Vec<&str> = t.iter().map(|s| s.as_str()).collect();
            crate::validate_input(content.unwrap_or(&existing.content), &tag_refs)?;
        }

        // Validate new memory_type if provided
        if let Some(mt) = memory_type {
            crate::memory::types::MemoryType::from_str_opt(mt).ok_or_else(|| {
                Error::Validation(format!(
                    "Unknown memory type '{mt}'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
                ))
            })?;
        }

        // Validate importance range
        if let Some(imp) = importance {
            if !(0.0..=1.0).contains(&imp) {
                return Err(Error::Validation(format!(
                    "Importance must be between 0.0 and 1.0, got {imp}"
                )));
            }
        }

        let now = chrono::Utc::now();

        // Re-embed if content changed
        if let Some(c) = content {
            let content_type = crate::memory::crud::detect_content_type(c);
            let embed_text = if content_type == "json" {
                crate::memory::crud::flatten_json_for_embedding(c)
            } else {
                c.to_string()
            };
            self.ensure_embedder()?;
            let new_embedding = retry_embed(&self.embedder, &embed_text)?;

            // Update SQLite first, then vector index
            let updated = self.store.update_fields(
                id,
                content,
                tags,
                metadata,
                importance,
                pinned,
                memory_type,
                now,
            )?;

            if !updated {
                return Ok(false);
            }

            // Update vector index (insert handles dedup by removing old entry)
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during update_memory"))?;
            index.insert(id, &new_embedding)?;

            // Persist index with retry
            for attempt in 0..3 {
                match index.save() {
                    Ok(()) => break,
                    Err(e) => {
                        if attempt < 2 {
                            tracing::warn!(
                                "Index save attempt {}/3 failed after update_memory for id={id}: {e}. Retrying...",
                                attempt + 1
                            );
                            std::thread::sleep(std::time::Duration::from_millis(200));
                        } else {
                            tracing::error!(
                                "Index save failed after 3 attempts for id={id}: {e}. Index may be stale on next launch."
                            );
                        }
                    }
                }
            }
        } else {
            // No content change — just update SQLite fields
            let updated = self.store.update_fields(
                id,
                None,
                tags,
                metadata,
                importance,
                pinned,
                memory_type,
                now,
            )?;
            if !updated {
                return Ok(false);
            }
        }

        // Invalidate recall cache for the memory's namespace
        self.recall_cache.invalidate_namespace(&existing.namespace);

        Ok(true)
    }

    /// Recall memories that existed at a specific point in time.
    ///
    /// Runs a semantic recall to gather candidates, then post-filters by
    /// temporal validity at `point_in_time`:
    /// - `created_at <= point_in_time`
    /// - `valid_until IS NULL OR valid_until > point_in_time`
    /// - `valid_from IS NULL OR valid_from <= point_in_time`
    /// - `deprecated = false`
    #[allow(clippy::too_many_arguments)]
    pub fn recall_at_time(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        point_in_time: chrono::DateTime<chrono::Utc>,
        min_score: f32,
        entity_filter: Option<&str>,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, Error> {
        // Retry loop: over-fetch with increasing multipliers to compensate
        // for temporal filtering removing candidates. If post-filtering
        // yields fewer than `limit` results, expand the search scope.
        let mut multiplier = 3usize;
        let mut attempt = 0;

        loop {
            let fetch_limit = (limit * multiplier).max(50);
            let candidates = self.recall_inner(
                query,
                fetch_limit,
                tags_filter,
                namespace,
                min_score,
                entity_filter,
                category_filter,
                true,
            )?;
            let candidates_len = candidates.len();

            let mut results: Vec<SearchResult> = candidates
                .into_iter()
                .filter(|r| memory_existed_at(&r.memory, point_in_time))
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

/// Persist the vector index to disk after a delete operation.
///
/// Retries up to 3 times with 200ms backoff. Unlike the old per-call inline
/// code, this helper **returns an error** on persistent failure so the caller
/// can surface it to the user instead of silently swallowing it (#926).
///
/// The database is always the source of truth: if the index save fails, the
/// rows are already gone from SQLite. `uteke repair` will resync the index.
fn persist_index_after_delete(
    index: &mut crate::memory::vector::VectorIndex,
    context: &str,
) -> Result<(), Error> {
    let mut last_err = None;
    for attempt in 0..3 {
        match index.save() {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt < 2 {
                    tracing::warn!(
                        "Index save attempt {}/3 failed after {context}: {}. Retrying...",
                        attempt + 1,
                        last_err.as_ref().unwrap()
                    );
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
    }

    tracing::error!(
        "Failed to persist vector index after 3 attempts in {context}. \
         SQLite rows deleted but index is stale. Run `uteke repair` to resync."
    );
    Err(Error::embed_msg(format!(
        "Vector index could not be saved after 3 attempts in {context}: {}. \
         Database rows were deleted. Run `uteke repair` to resync the index.",
        last_err.unwrap()
    )))
}

#[cfg(test)]
mod forget_tests {
    use crate::Uteke;

    /// Insert a memory with a fake embedding (bypassing ONNX), then forget it.
    /// Verifies the memory is soft-deleted (deprecated) by default (#926, #932).
    #[test]
    fn test_forget_deletes_memory() {
        let uteke = Uteke::open(":memory:").unwrap();
        let embedding = vec![0.1_f32; 768]; // matches DEFAULT_DIMS
        let id = uteke
            .remember_precomputed(
                "test forget content",
                &["test-forget"],
                None,
                Some("forget-test"),
                "fact",
                "text",
                &embedding,
            )
            .unwrap();

        // Verify it exists and is active
        let mem = uteke.get_by_id(&id).unwrap().unwrap();
        assert!(!mem.deprecated, "memory should be active before forget");

        // Forget it (soft-delete by default: soft_delete_only=true)
        uteke.forget(&id).unwrap();

        // Memory still exists but is now deprecated (soft-delete)
        let mem = uteke.get_by_id(&id).unwrap().unwrap();
        assert!(mem.deprecated, "memory should be deprecated after forget");
    }

    /// Forgetting a non-existent ID should return an error, not Ok(()) (#926).
    #[test]
    fn test_forget_nonexistent_returns_error() {
        let uteke = Uteke::open(":memory:").unwrap();
        let result = uteke.forget("nonexistent-id-12345");
        assert!(
            result.is_err(),
            "forget on non-existent ID should return Err, not Ok(())"
        );
    }

    /// Forget → re-insert should work (soft-deleted memory stays, new ID for re-insert) (#926).
    #[test]
    fn test_forget_then_reinsert() {
        let uteke = Uteke::open(":memory:").unwrap();
        let embedding = vec![0.5_f32; 768]; // matches DEFAULT_DIMS
        let id = uteke
            .remember_precomputed(
                "forget then reinsert",
                &[],
                None,
                Some("reinsert-test"),
                "fact",
                "text",
                &embedding,
            )
            .unwrap();

        // Soft-delete: memory becomes deprecated, not removed
        uteke.forget(&id).unwrap();
        let mem = uteke.get_by_id(&id).unwrap().unwrap();
        assert!(mem.deprecated, "original memory should be deprecated");

        // Re-insert with same content: should get a new ID (deprecated memories don't block re-insert)
        let id2 = uteke
            .remember_precomputed(
                "forget then reinsert",
                &[],
                None,
                Some("reinsert-test"),
                "fact",
                "text",
                &embedding,
            )
            .unwrap();
        assert_ne!(id, id2, "re-inserted memory should have a new ID");
        let mem2 = uteke.get_by_id(&id2).unwrap().unwrap();
        assert!(!mem2.deprecated, "new memory should be active");
    }

    /// #1047: after soft-forget, the deprecated row must vanish from list(),
    /// load_all(), and the doctor/verify count — otherwise list shows ghosts
    /// and doctor reports DB/Index mismatch forever.
    #[test]
    fn test_soft_forget_hides_from_list_and_doctor() {
        // Isolated temp-dir store: ":memory:" stores still resolve the vector
        // index to a file in the CWD, which cross-contaminates parallel runs.
        let dir = std::env::temp_dir().join(format!("ghost-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();
        let embedding = vec![0.7_f32; 768];
        let id = uteke
            .remember_precomputed(
                "ghost row after soft forget",
                &[],
                None,
                Some("ghost-test"),
                "fact",
                "text",
                &embedding,
            )
            .unwrap();

        uteke.forget(&id).unwrap();

        // list() must not return the deprecated row
        let listed = uteke.list(None, 100, 0, Some("ghost-test")).unwrap();
        assert!(
            listed.iter().all(|m| m.id != id),
            "deprecated row must not appear in list()"
        );

        // store-level list filter
        let rows = uteke.store.list(None, Some("ghost-test"), 100, 0).unwrap();
        assert!(rows.iter().all(|m| m.id != id));

        // load_all() (repair/verify source) must exclude it
        let all = uteke.store.load_all(Some("ghost-test")).unwrap();
        assert!(all.iter().all(|m| m.id != id));

        // doctor: DB count (active-only now) must equal index count
        let report = uteke.doctor().unwrap();
        let consistency = report
            .checks
            .iter()
            .find(|c| c.name == "Index consistency")
            .expect("doctor reports index consistency");
        assert!(
            !consistency.detail.contains("MISMATCH"),
            "doctor must not report mismatch after soft-forget, got: {}",
            consistency.detail
        );
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod recall_cache_parity_tests {
    use crate::RecallStrategy;
    use crate::Uteke;

    /// #1037: cold (cache miss) and warm (cache hit) recall_hybrid calls must
    /// return identical scores. The cache-hit path used to skip salience/
    /// recency boosts, so warm results scored lower than cold results.
    #[test]
    fn test_recall_hybrid_cold_warm_score_parity() {
        let dir = std::env::temp_dir().join(format!("parity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();

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
        let mut uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();
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
        let mut uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();
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

/// Fusion weights benchmark-tuned on LongMemEval fast50 (#1123):
/// plateau [1.7, 1.9] → R@5 0.98; 1.7 chosen mid-plateau.
/// k=60 matches the k used by recall_rrf.
pub(crate) const FUSION_W_VECTOR: f64 = 1.7;
pub(crate) const FUSION_W_HYBRID: f64 = 1.0;
pub(crate) const FUSION_RRF_K: f64 = 60.0;

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
    use crate::Uteke;
    use crate::memory::types::RecallStrategy;

    /// #1123: Fusion strategy end-to-end — seeds a store, runs recall with
    /// the explicit Fusion strategy, and asserts the on-topic memory surfaces
    /// in the top results. Also asserts cold/warm parity through the cache.
    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn fusion_recall_finds_on_topic_memory() {
        let dir = std::env::temp_dir().join(format!("fusion-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();

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
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn fusion_recall_empty_store_returns_empty() {
        let dir = std::env::temp_dir().join(format!("fusion-e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();
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
mod dedup_tests {
    use super::memory_existed_at;
    use crate::Uteke;

    fn probe_memory(created: chrono::DateTime<chrono::Utc>) -> crate::memory::types::Memory {
        crate::memory::types::Memory {
            id: "probe-1086".to_string(),
            content: "temporal deprecation probe alpha".to_string(),
            embedding: vec![],
            tags: vec![],
            metadata: serde_json::json!({}),
            created_at: created,
            updated_at: created,
            namespace: crate::memory::types::DEFAULT_NAMESPACE.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: Some(created),
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "direct".to_string(),
            author_type: "agent".to_string(),
        }
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn test_dedup_blocks_exact_duplicate() {
        let uteke = Uteke::open(":memory:").unwrap();
        let id1 = uteke
            .remember("The sky is blue today", &[], None, Some("dedup"))
            .unwrap();
        // Same content again — should return the SAME id, not a new one.
        let id2 = uteke
            .remember("The sky is blue today", &[], None, Some("dedup"))
            .unwrap();
        assert_eq!(id1, id2, "exact duplicate should return existing ID");
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn test_dedup_allows_different_content() {
        let uteke = Uteke::open(":memory:").unwrap();
        let id1 = uteke
            .remember("The sky is blue", &[], None, Some("dedup2"))
            .unwrap();
        let id2 = uteke
            .remember("Rust is a programming language", &[], None, Some("dedup2"))
            .unwrap();
        assert_ne!(id1, id2, "different content should create new memory");
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn test_dedup_namespace_scoped() {
        let uteke = Uteke::open(":memory:").unwrap();
        let id1 = uteke
            .remember("Same content different namespace", &[], None, Some("ns1"))
            .unwrap();
        // Same content in DIFFERENT namespace — should NOT be blocked.
        let id2 = uteke
            .remember("Same content different namespace", &[], None, Some("ns2"))
            .unwrap();
        assert_ne!(id1, id2, "different namespace should not dedup");
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn test_dedup_skips_deprecated_stale_index_entry() {
        // #1210: a stale vector index on a long-running server can still hold
        // the vector of a soft-deleted (deprecated) memory. Dedup must NOT
        // match it — the rewrite must produce a NEW live memory instead of
        // silently returning the deprecated ghost id (which also gets linked
        // into rooms by remember_in_room, invisible in recall).
        let uteke = Uteke::open(":memory:").unwrap();
        let ns = "dedup-depr";
        let content = "Ghost dedup regression content #1210";
        let id1 = uteke.remember(content, &[], None, Some(ns)).unwrap();

        // Healthy path: soft-delete removes the vector from the index.
        uteke.soft_forget(&id1, "test: 1210 repro").unwrap();

        // Simulate index/SQLite desync (missed repair / failed persistence):
        // the deprecated row's vector re-enters the index.
        let emb = uteke.store.get_by_id(&id1).unwrap().unwrap().embedding;
        assert!(!emb.is_empty(), "precondition: first row must be embedded");
        uteke.index.write().unwrap().insert(&id1, &emb).unwrap();

        // The rewrite must NOT dedup onto the deprecated ghost.
        let id2 = uteke.remember(content, &[], None, Some(ns)).unwrap();
        assert_ne!(id2, id1, "deprecated row must not be a dedup target");

        // The new row is live, holds the content, and the ghost stays hidden.
        let live = uteke.store.get_by_id(&id2).unwrap().unwrap();
        assert!(!live.deprecated);
        assert_eq!(live.content, content);
        let ghost = uteke.store.get_by_id(&id1).unwrap().unwrap();
        assert!(ghost.deprecated);
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn test_contradiction_remembers_metadata() {
        // Regression: remember_with_contradiction must pass metadata through
        // to remember_precomputed (was dropping it as None).
        let uteke = Uteke::open(":memory:").unwrap();
        let meta = Some(serde_json::json!({
            "entity": "test-app",
            "category": "integration"
        }));
        let (id, _contradiction) = uteke
            .remember_with_contradiction(
                "Contradiction metadata test content",
                &[],
                meta,
                Some("meta-test"),
                None,
                true,
                0.65,
            )
            .unwrap();
        // Retrieve and verify metadata was stored
        let memory = uteke
            .get_by_id(&id)
            .expect("get_by_id should not error")
            .expect("memory should exist");
        let obj = memory
            .metadata
            .as_object()
            .expect("metadata should be object");
        assert_eq!(obj.get("entity").unwrap(), "test-app");
        assert_eq!(obj.get("category").unwrap(), "integration");
    }

    /// #1086: a memory deprecated AFTER the point-in-time must still appear in
    /// time-travel recall; one deprecated BEFORE it must not.
    #[test]
    fn test_recall_at_time_deprecated_after_pit() {
        let past = chrono::Utc::now() - chrono::Duration::hours(2);
        let now = chrono::Utc::now();

        // Deprecated after `past` -> existed at `past`.
        let mut m = probe_memory(past);
        m.deprecated = true;
        m.deprecated_at = Some(now);
        assert!(memory_existed_at(&m, past));
        assert!(!memory_existed_at(&m, now));
    }
}

/// Temporal predicate shared by `recall_at_time` (core) and
/// `memory_exists_at` (server): did this memory exist at `pit`?
/// Deprecated-before-pit excludes; deprecated-after-pit includes (#1086).
pub fn memory_existed_at(
    m: &crate::memory::types::Memory,
    pit: chrono::DateTime<chrono::Utc>,
) -> bool {
    if m.created_at > pit {
        return false;
    }
    if let Some(valid_until) = m.valid_until {
        if valid_until <= pit {
            return false;
        }
    }
    if m.deprecated {
        match m.deprecated_at {
            Some(dep_at) if dep_at <= pit => return false,
            _ => {}
        }
    }
    if let Some(valid_from) = m.valid_from {
        if valid_from > pit {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod namespace_management_tests {
    use crate::Uteke;

    /// #1181: move_memory updates the namespace column and both the old and
    /// the new namespace disappear/appear correctly in listings.
    #[test]
    fn move_memory_updates_namespace() {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        let id = u
            .remember("move me", &[], None, Some("alpha"))
            .expect("remember");

        let moved = u.move_memory(&id, "beta").expect("move");
        assert!(moved);
        let mem = u.get_by_id(&id).expect("get").expect("exists");
        assert_eq!(mem.namespace, "beta");

        let listings = u.list_namespaces().expect("list");
        assert!(
            !listings.contains(&"alpha".to_string()),
            "old name vanishes"
        );
        assert!(listings.contains(&"beta".to_string()));

        // Unknown ID → Ok(false), not an error.
        let missing = u.move_memory("00000000-0000-4000-8000-000000000000", "beta");
        assert!(matches!(missing, Ok(false)));
    }

    /// #1181: rename moves all memories; renaming onto an existing namespace
    /// merges and reports `target_existed = true`.
    #[test]
    fn rename_namespace_moves_and_merges() {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        u.remember("a one", &[], None, Some("old")).expect("a1");
        u.remember("a two", &[], None, Some("old")).expect("a2");
        u.remember("b one", &[], None, Some("new")).expect("b1");

        // Merge path: target exists.
        let merge = u.rename_namespace("old", "new").expect("merge");
        assert!(merge.target_existed);
        assert_eq!(merge.moved, 2);
        assert_eq!(merge.from, "old");
        assert_eq!(merge.to, "new");

        let listings = u.list_namespaces().expect("list");
        assert!(!listings.contains(&"old".to_string()));
        assert!(listings.contains(&"new".to_string()));

        // Plain rename path: target does not exist.
        u.remember("c one", &[], None, Some("temp")).expect("c1");
        let plain = u.rename_namespace("temp", "final").expect("rename");
        assert!(!plain.target_existed);
        assert_eq!(plain.moved, 1);

        // Same-name rename is rejected.
        assert!(u.rename_namespace("final", "final").is_err());
        // Unknown source namespace is rejected.
        assert!(u.rename_namespace("ghost", "anywhere").is_err());
    }

    /// #1181: delete strategies — refuse (default) blocks while memories
    /// remain; merge moves everything away; deprecate soft-deletes without
    /// hard-deleting anything.
    #[test]
    fn delete_namespace_strategies() {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        let id1 = u.remember("d one", &[], None, Some("temp-ns")).expect("d1");
        let id2 = u.remember("d two", &[], None, Some("temp-ns")).expect("d2");

        // refuse (default): blocked while memories exist.
        let refused = u.delete_namespace("temp-ns", "refuse", None);
        assert!(refused.is_err(), "refuse must block a non-empty namespace");

        // merge: moves all memories into the target, name disappears.
        let merged = u
            .delete_namespace("temp-ns", "merge", Some("archive"))
            .expect("merge delete");
        assert_eq!(merged.strategy, "merge");
        assert_eq!(merged.affected, 2);
        assert_eq!(merged.target.as_deref(), Some("archive"));
        assert!(merged.empty);
        let listings = u.list_namespaces().expect("list");
        assert!(!listings.contains(&"temp-ns".to_string()));
        assert!(u.get_by_id(&id1).expect("get").expect("survives").namespace == "archive");

        // deprecate: soft-delete only, no hard delete; the name remains as a
        // deprecated-only ghost with honest lifecycle counts.
        let dep1 = u
            .remember("e one", &[], None, Some("ghost-ns"))
            .expect("e1");
        let dep = u
            .delete_namespace("ghost-ns", "deprecate", None)
            .expect("deprecate delete");
        assert_eq!(dep.strategy, "deprecate");
        assert_eq!(dep.affected, 1);
        assert!(!dep.empty);
        let mem = u.get_by_id(&dep1).expect("get").expect("still stored");
        assert!(mem.deprecated, "memory must be soft-deleted, not removed");
        let lifecycle = u
            .list_namespaces_with_lifecycle_counts()
            .expect("lifecycle counts");
        let ghost = lifecycle
            .iter()
            .find(|(name, _, _)| name == "ghost-ns")
            .expect("ghost namespace stays listed");
        assert_eq!((ghost.1, ghost.2), (0, 1), "0 active / 1 deprecated");
        assert!(
            u.get_by_id(&id2).is_ok(),
            "nothing from the earlier merge path was lost"
        );

        // Unknown strategy is rejected.
        assert!(u.delete_namespace("ghost-ns", "hard-delete", None).is_err());
    }

    /// #1172 Fase 1: remember records a SHA-256 source hash; provenance()
    /// returns the full chain (fields + tier + timeline) and detects content
    /// modified after write via hash mismatch.
    #[test]
    fn provenance_chain_and_source_hash() {
        use sha2::Digest;

        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        let id = u
            .remember("the deploy window is 09:00 WIB", &[], None, Some("ops"))
            .expect("remember");

        let report = u.provenance(&id).expect("provenance").expect("exists");
        assert_eq!(report.id, id);
        assert_eq!(report.namespace, "ops");
        assert_eq!(report.author_type, "agent");
        // Hash at write time must match a live recomputation.
        let expected: String = sha2::Sha256::digest(report.content.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(report.source_hash.as_deref(), Some(expected.as_str()));
        assert_eq!(report.content_hash_now, expected);
        assert!(!report.events.is_empty(), "Created event must be recorded");
        assert!(report.events.iter().any(|e| e.event_type == "created"));

        // Update content without touching source_hash → mismatch detected
        // (this is the tamper-evidence property).
        u.store
            .conn
            .execute(
                "UPDATE memories SET content = 'tampered' WHERE id = ?1",
                rusqlite::params![id],
            )
            .expect("tamper");
        let after = u.provenance(&id).expect("provenance").expect("exists");
        assert_eq!(after.content, "tampered");
        assert_ne!(
            after.source_hash.as_deref(),
            Some(after.content_hash_now.as_str()),
            "post-write modification must break the hash match"
        );

        // Unknown ID → Ok(None).
        assert!(
            u.provenance("00000000-0000-4000-8000-000000000000")
                .unwrap()
                .is_none()
        );
    }

    /// #1172 Fase 1: timeline events carry actor + evidence when written via
    /// the provenance-aware API, and old callers (no provenance) stay working.
    #[test]
    fn timeline_events_carry_provenance() {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        let id = u
            .remember("evidence chain probe", &[], None, None)
            .expect("remember");

        u.store
            .add_timeline_event_with_provenance(
                &id,
                crate::timeline::TimelineEventType::Updated,
                Some(&serde_json::json!({"field": "importance"})),
                Some("agent:cto"),
                Some(&serde_json::json!([{"memory": "other-id", "score": 0.82}])),
            )
            .expect("append provenance event");

        let events = u.timeline(&id, 0).expect("timeline");
        assert_eq!(events.len(), 2, "created + updated");
        let provenance_event = events
            .iter()
            .find(|e| e.event_type == "updated")
            .expect("updated event");
        assert_eq!(provenance_event.actor.as_deref(), Some("agent:cto"));
        let evidence = provenance_event.evidence.as_ref().expect("evidence");
        assert_eq!(evidence[0]["memory"], serde_json::json!("other-id"));

        // Plain events (Created) have no actor — backward compatible shape.
        let created = events.iter().find(|e| e.event_type == "created").unwrap();
        assert!(created.actor.is_none());
        assert!(created.evidence.is_none());
    }
}
