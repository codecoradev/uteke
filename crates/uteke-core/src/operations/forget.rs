//! Forgetting memories: single, soft, promote and the bulk variants.

use super::*;

impl crate::Uteke {
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
        // Audit tombstone BEFORE the delete: with schema v20+ events survive the
        // row, and on pre-v20 stores (FK CASCADE) this is the only chance to
        // record the deletion at all. Written ONLY when the memory actually
        // exists — `delete` below returns false for unknown IDs and must not
        // produce spurious tombstones. Best-effort.
        if self.store.get_by_id(id).ok().flatten().is_some() {
            self.try_timeline_event(id, crate::timeline::TimelineEventType::Forgot, None);
        }
        // SQLite delete (source of truth).
        // Check the return value: Ok(false) means the ID was not found in the DB (#926).
        let deleted = self.store.delete(id)?;
        if !deleted {
            return Err(Error::db_msg(format!(
                "Memory with id='{id}' not found in store. Nothing was deleted."
            )));
        }
        // Vector index remove + persist. A stale on-disk index is surfaced as an
        // error (#926): the DB row is gone, `uteke repair` resyncs the index.
        let sync = index_sync::remove_ids(&mut *index, [id], &SyncPolicy::STANDARD);
        if sync.missing > 0 {
            tracing::warn!("Vector index entry not found during forget for id={id}");
        }
        sync.into_delete_result(&format!("forget id={id}"))
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
        // Remove + persist so the removal survives restart; a stale on-disk
        // index only warns here (the row is deprecated, recall filters it).
        let sync = index_sync::remove_ids(&mut *index, [id], &SyncPolicy::STANDARD);
        if let Some(msg) = sync.error_message() {
            tracing::warn!("Failed to persist vector index after soft_forget id={id}: {msg}");
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
                let sync = index_sync::upsert(
                    &mut *index,
                    &memory.id,
                    &memory.embedding,
                    &SyncPolicy::STANDARD,
                );
                if let Some(msg) = sync.error_message() {
                    tracing::warn!(
                        "Failed to re-add memory id={} to the vector index during promote: {msg}",
                        memory.id
                    );
                }
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
            index_sync::remove_ids(
                &mut *index,
                ids.iter().map(String::as_str),
                &SyncPolicy::STANDARD,
            )
            .into_delete_result("bulk_forget_by_tag (soft)")?;
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
        index_sync::remove_ids(
            &mut *index,
            ids.iter().map(String::as_str),
            &SyncPolicy::STANDARD,
        )
        .into_delete_result("bulk_forget_by_tag")?;
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
            index_sync::remove_ids(
                &mut *index,
                ids.iter().map(String::as_str),
                &SyncPolicy::STANDARD,
            )
            .into_delete_result("bulk_forget_cold (soft)")?;
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
        index_sync::remove_ids(
            &mut *index,
            ids.iter().map(String::as_str),
            &SyncPolicy::STANDARD,
        )
        .into_delete_result("bulk_forget_cold")?;
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
            index_sync::remove_ids(
                &mut *index,
                ids.iter().map(String::as_str),
                &SyncPolicy::STANDARD,
            )
            .into_delete_result("bulk_forget_all (soft)")?;
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
        index_sync::remove_ids(
            &mut *index,
            ids.iter().map(String::as_str),
            &SyncPolicy::STANDARD,
        )
        .into_delete_result("bulk_forget_all")?;
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }
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

    /// Owner decision 2026-09-18 (audit-grade timeline): the deprecate path
    /// (soft forget, the fleet's dominant delete traffic) must write a
    /// `deprecated` timeline event, and a hard forget must write a `forgot`
    /// tombstone whose events survive the row deletion (schema v20).
    #[test]
    fn test_timeline_deprecate_and_forget_events() {
        let uteke = Uteke::open(":memory:").unwrap();

        // Soft forget → deprecated event recorded.
        let embedding = vec![0.1_f32; 768];
        let id = uteke
            .remember_precomputed(
                "soft-deleted memory",
                &[],
                None,
                None,
                "fact",
                "text",
                &embedding,
            )
            .unwrap();
        uteke.forget(&id).unwrap();
        let events = uteke.timeline(&id, 50).unwrap();
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(
            types.contains(&"deprecated"),
            "soft forget must record a deprecated event, got: {types:?}"
        );

        // Hard forget → forgot tombstone, and history survives the delete.
        let dir = std::env::temp_dir().join(format!("uteke-tl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut uteke2 = crate::test_support::open_keyword_at(dir.join("t.db"));
        uteke2.set_lifecycle_config(crate::LifecycleConfig {
            soft_delete_only: false,
            ..crate::LifecycleConfig::default()
        });
        let id2 = uteke2
            .remember_precomputed(
                "hard-deleted memory",
                &[],
                None,
                None,
                "fact",
                "text",
                &embedding,
            )
            .unwrap();
        uteke2.forget(&id2).unwrap();

        // Direct SQL through a fresh store handle: the memory row is gone but
        // its events (created + forgot tombstone) must still be there.
        let conn = rusqlite::Connection::open(dir.join("t.db")).unwrap();
        let mem_count: i32 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE id = ?1", [&id2], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(mem_count, 0, "hard forget must remove the memory row");
        let event_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM timeline_events WHERE memory_id = ?1",
                [&id2],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            event_count >= 2,
            "history must survive hard delete: expected created+forgot events, got {event_count}"
        );
        let forgot: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM timeline_events WHERE memory_id = ?1 AND event_type='forgot'",
                [&id2],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            forgot, 1,
            "hard forget must record exactly one forgot tombstone"
        );
        std::fs::remove_dir_all(&dir).ok();
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
        let uteke = crate::test_support::open_keyword_at(dir.join("t.db"));
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
