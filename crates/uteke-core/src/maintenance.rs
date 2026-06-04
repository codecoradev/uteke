//! Maintenance operations: doctor, verify, repair, stats, aging, prune, shutdown.

use crate::error::{format_bytes, Error};
use crate::memory::store::Store;
use crate::memory::types::{AgingStatus, CleanupResult, Memory, PruneResult, StoreStats};
use crate::types::{DoctorCheck, DoctorReport, DoctorStatus, RepairReport, VerifyReport};
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

        // 2. usearch index
        let index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during doctor"))?;
        let index_count = index.len();
        checks.push(DoctorCheck {
            name: "usearch index".to_string(),
            status: DoctorStatus::Ok,
            detail: format!("{} vectors", index_count),
        });

        // 3. Index consistency
        if db_count == index_count {
            checks.push(DoctorCheck {
                name: "Index consistency".to_string(),
                status: DoctorStatus::Ok,
                detail: format!("DB={} Index={}", db_count, index_count),
            });
        } else {
            checks.push(DoctorCheck {
                name: "Index consistency".to_string(),
                status: DoctorStatus::Error,
                detail: format!(
                    "MISMATCH: DB={} Index={} — run `uteke repair`",
                    db_count, index_count
                ),
            });
        }

        // 4. Embedding model
        let model_dir = uteke_home().join("models").join("embeddinggemma-q4");
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
        let index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during verify"))?;
        let index_count = index.len();

        let consistent = db_count == index_count;
        Ok(VerifyReport {
            db_count,
            index_count,
            consistent,
        })
    }

    /// Repair: rebuild usearch index from SQLite.
    pub fn repair(&self) -> Result<RepairReport, Error> {
        let before_db = self.store.count(None)?;
        let before_index = {
            let index = self
                .index
                .lock()
                .map_err(|_| Error::lock("index lock during repair (count before)"))?;
            index.len()
        };

        // Load all from SQLite and rebuild index
        let all_memories = self.store.load_all(None)?;
        let items: Vec<(String, Vec<f32>)> = all_memories
            .iter()
            .map(|m| (m.id.clone(), m.embedding.clone()))
            .collect();

        {
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::lock("index lock during repair (rebuild)"))?;
            index.build(&items);
            index.save().ok();
        }

        Ok(RepairReport {
            db_count: before_db,
            index_before: before_index,
            index_after: items.len(),
        })
    }

    /// Get statistics about the memory store.
    /// Access the underlying store (read-only reference).
    pub fn store(&self) -> &Store {
        &self.store
    }

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

        Ok(StoreStats {
            total_memories,
            unique_tags,
            db_size_bytes,
            hot,
            warm,
            cold,
        })
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

    /// Cleanup aged memories — deletes from SQLite AND removes from vector index.
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
        let ids: Vec<String> = aged.into_iter().map(|m| m.id).collect();

        if ids.is_empty() {
            return Ok(CleanupResult { deleted: 0 });
        }

        // Delete from SQLite
        let deleted = self
            .store
            .cleanup_aged(older_than_days, max_access_count, namespace)?;

        // Remove from vector index
        {
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::lock("index lock during aging_cleanup"))?;
            for id in &ids {
                index.remove(id);
            }
            index.save().ok();
        }

        Ok(CleanupResult { deleted })
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

        // Delete from SQLite
        let pruned = self.store.prune_ttl(ttl_days, namespace)?;

        // Remove from vector index
        {
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::lock("index lock during prune"))?;
            for id in &ids {
                index.remove(id);
            }
            index.save().ok();
        }

        Ok(PruneResult {
            pruned,
            ids: ids.clone(),
            deprecated: count,
            deprecated_ids: ids,
        })
    }

    /// Graceful shutdown — save dirty index to disk.
    pub fn shutdown(&self) -> Result<(), Error> {
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during shutdown"))?;
        if index.is_dirty() {
            index.save()?;
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

    #[test]
    fn test_cleanup_result_serialization() {
        use crate::memory::types::CleanupResult;
        let result = CleanupResult { deleted: 5 };
        let json = serde_json::to_string(&result).unwrap();
        let restored: CleanupResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.deleted, 5);
    }
}
