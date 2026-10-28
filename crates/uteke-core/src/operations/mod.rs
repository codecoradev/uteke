//! Core memory operations: remember, recall, search, forget, list, get, tags.

use crate::error::Error;
use crate::index_sync::{self, SyncPolicy};
use crate::memory::types::{
    BulkDeleteResult, DEFAULT_NAMESPACE, Memory, MemoryTier, NamespaceDeleteResult,
    NamespaceRenameResult, RecallStrategy, SearchResult, TagInfo,
};
use crate::memory::vector::cosine_distance_to_similarity;
use std::sync::Mutex;
use std::time::Duration;

mod forget;
mod listing;
mod lookup;
mod tags;
mod time_travel;
mod update;

/// Retry embedding generation with exponential backoff (#621).
///
/// Embedding failures silently drop vector entries, causing the vector
/// index to desync from SQLite. This helper retries up to 3 times with
/// 200ms, 400ms, then 800ms delays between attempts.
///
/// If the embedder slot is empty (it should not be after `ensure_embedder`),
/// returns an embedding error instead of panicking. Validation errors are
/// returned immediately: they can never succeed on retry.
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
        let Some(embedder) = lock.as_ref() else {
            return Err(Error::embed_msg("No embedder configured"));
        };
        match embedder.embed(text) {
            Ok(embedding) => return Ok(embedding),
            Err(e @ Error::Validation(_)) => return Err(e),
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

/// Result of a remember operation with honest embedding-write reporting (#1273).
///
/// `embedding_written` is `false` when the memory row was stored (SQLite +
/// FTS5, keyword-searchable) but the vector entry is NOT durably usable:
/// embedding generation failed after retries, the index insert failed, or
/// index persistence failed after retries. Such rows are recoverable via
/// `uteke repair` / `POST /repair`. `warning` carries a human-readable
/// reason whenever `embedding_written` is `false` due to a FAILURE.
/// Note: a store with no embedder backend configured (#1166) also reports
/// `embedding_written: false` — by configuration, not failure — with no
/// warning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RememberOutcome {
    /// ID of the stored memory.
    pub id: String,
    /// Whether the vector embedding is durably usable for semantic search.
    pub embedding_written: bool,
    /// Failure reason when the embedding could not be persisted.
    pub warning: Option<String>,
    /// Secrets the privacy filter masked in the content of THIS call (#1464): a
    /// kind and a count each, never a value. Empty when nothing was masked. On a
    /// dedup hit nothing new is stored, so the stored memory keeps its own flags.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<crate::privacy::RedactionCount>,
}

/// What an update did (#1464): whether the memory was found and updated, and
/// what the privacy filter masked in the new content, tags or metadata.
#[derive(Debug, Clone, Default)]
pub struct UpdateOutcome {
    pub updated: bool,
    pub redactions: Vec<crate::privacy::RedactionCount>,
}

impl UpdateOutcome {
    /// Whether the privacy filter masked anything in this update.
    pub fn redacted(&self) -> bool {
        !self.redactions.is_empty()
    }

    /// What was masked, for a human (see [`RememberOutcome::redaction_summary`]).
    pub fn redaction_summary(&self) -> Option<String> {
        redaction_summary(&self.redactions)
    }
}

