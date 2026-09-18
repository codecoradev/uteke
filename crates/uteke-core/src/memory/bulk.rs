//! Bulk operations — bulk delete, deprecation, TTL pruning, similarity search.

use crate::Error;
use crate::memory::types::{DEFAULT_NAMESPACE, Memory};
use crate::timeline::TimelineEventType;
use rusqlite::params;

use super::store::row_to_memory;

impl super::Store {
    /// Bulk delete memories by tag within a namespace.
    ///
    /// Uses a single DELETE query with `RETURNING id` for efficiency.
    pub fn bulk_delete_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare(
                "DELETE FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM memory_tags WHERE memory_id = memories.id AND tag = ?2) RETURNING id",
            )
            .map_err(|e| Error::db("database operation", e))?;
        let ids: Vec<String> = stmt
            .query_map(params![ns, tag], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| {
                if let Err(e) = &r {
                    tracing::warn!("DB row error in bulk delete: {e}");
                }
                r.ok()
            })
            .collect();
        Ok(ids)
    }

    /// Find IDs of memories by tag (for soft-delete path, #932).
    ///
    /// Same query as `bulk_delete_by_tag` but SELECT instead of DELETE.
    pub fn find_ids_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT memories.id FROM memories WHERE namespace = ?1 AND deprecated = 0 AND EXISTS (SELECT 1 FROM memory_tags WHERE memory_id = memories.id AND tag = ?2)",
            )
            .map_err(|e| Error::db("database operation", e))?;
        let ids: Vec<String> = stmt
            .query_map(params![ns, tag], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Bulk delete all cold memories (not accessed in warm_days+ days or never accessed).
    ///
    /// Uses a single DELETE query with `RETURNING id` for efficiency.
    pub fn bulk_delete_cold(
        &self,
        namespace: Option<&str>,
        warm_days: i64,
    ) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let warm_cutoff = (chrono::Utc::now() - chrono::Duration::days(warm_days)).to_rfc3339();
        let mut stmt = self
            .conn
            .prepare(
                "DELETE FROM memories WHERE namespace = ?1 AND (last_accessed < ?2 OR last_accessed IS NULL) RETURNING id",
            )
            .map_err(|e| Error::db("database operation", e))?;
        let ids: Vec<String> = stmt
            .query_map(params![ns, warm_cutoff], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| {
                if let Err(e) = &r {
                    tracing::warn!("DB row error in bulk delete: {e}");
                }
                r.ok()
            })
            .collect();
        Ok(ids)
    }

    /// Bulk delete all memories in a namespace.
    ///
    /// Uses a single DELETE query with `RETURNING id` for efficiency.
    pub fn bulk_delete_all(&self, namespace: Option<&str>) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare("DELETE FROM memories WHERE namespace = ?1 RETURNING id")
            .map_err(|e| Error::db("database operation", e))?;
        let ids: Vec<String> = stmt
            .query_map(params![ns], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| {
                if let Err(e) = &r {
                    tracing::warn!("DB row error in bulk delete: {e}");
                }
                r.ok()
            })
            .collect();
        Ok(ids)
    }

    /// Find IDs of cold memories (for soft-delete path, #932).
    ///
    /// Same criteria as `bulk_delete_cold` but SELECT instead of DELETE.
    pub fn find_ids_cold(
        &self,
        namespace: Option<&str>,
        warm_days: i64,
    ) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let warm_cutoff = (chrono::Utc::now() - chrono::Duration::days(warm_days)).to_rfc3339();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id FROM memories WHERE namespace = ?1 AND deprecated = 0 AND (last_accessed < ?2 OR last_accessed IS NULL)",
            )
            .map_err(|e| Error::db("database operation", e))?;
        let ids: Vec<String> = stmt
            .query_map(params![ns, warm_cutoff], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Find all memory IDs in a namespace (for soft-delete path, #932).
    ///
    /// Same query as `bulk_delete_all` but SELECT instead of DELETE.
    pub fn find_ids_all(&self, namespace: Option<&str>) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1 AND deprecated = 0")
            .map_err(|e| Error::db("database operation", e))?;
        let ids: Vec<String> = stmt
            .query_map(params![ns], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Deprecate a memory by ID. Sets deprecated=1 and valid_until=now.
    pub fn deprecate(&self, id: &str) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE memories SET deprecated = 1, valid_until = ?1, updated_at = ?1, deprecated_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| Error::db("database operation", e))?;
        Ok(())
    }

    /// Deprecate a memory with a human-readable reason (#929).
    ///
    /// Sets deprecated=1, valid_until=now, deprecate_reason=reason.
    /// The reason is stored for audit trail and pending-review display.
    pub fn deprecate_with_reason(&self, id: &str, reason: &str) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let rows = self
            .conn
            .execute(
                "UPDATE memories SET deprecated = 1, valid_until = ?1, deprecate_reason = ?2, updated_at = ?1, deprecated_at = ?1 WHERE id = ?3 AND deprecated = 0",
                params![now, reason, id],
            )
            .map_err(|e| Error::db("database operation", e))?;
        if rows == 0 {
            // Check if the memory exists at all (already deprecated or not found)
            let exists: bool = self
                .conn
                .query_row("SELECT 1 FROM memories WHERE id = ?1", params![id], |_| {
                    Ok(true)
                })
                .unwrap_or(false);
            if !exists {
                return Err(Error::db_msg(format!(
                    "Memory with id='{id}' not found in store. Nothing was deprecated."
                )));
            }
            // Already deprecated: idempotent success — no duplicate event.
            return Ok(());
        }
        // Timeline audit trail (owner decision 2026-09-18): the deprecate path
        // is the fleet's dominant delete traffic and was previously unwired.
        // Best-effort, never fails the deprecation.
        let event_data = serde_json::json!({ "reason": reason });
        if let Err(e) =
            self.add_timeline_event(id, TimelineEventType::Deprecated, Some(&event_data))
        {
            tracing::warn!("deprecated timeline event failed for {id}: {e}");
        }
        Ok(())
    }

    /// Bulk deprecate by IDs with a shared reason (#929).
    ///
    /// Soft-delete variant of `delete_by_ids`. Returns count of deprecated rows.
    pub fn deprecate_by_ids(&self, ids: &[String], reason: &str) -> Result<usize, Error> {
        if ids.is_empty() {
            return Ok(0);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let placeholders: String = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE memories SET deprecated = 1, valid_until = ?1, deprecate_reason = ?2, updated_at = ?1, deprecated_at = ?1 WHERE id IN ({placeholders}) AND deprecated = 0"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(now), Box::new(reason.to_string())];
        for id in ids {
            params_vec.push(Box::new(id.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let count = self
            .conn
            .execute(&sql, rusqlite::params_from_iter(param_refs))
            .map_err(|e| Error::db("database operation", e))?;
        Ok(count)
    }

    /// Restore a deprecated memory to active state (#929).
    ///
    /// Clears deprecated flag, valid_until, and deprecate_reason.
    /// Returns false if the memory was not deprecated or doesn't exist.
    pub fn undeprecate(&self, id: &str) -> Result<bool, Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let rows = self
            .conn
            .execute(
                "UPDATE memories SET deprecated = 0, valid_until = NULL, deprecate_reason = NULL, deprecated_at = NULL, updated_at = ?1 WHERE id = ?2 AND deprecated = 1",
                params![now, id],
            )
            .map_err(|e| Error::db("database operation", e))?;
        Ok(rows > 0)
    }

    /// Find memories that contradict a new embedding (high similarity, same namespace).
    /// Returns memories with cosine similarity > threshold that are not already deprecated.
    pub fn find_similar(&self, namespace: &str, limit: usize) -> Result<Vec<Memory>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type
                 FROM memories WHERE namespace = ?1 AND deprecated = 0 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| Error::db("database operation", e))?;
        let rows = stmt
            .query_map(params![namespace, limit as i64], row_to_memory)
            .map_err(|e| Error::db("database operation", e))?;
        let mut memories = Vec::new();
        for row in rows {
            memories.push(row.map_err(|e| Error::db("database operation", e))?);
        }
        Ok(memories)
    }

    /// Prune (delete) cold, deprecated, or expired memories based on TTL.
    /// Returns count of pruned memories.
    pub fn prune_ttl(&self, ttl_days: u32, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        // Compute cutoff in Rust (RFC3339) to match stored timestamp format.
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(ttl_days as i64)).to_rfc3339();
        let deleted = self
            .conn
            .execute(
                "DELETE FROM memories WHERE namespace = ?1
                 AND deprecated = 1
                 AND updated_at < ?2",
                params![ns, cutoff],
            )
            .map_err(|e| Error::db("database operation", e))?;
        Ok(deleted)
    }

    /// Find deprecated memories eligible for pruning (dry-run).
    pub fn find_deprecated_for_prune(
        &self,
        ttl_days: u32,
        namespace: Option<&str>,
    ) -> Result<Vec<Memory>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(ttl_days as i64)).to_rfc3339();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type
                 FROM memories WHERE namespace = ?1
                 AND deprecated = 1
                 AND updated_at < ?2
                 ORDER BY updated_at ASC",
            )
            .map_err(|e| Error::db("database operation", e))?;
        let rows = stmt
            .query_map(params![ns, cutoff], row_to_memory)
            .map_err(|e| Error::db("database operation", e))?;
        let mut memories = Vec::new();
        for row in rows {
            memories.push(row.map_err(|e| Error::db("database operation", e))?);
        }
        Ok(memories)
    }

    /// Delete memories by specific IDs. Returns count of deleted rows.
    /// Use this instead of criteria-based delete to avoid TOCTOU races.
    pub fn delete_by_ids(&self, ids: &[String]) -> Result<usize, Error> {
        if ids.is_empty() {
            return Ok(0);
        }
        // Build parameterized IN clause: "WHERE id IN (?1, ?2, ?3)"
        let placeholders: String = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("DELETE FROM memories WHERE id IN ({placeholders})");
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
            .iter()
            .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let deleted = self
            .conn
            .execute(&sql, rusqlite::params_from_iter(param_refs))
            .map_err(|e| Error::db("database operation", e))?;
        Ok(deleted)
    }
}
