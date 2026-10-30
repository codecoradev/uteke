//! Writing a memory: validate, embed, store and everything that follows a store (#1514).

use super::*;

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
            None => crate::memory::types::MemoryType::infer_stored_name(content).to_string(),
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
