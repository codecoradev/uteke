//! Bulk operations — bulk delete, deprecation, TTL pruning, similarity search.

use crate::memory::types::{Memory, DEFAULT_NAMESPACE};
use crate::Error;
use rusqlite::params;

use super::store::row_to_memory;

impl super::Store {
    /// Bulk delete memories by tag within a namespace.
    pub fn bulk_delete_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2)")
            .map_err(|e| Error::db("database operation", e))?
            .query_map(rusqlite::params![ns, tag], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| r.ok())
            .collect();
        for id in &ids {
            self.conn
                .execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| Error::db("database operation", e))?;
        }
        Ok(ids)
    }

    /// Bulk delete all cold memories (not accessed in warm_days+ days or never accessed).
    pub fn bulk_delete_cold(
        &self,
        namespace: Option<&str>,
        warm_days: i64,
    ) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let warm_cutoff = (chrono::Utc::now() - chrono::Duration::days(warm_days)).to_rfc3339();
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1 AND (last_accessed < ?2 OR last_accessed IS NULL)")
            .map_err(|e| Error::db("database operation", e))?
            .query_map(rusqlite::params![ns, warm_cutoff], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| r.ok())
            .collect();
        for id in &ids {
            self.conn
                .execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| Error::db("database operation", e))?;
        }
        Ok(ids)
    }

    /// Bulk delete all memories in a namespace.
    pub fn bulk_delete_all(&self, namespace: Option<&str>) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1")
            .map_err(|e| Error::db("database operation", e))?
            .query_map(rusqlite::params![ns], |row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| r.ok())
            .collect();
        for id in &ids {
            self.conn
                .execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| Error::db("database operation", e))?;
        }
        Ok(ids)
    }

    /// Deprecate a memory by ID. Sets deprecated=1 and valid_until=now.
    pub fn deprecate(&self, id: &str) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE memories SET deprecated = 1, valid_until = ?1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| Error::db("database operation", e))?;
        Ok(())
    }

    /// Find memories that contradict a new embedding (high similarity, same namespace).
    /// Returns memories with cosine similarity > threshold that are not already deprecated.
    pub fn find_similar(&self, namespace: &str, limit: usize) -> Result<Vec<Memory>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type
                 FROM memories WHERE namespace = ?1 AND deprecated = 0 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| Error::db("database operation", e))?;
        let rows = stmt
            .query_map(params![namespace, limit], row_to_memory)
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
        let deleted = self
            .conn
            .execute(
                "DELETE FROM memories WHERE namespace = ?1
                 AND deprecated = 1
                 AND datetime(updated_at) < datetime('now', '-' || ?2 || ' days')",
                params![ns, ttl_days],
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
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type
                 FROM memories WHERE namespace = ?1
                 AND deprecated = 1
                 AND datetime(updated_at) < datetime('now', '-' || ?2 || ' days')
                 ORDER BY updated_at ASC",
            )
            .map_err(|e| Error::db("database operation", e))?;
        let rows = stmt
            .query_map(params![ns, ttl_days], row_to_memory)
            .map_err(|e| Error::db("database operation", e))?;
        let mut memories = Vec::new();
        for row in rows {
            memories.push(row.map_err(|e| Error::db("database operation", e))?);
        }
        Ok(memories)
    }
}
