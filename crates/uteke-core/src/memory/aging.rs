//! Aging and access tracking — touch access, find/cleanup aged, tier counts.

use crate::Error;
use crate::memory::store::memory_columns;
use crate::memory::types::{DEFAULT_NAMESPACE, Memory};
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

    /// Batch-increment `access_count` for memories fetched explicitly.
    pub fn touch_access_batch(&self, ids: &[&str]) -> Result<(), Error> {
        self.touch_batch(ids, "access_count")
    }

    /// Batch-increment `recall_count` for memories returned by recall/search
    /// (#1337). Kept apart from `access_count` so automated recalls weigh less
    /// than explicit use in salience, aging and orphan scoring.
    pub fn touch_recall_batch(&self, ids: &[&str]) -> Result<(), Error> {
        self.touch_batch(ids, "recall_count")
    }

    /// One transaction for the whole batch (no N+1 UPDATEs). `column` is one of
    /// two internal literals, never user input.
    fn touch_batch(&self, ids: &[&str], column: &'static str) -> Result<(), Error> {
        debug_assert!(matches!(column, "access_count" | "recall_count"));
        if ids.is_empty() {
            return Ok(());
        }
        let sql = format!(
            "UPDATE memories SET {column} = {column} + 1, last_accessed = ?1 WHERE id = ?2"
        );
        let now = chrono::Utc::now().to_rfc3339();
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin touch batch transaction", e))?;
        {
            let mut stmt = tx
                .prepare(&sql)
                .map_err(|e| Error::db("prepare touch batch", e))?;
            for id in ids {
                stmt.execute(params![now, id])
                    .map_err(|e| Error::db("touch batch execute", e))?;
            }
        }
        tx.commit()
            .map_err(|e| Error::db("commit touch batch", e))?;
        Ok(())
    }

    /// Count active (non-deprecated) memories in a namespace.
    pub fn count_active(&self, namespace: Option<&str>) -> Result<usize, Error> {
        let count: i64 = match namespace {
            Some(ns) => self.conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND deprecated = 0",
                params![ns],
                |row| row.get(0),
            ),
            None => self.conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE deprecated = 0",
                [],
                |row| row.get(0),
            ),
        }
        .map_err(|e| Error::db("database operation", e))?;
        Ok(count as usize)
    }

    /// Count deprecated memories in a namespace.
    pub fn count_deprecated(&self, namespace: Option<&str>) -> Result<usize, Error> {
        let count: i64 = match namespace {
            Some(ns) => self.conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND deprecated = 1",
                params![ns],
                |row| row.get(0),
            ),
            None => self.conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE deprecated = 1",
                [],
                |row| row.get(0),
            ),
        }
        .map_err(|e| Error::db("database operation", e))?;
        Ok(count as usize)
    }

    /// List deprecated memories with metadata for TTL display.
    ///
    /// Returns memories that are deprecated=1, ordered by most recently deprecated first.
    /// Optionally filter by namespace.
    pub fn list_deprecated(
        &self,
        namespace: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DeprecatedMemoryInfo>, Error> {
        let limit = limit.max(1) as i64;
        let sql = if namespace.is_some() {
            r#"SELECT id, content, memory_type, namespace, tags, importance,
                      valid_until, deprecate_reason, updated_at
               FROM memories
               WHERE deprecated = 1 AND namespace = ?1
               ORDER BY updated_at DESC
               LIMIT ?2"#
        } else {
            r#"SELECT id, content, memory_type, namespace, tags, importance,
                      valid_until, deprecate_reason, updated_at
               FROM memories
               WHERE deprecated = 1
               ORDER BY updated_at DESC
               LIMIT ?1"#
        };

        let ns = namespace.map(|s| s.to_string());
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("prepare list_deprecated", e))?;

        let rows = match &ns {
            Some(ns_val) => stmt.query_map(params![ns_val, limit], dep_row_to_info),
            None => stmt.query_map(params![limit], dep_row_to_info),
        }
        .map_err(|e| Error::db("query list_deprecated", e))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| Error::db("row list_deprecated", e))?);
        }
        Ok(results)
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
        // Compute cutoffs in Rust using chrono (RFC3339) to match stored timestamp format.
        // SQLite datetime('now') returns a different format than our RFC3339 strings,
        // causing lexicographic comparison to fail.
        let cutoff =
            (chrono::Utc::now() - chrono::Duration::days(older_than_days as i64)).to_rfc3339();
        let sql = concat!(
            "SELECT ",
            memory_columns!(),
            r#"
                 FROM memories
            WHERE namespace = ?1
              AND deprecated = 0
              AND pinned = 0
              AND created_at < ?2
              AND access_count <= ?3
              AND (last_accessed IS NULL OR last_accessed < ?4)
            ORDER BY created_at ASC
        "#
        );

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("database operation", e))?;

        let rows = stmt
            .query_map(params![ns, cutoff, max_access_count, cutoff], row_to_memory)
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
    /// Same criteria as `find_aged` (including `deprecated = 0` filter).
    /// Does NOT touch the vector index.
    pub fn cleanup_aged(
        &self,
        older_than_days: u32,
        max_access_count: u32,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let cutoff =
            (chrono::Utc::now() - chrono::Duration::days(older_than_days as i64)).to_rfc3339();
        let sql = r#"
            DELETE FROM memories
            WHERE namespace = ?1
              AND deprecated = 0
              AND pinned = 0
              AND created_at < ?2
              AND access_count <= ?3
              AND (last_accessed IS NULL OR last_accessed < ?4)
        "#;

        let deleted = self
            .conn
            .execute(sql, params![ns, cutoff, max_access_count, cutoff])
            .map_err(|e| Error::db("database operation", e))?;
        Ok(deleted)
    }

    /// Count memories never accessed in a namespace.
    pub fn count_never_accessed(&self, namespace: Option<&str>) -> Result<usize, Error> {
        let count: usize = match namespace {
            Some(ns) => self.conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND last_accessed IS NULL",
                params![ns],
                |row| row.get::<_, i64>(0),
            ),
            None => self.conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE last_accessed IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            ),
        }
        .map_err(|e| Error::db("database operation", e))? as usize;
        Ok(count)
    }

    /// Count memories by tier (hot/warm/cold) for a namespace.
    pub fn tier_counts(
        &self,
        namespace: Option<&str>,
        hot_days: i64,
        warm_days: i64,
    ) -> Result<(usize, usize, usize), Error> {
        let now = chrono::Utc::now();
        let hot_cutoff = (now - chrono::Duration::days(hot_days)).to_rfc3339();
        let warm_cutoff = (now - chrono::Duration::days(warm_days)).to_rfc3339();

        let (hot, warm, cold) = match namespace {
            Some(ns) => {
                let hot: usize = self.conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND last_accessed >= ?2",
                    params![ns, hot_cutoff],
                    |row| row.get::<_, i64>(0),
                ).map_err(|e| Error::db("database operation", e))? as usize;

                let warm: usize = self.conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND last_accessed >= ?2 AND last_accessed < ?3",
                    params![ns, warm_cutoff, hot_cutoff],
                    |row| row.get::<_, i64>(0),
                ).map_err(|e| Error::db("database operation", e))? as usize;

                let cold: usize = self.conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND (last_accessed < ?2 OR last_accessed IS NULL)",
                    params![ns, warm_cutoff],
                    |row| row.get::<_, i64>(0),
                ).map_err(|e| Error::db("database operation", e))? as usize;

                (hot, warm, cold)
            }
            None => {
                let hot: usize = self
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM memories WHERE last_accessed >= ?1",
                        params![hot_cutoff],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|e| Error::db("database operation", e))?
                    as usize;

                let warm: usize = self.conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE last_accessed >= ?1 AND last_accessed < ?2",
                    params![warm_cutoff, hot_cutoff],
                    |row| row.get::<_, i64>(0),
                ).map_err(|e| Error::db("database operation", e))? as usize;

                let cold: usize = self.conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE last_accessed < ?1 OR last_accessed IS NULL",
                    params![warm_cutoff],
                    |row| row.get::<_, i64>(0),
                ).map_err(|e| Error::db("database operation", e))? as usize;

                (hot, warm, cold)
            }
        };

        Ok((hot, warm, cold))
    }
}

