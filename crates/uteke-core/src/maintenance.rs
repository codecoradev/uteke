//! Maintenance operations: doctor, verify, repair, stats, aging, prune, shutdown.

use crate::error::{Error, format_bytes};
use crate::index_sync::{self, SyncPolicy};
use crate::memory::types::{
    AgingStatus, CleanupResult, LifecycleCycleResult, Memory, PruneResult, StoreStats,
};
use crate::types::{
    DoctorCheck, DoctorReport, DoctorStatus, ReembedReport, RepairReport, VerifyReport,
};
use crate::uteke_home;

impl crate::Uteke {
    /// Check system health: DB, index, model, consistency.
    pub fn doctor(&self) -> Result<DoctorReport, Error> {
        let mut checks = Vec::new();

        // 1. SQLite DB
        let db_count = self.store.count(None)?;
        let db_path = self.store.path();
        let db_size = db_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);
        checks.push(DoctorCheck {
            name: "SQLite DB".to_string(),
            status: DoctorStatus::Ok,
            detail: format!("{} memories, {}", db_count, format_bytes(db_size)),
        });

        // 2. vector index (backend-aware label, #1112)
        let index = self
            .index
            .read()
            .map_err(|_| Error::lock("index read lock during doctor"))?;
        let index_count = index.len();
        checks.push(DoctorCheck {
            name: format!("{} index", crate::memory::vector::INDEX_EXT),
            status: DoctorStatus::Ok,
            detail: format!("{} vectors", index_count),
        });

        // 3. Index consistency
        // The vector index holds memories AND document chunk vectors
        // ("chunk:<id>" keys) — compare index.len() against both counts so
        // stores with documents don't report a false MISMATCH (#1111).
        let chunk_count = self.store.load_all_chunk_embeddings()?.len();
        let expected = db_count + chunk_count;
        if expected == index_count {
            checks.push(DoctorCheck {
                name: "Index consistency".to_string(),
                status: DoctorStatus::Ok,
                detail: format!(
                    "DB={} (+{} chunks) Index={}",
                    db_count, chunk_count, index_count
                ),
            });
        } else {
            checks.push(DoctorCheck {
                name: "Index consistency".to_string(),
                status: DoctorStatus::Error,
                detail: format!(
                    "MISMATCH: DB={} (+{} chunks) Index={} — run `uteke repair`",
                    db_count, chunk_count, index_count
                ),
            });
        }

        // 4. Embedding model
        let model_dir = match uteke_home() {
            Ok(p) => p.join("models").join("embeddinggemma-q4"),
            Err(_) => {
                checks.push(DoctorCheck {
                    name: "Home directory".to_string(),
                    status: DoctorStatus::Error,
                    detail: "Cannot determine home directory. Set UTEKE_HOME.".to_string(),
                });
                return Ok(DoctorReport { checks });
            }
        };
        let model_file = model_dir.join("onnx").join("model_q4.onnx");
        let tokenizer_file = model_dir.join("tokenizer.json");
        let model_exists = model_file.exists() && tokenizer_file.exists();
        checks.push(DoctorCheck {
            name: "Embedding model".to_string(),
            status: if model_exists {
                DoctorStatus::Ok
            } else {
                DoctorStatus::Error
            },
            detail: if model_exists {
                "embeddinggemma-q4".to_string()
            } else {
                "Model files not found — will download on first use".to_string()
            },
        });

