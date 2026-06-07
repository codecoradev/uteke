//! Aging and access tracking — touch access, find/cleanup aged, tier counts.

use crate::memory::types::{Memory, DEFAULT_NAMESPACE};
use crate::Error;
use rusqlite::params;

use super::store::row_to_memory;

impl super::Store {
    /// Increment access count and update last_accessed for a memory.
    pub fn touch_access(&self, id: &str) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE memories SET access_count = access_count + 1, last_accessed = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| Error::db("database operation", e))?;
        Ok(())
    }

    /// Find aged memories eligible for cleanup.
    ///
    /// Returns memories matching: older than `older_than_days`, access_count <= max_access_count,
    /// and last_accessed older than `older_than_days` (or never accessed).
    pub fn find_aged(
        &self,
        older_than_days: u32,
        max_access_count: u32,
        namespace: Option<&str>,
    ) -> Result<Vec<Memory>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let sql = r#"
            SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type
            FROM memories
            WHERE namespace = ?1
              AND deprecated = 0
              AND created_at < datetime('now', '-' || ?2 || ' days')
              AND access_count <= ?3
              AND (last_accessed IS NULL OR last_accessed < datetime('now', '-' || ?4 || ' days'))
            ORDER BY created_at ASC
        "#;

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("database operation", e))?;

        let rows = stmt
            .query_map(
                params![ns, older_than_days, max_access_count, older_than_days],
                row_to_memory,
            )
            .map_err(|e| Error::db("database operation", e))?;

        let mut memories = Vec::new();
        for row in rows {
            let m = row.map_err(|e| Error::db("database operation", e))?;
            memories.push(m);
        }
        Ok(memories)
    }

    /// Delete aged memories from SQLite. Returns count of deleted rows.
    ///
    /// Same criteria as `find_aged`. Does NOT touch the vector index.
    pub fn cleanup_aged(
        &self,
        older_than_days: u32,
        max_access_count: u32,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let sql = r#"
            DELETE FROM memories
            WHERE namespace = ?1
              AND created_at < datetime('now', '-' || ?2 || ' days')
              AND access_count <= ?3
              AND (last_accessed IS NULL OR last_accessed < datetime('now', '-' || ?4 || ' days'))
        "#;

        let deleted = self
            .conn
            .execute(
                sql,
                params![ns, older_than_days, max_access_count, older_than_days],
            )
            .map_err(|e| Error::db("database operation", e))?;
        Ok(deleted)
    }

    /// Count memories never accessed in a namespace.
    pub fn count_never_accessed(&self, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let count: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND last_accessed IS NULL",
                params![ns],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("database operation", e))?;
        Ok(count)
    }

    /// Count memories by tier (hot/warm/cold) for a namespace.
    pub fn tier_counts(
        &self,
        namespace: Option<&str>,
        hot_days: i64,
        warm_days: i64,
    ) -> Result<(usize, usize, usize), Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let now = chrono::Utc::now();
        let hot_cutoff = (now - chrono::Duration::days(hot_days)).to_rfc3339();
        let warm_cutoff = (now - chrono::Duration::days(warm_days)).to_rfc3339();

        let hot: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND last_accessed >= ?2",
                params![ns, hot_cutoff],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("database operation", e))?;

        let warm: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND last_accessed >= ?2 AND last_accessed < ?3",
                params![ns, warm_cutoff, hot_cutoff],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("database operation", e))?;

        let cold: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND (last_accessed < ?2 OR last_accessed IS NULL)",
                params![ns, warm_cutoff],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("database operation", e))?;

        Ok((hot, warm, cold))
    }
}