fn redaction_summary(redactions: &[crate::privacy::RedactionCount]) -> Option<String> {
    (!redactions.is_empty()).then(|| {
        redactions
            .iter()
            .map(|r| format!("{} x{}", r.kind, r.count))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

impl RememberOutcome {
    /// Whether the privacy filter masked anything in the stored content.
    pub fn redacted(&self) -> bool {
        !self.redactions.is_empty()
    }

    /// What was masked, for a human: `github-token x1, jwt x2`. `None` when
    /// nothing was.
    pub fn redaction_summary(&self) -> Option<String> {
        redaction_summary(&self.redactions)
    }
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

    /// Store a new memory, reporting embedding status (#1273).
    pub fn remember_detailed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
    ) -> Result<RememberOutcome, Error> {
        self.remember_auto_infer_detailed(content, tags, metadata, namespace, None)
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
        self.remember_typed_detailed(content, tags, metadata, namespace, memory_type)
            .map(|o| o.id)
    }

    /// Store a new memory with explicit type, reporting embedding status (#1273).
    pub fn remember_typed_detailed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
    ) -> Result<RememberOutcome, Error> {
        crate::validate_input(content, tags)?;
        // Validate memory_type against known variants. The type is used
        // as-is — no inference, no override.
        crate::memory::types::MemoryType::from_str_opt(memory_type).ok_or_else(|| {
            Error::Validation(format!(
                "Unknown memory type '{memory_type}'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
            ))
        })?;
        self.remember_embed_detailed(content, tags, metadata, namespace, memory_type)
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
        self.remember_auto_infer_detailed(content, tags, metadata, namespace, explicit_type)
            .map(|o| o.id)
    }

    /// Store a new memory with auto-inferred type, reporting embedding status (#1273).
    pub fn remember_auto_infer_detailed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        explicit_type: Option<&str>,
    ) -> Result<RememberOutcome, Error> {
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
        self.remember_embed_detailed(content, tags, metadata, namespace, &effective_type)
    }

    /// Audit entry for a write whose content the privacy filter changed
    /// (#1464): kinds and counts, never a value. Best-effort like every
    /// timeline event.
    pub(crate) fn record_redaction(&self, id: &str, redactions: &[crate::privacy::RedactionCount]) {
        if redactions.is_empty() {
            return;
        }
        self.try_timeline_event(
            id,
            crate::timeline::TimelineEventType::Redacted,
            Some(&serde_json::json!({ "redactions": redactions })),
        );
    }

    /// Reject a memory type this version does not know.
    pub(crate) fn validate_memory_type(memory_type: &str) -> Result<(), Error> {
        crate::memory::types::MemoryType::from_str_opt(memory_type).ok_or_else(|| {
            Error::Validation(format!(
                "Unknown memory type '{memory_type}'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
            ))
        })?;
        Ok(())
    }

    /// Embed the text of a memory about to be written: JSON content is
    /// flattened first, the embedding is retried (#621), and a backend that
    /// is not configured or keeps failing gives an EMPTY embedding (the row is
    /// stored FTS5-only, #1166) with the reason in the second field (#1273).
    pub(crate) fn embed_for_write(
        &self,
        content: &str,
        content_type: &str,
    ) -> Result<(Vec<f32>, Option<String>), Error> {
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
        // #1273: embedding GENERATION failure is also a store-and-warn case —
        // the row is kept (SQLite + FTS5) and the outcome carries the reason.
        Ok(if self.embedder_backend.is_empty() {
            tracing::debug!("No embedding backend configured; storing memory FTS5-only (#1166)");
            (Vec::new(), None)
        } else {
            // Failing to initialise the embedder is an error; failing to generate
            // a vector, after the retry in `embed`, is FTS5-only with a warning
            // (embedding failures silently drop vector entries, #621).
            self.ensure_embedder()?;
            match self.embed(&embed_text) {
                Ok(e) => (e, None),
                Err(e) => {
                    tracing::warn!(
                        "Embedding generation failed after retries ({e}). \
                             Memory stored FTS5-only; run `uteke repair` to backfill. (#1273)"
                    );
                    (
                        Vec::new(),
                        Some(format!("embedding generation failed after retries: {e}")),
                    )
                }
            }
        })
    }

    /// Embed-then-store with honest embedding reporting (#1273).
    fn remember_embed_detailed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
    ) -> Result<RememberOutcome, Error> {
        crate::validate_input(content, tags)?;
        Self::validate_memory_type(memory_type)?;
        // Privacy filter (#1464): before the content is embedded, indexed or
        // stored, so a secret never reaches the vector, FTS5 or the row.
        let prepared = self
            .privacy_config
            .prepare_with_tags(content, tags, metadata)?;
        let content = prepared.content.as_str();
        let metadata = prepared.metadata.clone();
        let tag_owned = prepared.tags.clone();
        let tags: Vec<&str> = tag_owned.iter().map(String::as_str).collect();
        let tags = tags.as_slice();
        let content_type = crate::memory::crud::detect_content_type(content);
        let (embedding, generation_error) = self.embed_for_write(content, content_type)?;

        // Dedup check: if an existing memory has cosine >= 0.95, return it
        // instead of creating a duplicate (#442 enhancement).
        // #1166: skip when no embedding was produced (no embedder) — cosine
        // dedup is meaningless without vectors.
        if !embedding.is_empty() {
            if let Some(existing_id) = self.check_duplicate(&embedding, namespace)? {
                tracing::info!("Dedup: memory {existing_id} is nearly identical, skipping insert");
                return Ok(RememberOutcome {
                    id: existing_id,
                    embedding_written: true,
                    warning: None,
                    redactions: prepared.redactions.clone(),
                });
            }
        }

        let mut outcome = self.remember_precomputed_detailed(
            content,
            tags,
            metadata,
            namespace,
            memory_type,
            content_type,
            &embedding,
            generation_error,
        )?;
        self.record_redaction(&outcome.id, &prepared.redactions);
        outcome.redactions = prepared.redactions;
        Ok(outcome)
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

        // Namespace + liveness filter over the (few) candidates only; the old
        // code loaded every id of the namespace on each remember.
        let candidate_ids: Vec<&str> = results
            .iter()
            .map(|(id, _)| id.as_str())
            .filter(|id| !id.starts_with("chunk:"))
            .collect();
        let live = match self.store.live_ids_in_namespace(&candidate_ids, namespace) {
            Ok(set) => set,
            Err(_) => return Ok(None),
        };

        for (id, dist) in &results {
            // Skip chunk: prefixed entries (document chunks).
            if id.starts_with("chunk:") {
                continue;
            }
            // Namespace + liveness filter.
            if !live.contains(id) {
                continue;
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
    /// Test-only ID convenience wrapper — production callers use
    /// [`Self::remember_precomputed_detailed`] for the honest
    /// `RememberOutcome` (#1273).
    #[cfg(test)]
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
        self.remember_precomputed_detailed(
            content,
            tags,
            metadata,
            namespace,
            memory_type,
            content_type,
            embedding,
            None,
        )
        .map(|o| o.id)
    }

    /// Store a memory whose embedding already exists, with honest embedding
    /// reporting (#1273). SQLite commit + vector insert + index save are
    /// each allowed to fail INDEPENDENTLY without losing the honest result:
    /// the memory row stays stored whenever it was committed, and
    /// `embedding_written: false` + `warning` surface any vector-side loss
    /// instead of a misleading hard error or a silent success.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remember_precomputed_detailed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
        content_type: &str,
        embedding: &[f32],
        embedding_error: Option<String>,
    ) -> Result<RememberOutcome, Error> {
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
            recall_count: 0,
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
            author: None,
        };

        self.commit_memory(&memory, embedding_error)
    }

    /// Everything that happens once a memory has been built and embedded:
    /// the SQLite row, the source hash, the `Created` event, edge wiring,
    /// recall-cache invalidation, the vector, cosine links. The memory
    /// carries its own embedding (empty: stored FTS5-only).
    pub(crate) fn commit_memory(
        &self,
        memory: &Memory,
        embedding_error: Option<String>,
    ) -> Result<RememberOutcome, Error> {
        let id = memory.id.clone();
        let content = memory.content.as_str();
        let embedding = memory.embedding.as_slice();
        let tag_refs: Vec<&str> = memory.tags.iter().map(String::as_str).collect();
        let tags = tag_refs.as_slice();

        // SQLite is the source of truth and is written first, WITHOUT holding the
        // index write lock: the timeline / provenance / edge wiring below issue
        // many queries and would otherwise block every concurrent recall for
        // their whole duration. The vector is mirrored afterwards under a short
        // write lock; if that fails the row stays FTS5-only and the outcome says
        // so (#1273), `uteke repair` can backfill.
        self.store.insert(memory)?;

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
        // Honest vector-write accounting (#1273): a vector insert or index
        // persistence failure must NOT fail the whole remember (the SQLite
        // row is already committed) nor pass as silent success. The outcome
        // carries embedding_written=false + warning so every surface can
        // tell the truth; `uteke repair` can backfill the vector later.
        let mut vector_error = embedding_error;
        // One policy for insert + persist (retry, deterministic errors not
        // retried) lives in `index_sync`; the outcome says what landed (#1273).
        // An empty embedding means "no embedder": nothing to mirror, no lock.
        if !embedding.is_empty() {
            match self.index.write() {
                Ok(mut index) => {
                    let sync =
                        index_sync::upsert(&mut *index, &id, embedding, &SyncPolicy::STANDARD);
                    if let Some(msg) = sync.error_message() {
                        vector_error = Some(msg);
                    }
                    // The write lock must be dropped BEFORE auto_link_cosine
                    // (it takes a read lock on the same index, #442).
                }
                Err(_) => {
                    vector_error = Some("index write lock poisoned; vector not written".into());
                }
            }
        }

        // Cosine-similarity auto-linking (#401).
        // Must run AFTER index.insert() so the new memory is searchable.
        // Best-effort: errors logged, never fails remember().
        // #1166: cosine auto-linking needs a real embedding; skip when the
        // row was stored FTS5-only (no embedder configured).
        let embedding_written = !embedding.is_empty() && vector_error.is_none();
        if embedding_written {
            self.auto_link_cosine(&id, embedding, Some(memory.namespace.as_str()));
        }

        Ok(RememberOutcome {
            embedding_written,
            warning: vector_error,
            id,
            redactions: Vec::new(),
        })
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
mod dedup_tests {
    use super::time_travel::memory_existed_at;

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
            recall_count: 0,
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
            author: None,
        }
    }

    #[test]
    fn test_dedup_blocks_exact_duplicate() {
        let uteke = crate::test_support::open_keyword();
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
    fn test_dedup_allows_different_content() {
        let uteke = crate::test_support::open_keyword();
        let id1 = uteke
            .remember("The sky is blue", &[], None, Some("dedup2"))
            .unwrap();
        let id2 = uteke
            .remember("Rust is a programming language", &[], None, Some("dedup2"))
            .unwrap();
        assert_ne!(id1, id2, "different content should create new memory");
    }

    #[test]
    fn test_dedup_namespace_scoped() {
        let uteke = crate::test_support::open_keyword();
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
    fn test_dedup_skips_deprecated_stale_index_entry() {
        // #1210: a stale vector index on a long-running server can still hold
        // the vector of a soft-deleted (deprecated) memory. Dedup must NOT
        // match it — the rewrite must produce a NEW live memory instead of
        // silently returning the deprecated ghost id (which also gets linked
        // into rooms by remember_in_room, invisible in recall).
        let uteke = crate::test_support::open_keyword();
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
    fn test_contradiction_remembers_metadata() {
        // Regression: remember_with_contradiction must pass metadata through
        // to remember_precomputed (was dropping it as None).
        let uteke = crate::test_support::open_keyword();
        let meta = Some(serde_json::json!({
            "entity": "test-app",
            "category": "integration"
        }));
        let (outcome, _contradiction) = uteke
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
        let id = outcome.id;
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

#[cfg(test)]
mod remember_embedding_status_tests {
    //! #1273 — honest embedding-write reporting across the remember chain.
    use super::RememberOutcome;
    use crate::Uteke;
    use crate::embed::Embedder;
    use crate::error::Error;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Embedder whose embed() always fails — simulates a provider outage.
    struct FailingEmbedder {
        calls: AtomicUsize,
    }

    impl Embedder for FailingEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(Error::embed_msg("simulated provider outage"))
        }
        fn dims(&self) -> usize {
            4
        }
        fn max_seq_len(&self) -> usize {
            128
        }
        fn name(&self) -> &str {
            "failing-test-embedder"
        }
    }

    /// Embedder that fails `fail_times` times, then succeeds — proves the
    /// retry path (#621) still lands a usable vector.
    struct FlakyThenOkEmbedder {
        fail_times: usize,
        calls: AtomicUsize,
    }

    impl Embedder for FlakyThenOkEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_times {
                Err(Error::embed_msg(format!("transient failure #{n}")))
            } else {
                Ok(vec![0.1f32; 4])
            }
        }
        fn dims(&self) -> usize {
            4
        }
        fn max_seq_len(&self) -> usize {
            128
        }
        fn name(&self) -> &str {
            "flaky-test-embedder"
        }
    }

    /// Open an in-memory Uteke with an injected embedder (test-only; both
    /// constructor pieces are crate-private and reachable from here).
    fn open_with_embedder(embedder: Option<Box<dyn Embedder>>, backend: &str) -> Uteke {
        let (_db_str, store) = Uteke::open_store(":memory:").expect("open_store");
        Uteke::finish_open_full(
            store,
            embedder,
            backend.to_string(),
            crate::TierConfig::default(),
            crate::RecallConfig::default(),
            crate::EmbeddingSettings::default(),
            crate::graph_rerank::GraphRerankConfig::default(),
            None,
        )
        .expect("finish_open_full")
    }

    #[test]
    #[serial_test::serial]
    fn remember_reports_missing_embedding_when_generation_fails() {
        let u = open_with_embedder(
            Some(Box::new(FailingEmbedder {
                calls: AtomicUsize::new(0),
            })),
            "test-failing",
        );

        // The store must SUCCEED (row kept) while honestly reporting that
        // no embedding was written — never a bare success, never a hard error.
        let out: RememberOutcome = u
            .remember_typed_detailed("embedding outage probe", &[], None, None, "fact")
            .expect("store must succeed even when embedding fails (#1273)");

        assert!(!out.embedding_written, "embedding_written must be false");
        let warning = out.warning.expect("warning must be present on failure");
        assert!(
            warning.contains("embedding generation failed"),
            "warning should name the root cause, got: {warning}"
        );

        // The row is stored and FTS5/keyword-findable (embedding empty).
        let stored = u
            .store
            .get_by_id(&out.id)
            .expect("store read must work")
            .expect("memory row must exist after embedding failure");
        assert!(
            stored.embedding.is_empty(),
            "stored row must carry no vector when generation failed"
        );
    }

    #[test]
    #[serial_test::serial]
    fn remember_retries_then_succeeds_with_embedding_written() {
        let embedder = FlakyThenOkEmbedder {
            fail_times: 2,
            calls: AtomicUsize::new(0),
        };
        // AtomicUsize is not Clone/Copy-friendly for two consumers; wrap a
        // shared counter instead by leaking an alias is overkill — just use
        // the same struct for construction and later assertion.
        let u = open_with_embedder(
            Some(Box::new(FlakyThenOkEmbedder {
                fail_times: 2,
                calls: AtomicUsize::new(0),
            })),
            "test-flaky",
        );
        let _ = &embedder; // (counter assertion handled via outcome below)

        let out = u
            .remember_typed_detailed("retry path probe", &[], None, None, "fact")
            .expect("store must succeed after transient failures");

        assert!(out.embedding_written, "vector must be written after retry");
        assert!(out.warning.is_none(), "no warning on the success path");
        let stored = u
            .store
            .get_by_id(&out.id)
            .expect("store read must work")
            .expect("memory row must exist");
        assert_eq!(stored.embedding.len(), 4, "vector dims must match embedder");
    }

    #[test]
    #[serial_test::serial]
    fn remember_without_backend_reports_false_but_no_warning() {
        // #1166 by-design case: no embedder configured → FTS5-only storage.
        let u = open_with_embedder(None, "");
        let out = u
            .remember_detailed("no backend probe", &[], None, None)
            .expect("store must succeed without a backend");
        assert!(!out.embedding_written);
        assert!(
            out.warning.is_none(),
            "configuration (no backend) is not a failure — no warning"
        );
        let stored = u
            .store
            .get_by_id(&out.id)
            .expect("store read must work")
            .expect("memory row must exist");
        assert!(stored.embedding.is_empty());
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
mod retry_embed_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fake embedder that counts calls and fails the first `failures` calls
    /// with the error built by `make_err`, then succeeds.
    struct ScriptedEmbedder {
        calls: Arc<AtomicUsize>,
        failures: usize,
        make_err: fn() -> Error,
    }

    impl crate::embed::Embedder for ScriptedEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.failures {
                Err((self.make_err)())
            } else {
                Ok(vec![0.0; 4])
            }
        }
        fn dims(&self) -> usize {
            4
        }
        fn max_seq_len(&self) -> usize {
            8
        }
        fn name(&self) -> &str {
            "scripted"
        }
    }

    fn scripted(
        failures: usize,
        make_err: fn() -> Error,
    ) -> (
        Mutex<Option<Box<dyn crate::embed::Embedder>>>,
        Arc<AtomicUsize>,
    ) {
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder: Box<dyn crate::embed::Embedder> = Box::new(ScriptedEmbedder {
            calls: Arc::clone(&calls),
            failures,
            make_err,
        });
        (Mutex::new(Some(embedder)), calls)
    }

    #[test]
    fn validation_error_is_not_retried() {
        let (embedder, calls) = scripted(usize::MAX, || Error::Validation("bad input".into()));
        let err = retry_embed(&embedder, "hello").unwrap_err();
        assert!(matches!(err, Error::Validation(_)), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn transient_error_is_retried_then_succeeds() {
        let (embedder, calls) = scripted(1, || Error::embed_msg("timeout"));
        let embedding = retry_embed(&embedder, "hello").unwrap();
        assert_eq!(embedding.len(), 4);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn no_embedder_is_an_error_not_a_panic() {
        let embedder: Mutex<Option<Box<dyn crate::embed::Embedder>>> = Mutex::new(None);
        let err = retry_embed(&embedder, "hello").unwrap_err();
        assert!(matches!(err, Error::Embedding { .. }), "{err:?}");
        assert!(err.to_string().contains("No embedder"), "{err}");
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