/// Lightweight info about a deprecated memory, for lifecycle UI display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeprecatedMemoryInfo {
    pub id: String,
    pub content: String,
    pub memory_type: String,
    pub namespace: String,
    pub tags: Vec<String>,
    pub importance: f64,
    pub deprecated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deprecate_reason: Option<String>,
}

fn dep_row_to_info(row: &rusqlite::Row<'_>) -> Result<DeprecatedMemoryInfo, rusqlite::Error> {
    let id: String = row.get(0)?;
    let content: String = row.get(1)?;
    let memory_type: String = row.get(2).unwrap_or_else(|_| "fact".to_string());
    let namespace: String = row.get(3).unwrap_or_else(|_| "default".to_string());
    let tags_str: Option<String> = row.get(4).ok().flatten();
    let tags = tags_str
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    let importance: f64 = row.get(5).unwrap_or(0.5);
    let valid_until_str: Option<String> = row.get(6).ok().flatten();
    let deprecated_at = valid_until_str
        .as_deref()
        .and_then(super::store::parse_datetime_opt);
    let deprecate_reason: Option<String> = row.get(7).ok().flatten();
    let _updated_at_str: Option<String> = row.get(8).ok().flatten();

    Ok(DeprecatedMemoryInfo {
        id,
        content,
        memory_type,
        namespace,
        tags,
        importance,
        deprecated_at,
        deprecate_reason,
    })
}