        Ok(DoctorReport { checks })
    }

    /// Verify DB and index consistency. Returns mismatch count.
    pub fn verify(&self) -> Result<VerifyReport, Error> {
        let db_count = self.store.count(None)?;
        // Chunk vectors share the index ("chunk:<id>" keys) — include them on
        // the DB side so stores with documents don't report false MISMATCH (#1111).
        let chunk_count = self.store.load_all_chunk_embeddings()?.len();
        let index = self
            .index
            .read()
            .map_err(|_| Error::lock("index read lock during verify"))?;
        let index_count = index.len();

        let consistent = db_count + chunk_count == index_count;
        Ok(VerifyReport {
            db_count,
            chunk_count,
            index_count,
            consistent,
        })
    }

    /// Repair: rebuild usearch index from SQLite + fix schema inconsistencies.
    pub fn repair(&self) -> Result<RepairReport, Error> {
        // Fix partially-migrated schema (e.g. missing has_children column, #500).
        self.store.ensure_schema_consistency()?;

        let before_db = self.store.count(None)?;
        let before_index = {
            let index = self
                .index
                .read()
                .map_err(|_| Error::lock("index read lock during repair (count before)"))?;
            index.len()
        };

        // Load all from SQLite and rebuild index (NULL embeddings filtered in load_all)
        let all_memories = self.store.load_all(None)?;
        let mut items: Vec<(String, Vec<f32>)> = all_memories
            .iter()
            .filter(|m| !m.embedding.is_empty())
            .map(|m| (m.id.clone(), m.embedding.clone()))
            .collect();

        // Document chunk vectors live in the index under "chunk:<id>" keys.
        // load_all returns memories only — without this the rebuild silently
        // evicts every chunk entry (#1110). Chunk embeddings are persisted in
        // document_chunks, so no re-embedding is needed.
        let chunk_count = {
            let chunks = self.store.load_all_chunk_embeddings()?;
            let n = chunks.len();
            items.extend(chunks.into_iter().map(|(id, emb)| {
                let key = format!("chunk:{id}");
                (key, emb)
            }));
            n
        };
        if chunk_count > 0 {
            tracing::info!(
                chunks = chunk_count,
                "including document chunks in index rebuild"
            );
        }

        {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during repair (rebuild)"))?;
            index.build(&items)?;
            if let Err(e) = index_sync::persist(&mut *index, &SyncPolicy::STANDARD) {
                tracing::warn!("Failed to save index after repair rebuild: {e}");
            }
        }

        Ok(RepairReport {
            db_count: before_db,
            index_before: before_index,
            index_after: items.len(),
            chunk_count,
        })
    }

    /// Re-embed memories that have missing or empty embedding vectors.
    ///
    /// Scans active memories with NULL/empty embeddings via a dedicated SQL
    /// query (`load_missing_embeddings`), generates new embeddings, updates
    /// the database, and adds them to the index.
    pub fn reembed_missing(&self) -> Result<ReembedReport, Error> {
        // Scan directly for NULL/empty embeddings (#1146). This must NOT go
        // through load_all(): its `embedding IS NOT NULL` guard (kept for
        // index.build() safety, #992) filtered out exactly the rows this
        // function exists to repair, making NULL rows permanently invisible.
        let missing: Vec<Memory> = self.store.load_missing_embeddings(None)?;
        let total_scanned = self.store.load_all(None)?.len();

        let missing_count = missing.len();
        if missing_count == 0 {
            return Ok(ReembedReport {
                total_scanned,
                missing_count,
                reembedded: 0,
                failed: 0,
            });
        }

        tracing::info!(
            total = total_scanned,
            missing = missing_count,
            "Re-embedding memories with missing vectors"
        );

        // Ensure embedder is initialized.
        self.ensure_embedder()?;

        let mut reembedded = 0usize;
        let mut failed = 0usize;
        let mut new_items: Vec<(String, Vec<f32>)> = Vec::new();

        // Acquire embedder lock ONCE before the loop to avoid
        // per-iteration mutex overhead (codecoradev/uteke#919 review).
        let embedder_guard = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during reembed"))?;
        let embedder = embedder_guard
            .as_ref()
            .ok_or_else(|| Error::embed("reembed", "embedder not initialized"))?;

        for mem in &missing {
            match embedder.embed(&mem.content) {
                Ok(vec) => {
                    // Update memory in database.
                    let mut updated = (*mem).clone();
                    updated.embedding = vec.clone();
                    updated.updated_at = chrono::Utc::now();
                    if let Err(e) = self.store.update(&updated) {
                        tracing::warn!(id = %mem.id, error = %e, "Failed to update embedding in DB");
                        failed += 1;
                        continue;
                    }
                    new_items.push((mem.id.clone(), vec));
                    reembedded += 1;
                }
                Err(e) => {
                    tracing::warn!(id = %mem.id, error = %e, "Failed to generate embedding");
                    failed += 1;
                }
            }
        }

        // Add newly-embedded memories to the index.
        if !new_items.is_empty() {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write during reembed"))?;
            for (id, vec) in &new_items {
                if let Err(e) =
                    index_sync::insert_unsaved(&mut *index, id, vec, &SyncPolicy::STANDARD)
                {
                    tracing::warn!(id = %id, error = %e, "Failed to add to index");
                }
            }
            if let Err(e) = index_sync::persist(&mut *index, &SyncPolicy::STANDARD) {
                tracing::warn!(error = %e, "Failed to save index after reembed");
            }
        }

        tracing::info!(reembedded, failed, "Re-embed complete");

        Ok(ReembedReport {
            total_scanned,
            missing_count,
            reembedded,
            failed,
        })
    }

    /// Get statistics about the memory store.
    pub fn stats(&self, namespace: Option<&str>) -> Result<StoreStats, Error> {
        let total_memories = self.store.count(namespace)?;
        let unique_tags = self.store.unique_tags(namespace)?.len();
        let (hot, warm, cold) = self.store.tier_counts(
            namespace,
            self.tier_config.hot_days,
            self.tier_config.warm_days,
        )?;

        let db_size_bytes = self
            .store
            .path()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);

        let (cache_hits, cache_misses) = self.recall_cache.metrics();

        Ok(StoreStats {
            total_memories,
            unique_tags,
            db_size_bytes,
            hot,
            warm,
            cold,
            cache_hits,
            cache_misses,
        })
    }

    /// Count PINNED (never-decay) memories, optionally per namespace (#1052).
    /// Surfaced for MCP stats triage output.
    pub fn count_pinned(&self, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.count_pinned(namespace)
    }

    /// Count DEPRECATED (soft-deleted) memories, optionally per namespace.
    /// Audit/triage counter — hidden from recall but restorable (#1052).
    pub fn count_deprecated(&self, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.count_deprecated(namespace)
    }

    /// Per-namespace memory counts for stats breakdown (#1052).
    pub fn namespace_counts(&self) -> Result<Vec<(String, usize)>, Error> {
        self.store.list_namespaces_with_counts()
    }

    /// Get aging status — breakdown of memories by access tier.
    pub fn aging_status(&self, namespace: Option<&str>) -> Result<AgingStatus, Error> {
        let total = self.store.count(namespace)?;
        let (hot, warm, cold) = self.store.tier_counts(
            namespace,
            self.tier_config.hot_days,
            self.tier_config.warm_days,
        )?;
        let never_accessed = self.store.count_never_accessed(namespace)?;

        Ok(AgingStatus {
            total,
            hot,
            warm,
            cold,
            never_accessed,
        })
    }

    /// Preview aged memories eligible for cleanup (dry-run).
    pub fn aging_preview(
        &self,
        older_than_days: u32,
        max_access_count: u32,
        namespace: Option<&str>,
    ) -> Result<Vec<Memory>, Error> {
        self.store
            .find_aged(older_than_days, max_access_count, namespace)
    }

    /// Cleanup aged memories — deprecates (soft-delete) or deletes based on lifecycle config.
    ///
    /// When `soft_delete_only` is enabled (default, #930): deprecates aged memories
    /// instead of hard-deleting them. The memories remain in SQLite but are hidden
    /// from recall and restorable via `promote()`.
    ///
    /// When `soft_delete_only` is disabled: hard-deletes from SQLite AND vector index
    /// (legacy behavior, use with caution).
    ///
    /// Safety limits (#933):
    /// - Caps at `max_deprecate_percent`% of total memories per cycle
    /// - Never more than `max_deprecate_per_cycle` items
    /// - Never less than `min_deprecate_per_cycle` (if candidates exist)
    pub fn aging_cleanup(
        &self,
        older_than_days: u32,
        max_access_count: u32,
        namespace: Option<&str>,
    ) -> Result<CleanupResult, Error> {
        // Find aged memories first to get IDs for vector index removal
        let aged = self
            .store
            .find_aged(older_than_days, max_access_count, namespace)?;

        if aged.is_empty() {
            return Ok(CleanupResult { deleted: 0 });
        }

        // Dynamic cap: limit deprecations to max_deprecate_percent of total memories.
        let total = self.store.count(namespace)?;
        let lc = &self.lifecycle_config;
        let pct_cap = ((total as f64 * lc.max_deprecate_percent / 100.0).round() as usize)
            .max(lc.min_deprecate_per_cycle)
            .min(lc.max_deprecate_per_cycle);
        let cap = pct_cap.min(aged.len());

        let ids: Vec<String> = aged.into_iter().take(cap).map(|m| m.id).collect();

        if ids.is_empty() {
            return Ok(CleanupResult { deleted: 0 });
        }

        if lc.soft_delete_only {
            // Soft-delete: deprecate with reason (#930)
            let reason = format!(
                "auto-aging: older than {older_than_days} days, access count ≤ {max_access_count}"
            );
            let deprecated = self.store.deprecate_by_ids(&ids, &reason)?;

            // Remove from vector index (so they don't appear in recall)
            {
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during aging_cleanup"))?;
                index_sync::remove_ids(
                    &mut *index,
                    ids.iter().map(String::as_str),
                    &SyncPolicy::STANDARD,
                )
                .warn_if_stale("aging_cleanup (soft-delete)");
            }

            tracing::info!(
                "Aging cleanup (soft-delete): deprecated {} of {} candidates (cap={cap}, total={total})",
                deprecated,
                ids.len(),
            );
            Ok(CleanupResult {
                deleted: deprecated,
            })
        } else {
            // Hard delete (legacy behavior, only when soft_delete_only=false)
            let deleted = self.store.delete_by_ids(&ids)?;

            // Remove from vector index
            {
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during aging_cleanup"))?;
                index_sync::remove_ids(
                    &mut *index,
                    ids.iter().map(String::as_str),
                    &SyncPolicy::STANDARD,
                )
                .warn_if_stale("aging_cleanup (hard delete)");
            }

            tracing::info!(
                "Aging cleanup (hard delete): deleted {} of {} candidates (cap={cap}, total={total})",
                deleted,
                ids.len(),
            );
            Ok(CleanupResult { deleted })
        }
    }

    /// Prune deprecated memories older than TTL days.
    ///
    /// Deletes from both SQLite and vector index.
    pub fn prune(
        &self,
        ttl_days: u32,
        namespace: Option<&str>,
        dry_run: bool,
    ) -> Result<PruneResult, Error> {
        let deprecated = self.store.find_deprecated_for_prune(ttl_days, namespace)?;
        let ids: Vec<String> = deprecated.iter().map(|m| m.id.clone()).collect();
        let count = ids.len();

        if dry_run || count == 0 {
            return Ok(PruneResult {
                pruned: 0,
                ids: vec![],
                deprecated: count,
                deprecated_ids: ids,
            });
        }

        // Delete by specific IDs to avoid TOCTOU race (not re-query by criteria)
        let pruned = self.store.delete_by_ids(&ids)?;

        // Remove from vector index
        {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during prune"))?;
            index_sync::remove_ids(
                &mut *index,
                ids.iter().map(String::as_str),
                &SyncPolicy::STANDARD,
            )
            .warn_if_stale("prune");
        }

        Ok(PruneResult {
            pruned,
            ids: ids.clone(),
            deprecated: count,
            deprecated_ids: ids,
        })
    }

    /// Run one lifecycle cycle (#933).
    ///
    /// Two-phase operation:
    /// 1. **Deprecate phase**: find aged memories, cap to max N% of total active,
    ///    soft-delete (deprecate) the oldest ones.
    /// 2. **Prune phase**: hard-delete memories that have been deprecated longer
    ///    than `deprecated_ttl_days`.
    ///
    /// The percentage cap (`max_deprecate_percent`) limits how many memories can
    /// be deprecated per cycle — defaults to 1.0%, clamped between
    /// `min_deprecate_per_cycle` and `max_deprecate_per_cycle`.
    pub fn lifecycle_cycle(&self, namespace: Option<&str>) -> Result<LifecycleCycleResult, Error> {
        let cfg = &self.lifecycle_config;

        // Phase 1: Find candidates and apply percentage cap.
        let total_active = self.store.count_active(namespace)?;
        let candidates = self
            .store
            .find_aged(cfg.min_age_days, cfg.max_access_count, namespace)?;

        // Calculate cap: percentage of total, clamped to [min, max].
        let raw_cap = ((total_active as f64) * cfg.max_deprecate_percent / 100.0) as usize;
        let cap = raw_cap
            .max(cfg.min_deprecate_per_cycle)
            .min(cfg.max_deprecate_per_cycle);

        // Take oldest `cap` candidates (find_aged already sorts by created_at ASC).
        let to_deprecate: Vec<String> = candidates.iter().take(cap).map(|m| m.id.clone()).collect();

        let deprecated_ids = to_deprecate.clone();
        let deprecated_count = if !to_deprecate.is_empty() {
            let reason = "lifecycle_cycle: aged memory (auto-deprecate)";
            self.store.deprecate_by_ids(&to_deprecate, reason)?
        } else {
            0
        };

        // Remove deprecated from vector index.
        if !to_deprecate.is_empty() {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during lifecycle_cycle"))?;
            index_sync::remove_ids(
                &mut *index,
                to_deprecate.iter().map(String::as_str),
                &SyncPolicy::STANDARD,
            )
            .warn_if_stale("lifecycle_cycle deprecate");
        }

        // Invalidate recall cache.
        if let Some(ns) = namespace {
            self.recall_cache.invalidate_namespace(ns);
        } else {
            self.recall_cache.clear();
        }

        tracing::info!(
            "Lifecycle cycle: {} of {} candidates deprecated (cap={}, total_active={})",
            deprecated_count,
            candidates.len(),
            cap,
            total_active
        );

        // Phase 2: Auto-prune expired deprecated memories.
        // soft_delete_only only restricts forget() from hard-deleting active memories.
        // Prune operates on already-deprecated memories past their TTL, which is the
        // intended hard-delete path regardless of soft_delete_only.
        let (pruned, pruned_ids) = if cfg.auto_prune_enabled {
            let expired = self
                .store
                .find_deprecated_for_prune(cfg.deprecated_ttl_days, namespace)?;
            let expired_ids: Vec<String> = expired.iter().map(|m| m.id.clone()).collect();
            if expired_ids.is_empty() {
                (0, vec![])
            } else {
                let pruned_count = self.store.delete_by_ids(&expired_ids)?;
                // Remove from vector index.
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during lifecycle prune"))?;
                index_sync::remove_ids(
                    &mut *index,
                    expired_ids.iter().map(String::as_str),
                    &SyncPolicy::STANDARD,
                )
                .warn_if_stale("lifecycle_cycle prune");
                tracing::info!(
                    "Lifecycle cycle: pruned {} expired deprecated memories (ttl={}d)",
                    pruned_count,
                    cfg.deprecated_ttl_days
                );
                (pruned_count, expired_ids)
            }
        } else {
            (0, vec![])
        };

        Ok(LifecycleCycleResult {
            total_active,
            candidates: candidates.len(),
            cap,
            deprecated: deprecated_count,
            deprecated_ids,
            pruned,
            pruned_ids,
        })
    }

    /// Graceful shutdown — save dirty index to disk.
    pub fn shutdown(&self) -> Result<(), Error> {
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::lock("index write lock during shutdown"))?;
        if index.is_dirty() {
            index_sync::persist(&mut *index, &SyncPolicy::STANDARD)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::memory::types::AgingStatus;

    #[test]
    fn test_aging_status_type_serialization() {
        let status = AgingStatus {
            total: 100,
            hot: 10,
            warm: 20,
            cold: 50,
            never_accessed: 20,
        };
        let json = serde_json::to_string(&status).unwrap();
        let restored: AgingStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.total, 100);
        assert_eq!(restored.never_accessed, 20);
    }

    #[test]
    fn test_prune_result_serialization() {
        use crate::memory::types::PruneResult;
        let result = PruneResult {
            pruned: 5,
            ids: vec!["a".to_string(), "b".to_string()],
            deprecated: 3,
            deprecated_ids: vec!["c".to_string()],
        };
        let json = serde_json::to_string(&result).unwrap();
        let restored: PruneResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.pruned, 5);
        assert_eq!(restored.deprecated, 3);
    }

    #[ignore = "requires ONNX embedder — validates reembed repairs NULL-embedding rows (#1146)"]
    #[test]
    fn test_reembed_repairs_null_embedding_rows() {
        // Regression test for #1146: `repair --reembed` used to scan through
        // load_all(), whose `embedding IS NOT NULL` guard (#992) filtered out
        // exactly the rows needing repair. NULL-embedding rows (write-path
        // crash artifacts) were permanently invisible to reembed and doctor
        // reported an unresolvable MISMATCH.
        let dir = tempfile::tempdir().unwrap();
        let uteke = crate::Uteke::open(dir.path().join("uteke.db")).unwrap();

        // Create one healthy memory (gets a real embedding).
        let id = uteke
            .remember("raft consensus requires a majority quorum", &[], None, None)
            .unwrap();

        // Simulate a write-path crash artifact: NULL embedding.
        let id_null = uteke
            .remember("vector quantization compresses embeddings", &[], None, None)
            .unwrap();
        uteke
            .graph_store()
            .execute(
                "UPDATE memories SET embedding = NULL WHERE id = ?1",
                rusqlite::params![id_null],
            )
            .unwrap();

        // And the empty-blob variant (what the old code could only see).
        let id_empty = uteke
            .remember(
                "hybrid search blends keyword and vector signals",
                &[],
                None,
                None,
            )
            .unwrap();
        uteke
            .graph_store()
            .execute(
                "UPDATE memories SET embedding = X'' WHERE id = ?1",
                rusqlite::params![id_empty],
            )
            .unwrap();

        // Scan finds both NULL and empty-blob rows.
        let scanned = uteke.store.load_missing_embeddings(None).unwrap();
        assert_eq!(
            scanned.len(),
            2,
            "scan must see NULL-embedding rows, not just empty-blob ones"
        );

        // Reembed repairs both; the healthy row is untouched.
        let report = uteke.reembed_missing().unwrap();
        assert_eq!(report.missing_count, 2);
        assert_eq!(report.reembedded, 2, "both rows must be re-embedded");
        assert_eq!(report.failed, 0);

        // DB no longer has NULL/empty embeddings among active memories.
        let remaining: i64 = uteke
            .graph_store()
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE (embedding IS NULL OR length(embedding) = 0) AND deprecated = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);

        // End-to-end: verify() reports a consistent store after repair.
        let v = uteke.verify().unwrap();
        assert!(
            v.consistent,
            "store must be consistent after reembed, got db={} index={}",
            v.db_count, v.index_count
        );

        // Reembed is now a no-op.
        let again = uteke.reembed_missing().unwrap();
        assert_eq!(again.missing_count, 0);
        assert_eq!(again.reembedded, 0);
        let _ = (id, id_null, id_empty);
    }

    #[test]
    fn test_cleanup_result_serialization() {
        use crate::memory::types::CleanupResult;
        let result = CleanupResult { deleted: 5 };
        let json = serde_json::to_string(&result).unwrap();
        let restored: CleanupResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.deleted, 5);
    }

    #[ignore = "requires ONNX embedder — verify/doctor count chunk vectors alongside memories"]
    #[test]
    fn test_verify_counts_doc_chunks() {
        // Regression test for #1111: verify()/doctor() must count document
        // chunk vectors on the DB side, otherwise any store with documents
        // reports a memory-only count vs index (which holds memories AND
        // chunk vectors) and flags a false MISMATCH.
        let dir = tempfile::tempdir().unwrap();
        let uteke = crate::Uteke::open(dir.path().join("uteke.db")).unwrap();
        uteke
            .doc_upsert(
                "verify-chunks-doc",
                "Verify Chunks Doc",
                "A document about distributed systems and consensus algorithms like raft.",
                &[],
                None,
            )
            .unwrap();

        let report = uteke.verify().unwrap();
        assert_eq!(report.db_count, 0, "no memories in this store");
        assert!(report.chunk_count >= 1, "doc chunks must be counted");
        assert_eq!(
            report.index_count,
            report.db_count + report.chunk_count,
            "index holds memories + chunk vectors"
        );
        assert!(
            report.consistent,
            "no false MISMATCH with documents present"
        );
    }

    #[ignore = "requires ONNX embedder — validates repair() keeps doc chunk vectors indexed"]
    #[test]
    fn test_repair_preserves_doc_chunk_vectors() {
        // Regression test for #1110: repair() rebuilt the index from
        // load_all() (memories only), silently evicting every "chunk:<id>"
        // vector. After repair, semantic doc search must still find chunks.
        let dir = tempfile::tempdir().unwrap();
        let uteke = crate::Uteke::open(dir.path().join("uteke.db")).unwrap();
        let doc_id = uteke
            .doc_upsert(
                "repair-chunks-doc",
                "Repair Chunks Doc",
                "Chapter one introduces the architecture. Chapter two covers vector quantization theory in depth with mathematical foundations.",
                &[],
                None,
            )
            .unwrap();

        // Sanity: semantic search finds the doc before repair.
        let before = uteke
            .doc_search("quantization theory mathematics", 5, "semantic")
            .unwrap();
        assert!(
            !before.is_empty(),
            "semantic doc search should find the doc pre-repair"
        );

        let report = uteke.repair().unwrap();
        // Index must include memories (0 here) + doc chunks (>= 1).
        assert!(
            report.index_after >= 1,
            "rebuild must include document chunk vectors, got index_after={}",
            report.index_after
        );

        // After repair, the chunk vector must still be searchable.
        let after = uteke
            .doc_search("quantization theory mathematics", 5, "semantic")
            .unwrap();
        assert!(
            !after.is_empty(),
            "semantic doc search must still find doc {} after repair",
            doc_id
        );
    }
}
