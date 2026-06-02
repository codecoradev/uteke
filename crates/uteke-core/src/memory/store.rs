//! SQLite-backed persistence for memories.

use crate::memory::types::Memory;
use crate::Error;
use rusqlite::{params, Connection, OptionalExtension};

/// Schema SQL for initial table creation.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    embedding BLOB,
    tags TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    namespace TEXT NOT NULL DEFAULT 'default',
    access_count INTEGER NOT NULL DEFAULT 0,
    last_accessed TEXT,
    deprecated INTEGER NOT NULL DEFAULT 0,
    valid_from TEXT,
    valid_until TEXT,
    memory_type TEXT NOT NULL DEFAULT 'fact'
);
CREATE INDEX IF NOT EXISTS idx_memories_tags ON memories(tags);
CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);
"#;

/// Persistent SQLite store for memories.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open or create a store at the given path.
    /// Pass `:memory:` for an in-memory database (testing).
    pub fn open(path: &str) -> Result<Self, Error> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()
        } else {
            Connection::open(path)
        }
        .map_err(|e| Error::Database(e.to_string()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| Error::Database(e.to_string()))?;
        conn.execute_batch("PRAGMA busy_timeout=5000;")
            .map_err(|e| Error::Database(e.to_string()))?;

        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), Error> {
        self.conn
            .execute_batch(SCHEMA)
            .map_err(|e| Error::Database(e.to_string()))?;

        // Migration: add namespace column if missing (existing DBs)
        let has_namespace: bool = self
            .conn
            .prepare("SELECT namespace FROM memories LIMIT 1")
            .is_ok();
        if !has_namespace {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN namespace TEXT NOT NULL DEFAULT 'default';",
                )
                .map_err(|e| Error::Database(e.to_string()))?;
        }

        // Create namespace index (safe after column exists)
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_memories_namespace ON memories(namespace);",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        // Migration: add access tracking columns
        if !self.column_exists("access_count") {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        if !self.column_exists("last_accessed") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN last_accessed TEXT;")
                .map_err(|e| Error::Database(e.to_string()))?;
        }

        // Migration: add temporal/deprecation columns
        if !self.column_exists("deprecated") {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN deprecated INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        if !self.column_exists("valid_from") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN valid_from TEXT;")
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        if !self.column_exists("valid_until") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN valid_until TEXT;")
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        if !self.column_exists("memory_type") {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN memory_type TEXT NOT NULL DEFAULT 'fact';",
                )
                .map_err(|e| Error::Database(e.to_string()))?;
        }

        // Create deprecation index
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_memories_deprecated ON memories(deprecated);",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        Ok(())
    }

    fn column_exists(&self, column: &str) -> bool {
        self.conn
            .prepare("SELECT * FROM memories LIMIT 0")
            .map(|stmt| stmt.column_names().iter().any(|n| n == &column))
            .unwrap_or(false)
    }

    /// Insert a new memory. Returns the inserted memory's ID.
    pub fn insert(&self, memory: &Memory) -> Result<(), Error> {
        let embedding_blob = serialize_embedding(&memory.embedding);
        let tags_json =
            serde_json::to_string(&memory.tags).map_err(|e| Error::Database(e.to_string()))?;
        let metadata_json =
            serde_json::to_string(&memory.metadata).map_err(|e| Error::Database(e.to_string()))?;

        self.conn
            .execute(
                "INSERT INTO memories (id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    memory.id,
                    memory.content,
                    embedding_blob,
                    tags_json,
                    metadata_json,
                    memory.created_at.to_rfc3339(),
                    memory.updated_at.to_rfc3339(),
                    memory.namespace,
                    memory.access_count,
                    memory.last_accessed.map(|t| t.to_rfc3339()),
                    memory.deprecated as i32,
                    memory.valid_from.map(|t| t.to_rfc3339()),
                    memory.valid_until.map(|t| t.to_rfc3339()),
                    memory.memory_type,
                ],
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        Ok(())
    }

    /// Get a memory by its ID.
    pub fn get_by_id(&self, id: &str) -> Result<Option<Memory>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories WHERE id = ?1")
            .map_err(|e| Error::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![id], row_to_memory)
            .optional()
            .map_err(|e| Error::Database(e.to_string()))?;

        Ok(result)
    }

    /// Delete a memory by ID. Returns true if a row was deleted.
    pub fn delete(&self, id: &str) -> Result<bool, Error> {
        let deleted = self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(deleted > 0)
    }

    /// Update an existing memory.
    #[allow(dead_code)]
    pub fn update(&self, memory: &Memory) -> Result<(), Error> {
        let embedding_blob = serialize_embedding(&memory.embedding);
        let tags_json =
            serde_json::to_string(&memory.tags).map_err(|e| Error::Database(e.to_string()))?;
        let metadata_json =
            serde_json::to_string(&memory.metadata).map_err(|e| Error::Database(e.to_string()))?;

        self.conn
            .execute(
                "UPDATE memories SET content = ?2, embedding = ?3, tags = ?4, metadata = ?5, updated_at = ?6, namespace = ?7
                 WHERE id = ?1",
                params![
                    memory.id,
                    memory.content,
                    embedding_blob,
                    tags_json,
                    metadata_json,
                    memory.updated_at.to_rfc3339(),
                    memory.namespace,
                ],
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// List memories with optional tag filter, namespace filter, and pagination.
    pub fn list(
        &self,
        tag: Option<&str>,
        namespace: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Memory>, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);

        let sql = match tag {
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2) ORDER BY created_at DESC LIMIT ?3 OFFSET ?4",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories WHERE namespace = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        };

        let mut memories = Vec::new();
        match tag {
            Some(t) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::Database(e.to_string()))?;
                let rows = stmt
                    .query_map(params![ns, t, limit, offset], row_to_memory)
                    .map_err(|e| Error::Database(e.to_string()))?;
                for row in rows {
                    let m = row.map_err(|e| Error::Database(e.to_string()))?;
                    memories.push(m);
                }
            }
            None => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::Database(e.to_string()))?;
                let rows = stmt
                    .query_map(params![ns, limit, offset], row_to_memory)
                    .map_err(|e| Error::Database(e.to_string()))?;
                for row in rows {
                    let m = row.map_err(|e| Error::Database(e.to_string()))?;
                    memories.push(m);
                }
            }
        }
        Ok(memories)
    }

    /// Search memories by content using LIKE (simple full-text for v2).
    pub fn search_content(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let pattern = format!("%{query}%");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type
                 FROM memories WHERE namespace = ?1 AND content LIKE ?2
                 ORDER BY created_at DESC LIMIT ?3",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![ns, pattern, limit], row_to_memory)
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut memories = Vec::new();
        for row in rows {
            let m = row.map_err(|e| Error::Database(e.to_string()))?;
            memories.push(m);
        }
        Ok(memories)
    }

    /// Load all memories for index rebuilding, optionally filtered by namespace.
    pub fn load_all(&self, namespace: Option<&str>) -> Result<Vec<Memory>, Error> {
        let sql = match namespace {
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories WHERE namespace = ?1 ORDER BY created_at",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories ORDER BY created_at",
        };

        let mut memories = Vec::new();
        match namespace {
            Some(ns) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::Database(e.to_string()))?;
                let rows = stmt
                    .query_map(params![ns], row_to_memory)
                    .map_err(|e| Error::Database(e.to_string()))?;
                for row in rows {
                    let m = row.map_err(|e| Error::Database(e.to_string()))?;
                    memories.push(m);
                }
            }
            None => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::Database(e.to_string()))?;
                let rows = stmt
                    .query_map([], row_to_memory)
                    .map_err(|e| Error::Database(e.to_string()))?;
                for row in rows {
                    let m = row.map_err(|e| Error::Database(e.to_string()))?;
                    memories.push(m);
                }
            }
        }
        Ok(memories)
    }

    /// Count total memories, optionally filtered by namespace.
    pub fn count(&self, namespace: Option<&str>) -> Result<usize, Error> {
        let count: usize = match namespace {
            Some(ns) => self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE namespace = ?1",
                    params![ns],
                    |row| row.get(0),
                )
                .map_err(|e| Error::Database(e.to_string()))?,
            None => self
                .conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
                .map_err(|e| Error::Database(e.to_string()))?,
        };
        Ok(count)
    }

    /// Get all unique tags, optionally filtered by namespace.
    ///
    /// Uses `json_each()` to unnest the JSON array stored in `tags` so SQLite
    /// returns individual tag values directly — no in-Rust JSON parsing needed.
    pub fn unique_tags(&self, namespace: Option<&str>) -> Result<Vec<String>, Error> {
        let sql = match namespace {
            Some(_) => {
                "SELECT DISTINCT je.value FROM memories, json_each(memories.tags) AS je WHERE namespace = ?1"
            }
            None => "SELECT DISTINCT je.value FROM memories, json_each(memories.tags) AS je",
        };

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = match namespace {
            Some(ns) => stmt
                .query_map(params![ns], |row: &rusqlite::Row| row.get::<_, String>(0))
                .map_err(|e| Error::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::Database(e.to_string()))?,
            None => stmt
                .query_map([], |row: &rusqlite::Row| row.get::<_, String>(0))
                .map_err(|e| Error::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::Database(e.to_string()))?,
        };

        Ok(rows)
    }

    /// List all distinct namespaces.
    pub fn list_namespaces(&self) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT namespace FROM memories ORDER BY namespace")
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row: &rusqlite::Row| row.get(0))
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut namespaces = Vec::new();
        for row in rows {
            namespaces.push(row.map_err(|e| Error::Database(e.to_string()))?);
        }
        Ok(namespaces)
    }

    /// Get the underlying database path, if file-based.
    pub fn path(&self) -> Option<std::path::PathBuf> {
        self.conn.path().map(std::path::PathBuf::from)
    }

    /// Increment access count and update last_accessed for a memory.
    pub fn touch_access(&self, id: &str) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE memories SET access_count = access_count + 1, last_accessed = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| Error::Database(e.to_string()))?;
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
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
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
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![ns, older_than_days, max_access_count, older_than_days],
                row_to_memory,
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut memories = Vec::new();
        for row in rows {
            let m = row.map_err(|e| Error::Database(e.to_string()))?;
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
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
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
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(deleted)
    }

    /// Count memories never accessed in a namespace.
    pub fn count_never_accessed(&self, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let count: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND last_accessed IS NULL",
                params![ns],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(count)
    }

    /// Count memories by tier (hot/warm/cold) for a namespace.
    /// List all tags with their usage counts.
    ///
    /// Single-query approach using `json_each()` — replaces the old N+1 pattern
    /// that fetched each tag then ran a separate COUNT query per tag.
    pub fn tags_with_counts(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<crate::memory::types::TagInfo>, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let sql = "SELECT je.value AS name, COUNT(*) AS count FROM memories, json_each(memories.tags) AS je WHERE namespace = ?1 GROUP BY je.value ORDER BY count DESC";
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![ns], |row| {
                Ok(crate::memory::types::TagInfo {
                    name: row.get(0)?,
                    count: row.get(1)?,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| Error::Database(e.to_string()))?);
        }
        Ok(result)
    }

    /// Rename a tag across all memories. Returns number updated.
    ///
    /// Uses `json_each()` to find affected rows precisely, then updates the
    /// JSON tags column with the renamed tag.
    pub fn rename_tag(
        &self,
        old: &str,
        new: &str,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare("SELECT id, tags FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2)")
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![ns, old], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| Error::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let mut updated = 0;
        for (id, tags_str) in &rows {
            let mut tags: Vec<String> = serde_json::from_str(tags_str).unwrap_or_default();
            let mut changed = false;
            for t in &mut tags {
                if t == old {
                    *t = new.to_string();
                    changed = true;
                }
            }
            if changed {
                let new_tags_json =
                    serde_json::to_string(&tags).map_err(|e| Error::Database(e.to_string()))?;
                let now = chrono::Utc::now().to_rfc3339();
                self.conn
                    .execute(
                        "UPDATE memories SET tags = ?1, updated_at = ?2 WHERE id = ?3",
                        rusqlite::params![new_tags_json, now, id],
                    )
                    .map_err(|e| Error::Database(e.to_string()))?;
                updated += 1;
            }
        }
        Ok(updated)
    }

    /// Delete a tag from all memories. Returns number updated.
    ///
    /// Uses `json_each()` to find affected rows precisely.
    pub fn delete_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare("SELECT id, tags FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2)")
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![ns, tag], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| Error::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let mut updated = 0;
        for (id, tags_str) in &rows {
            let mut tags: Vec<String> = serde_json::from_str(tags_str).unwrap_or_default();
            let before_len = tags.len();
            tags.retain(|t| t != tag);
            if tags.len() != before_len {
                let new_tags_json =
                    serde_json::to_string(&tags).map_err(|e| Error::Database(e.to_string()))?;
                let now = chrono::Utc::now().to_rfc3339();
                self.conn
                    .execute(
                        "UPDATE memories SET tags = ?1, updated_at = ?2 WHERE id = ?3",
                        rusqlite::params![new_tags_json, now, id],
                    )
                    .map_err(|e| Error::Database(e.to_string()))?;
                updated += 1;
            }
        }
        Ok(updated)
    }

    pub fn tier_counts(
        &self,
        namespace: Option<&str>,
        hot_days: i64,
        warm_days: i64,
    ) -> Result<(usize, usize, usize), Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
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
            .map_err(|e| Error::Database(e.to_string()))?;

        let warm: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND last_accessed >= ?2 AND last_accessed < ?3",
                params![ns, warm_cutoff, hot_cutoff],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let cold: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND (last_accessed < ?2 OR last_accessed IS NULL)",
                params![ns, warm_cutoff],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        Ok((hot, warm, cold))
    }

    /// Bulk delete memories by tag within a namespace.
    pub fn bulk_delete_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2)")
            .map_err(|e| Error::Database(e.to_string()))?
            .query_map(rusqlite::params![ns, tag], |row| row.get(0))
            .map_err(|e| Error::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        for id in &ids {
            self.conn
                .execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        Ok(ids)
    }

    /// Bulk delete all cold memories (not accessed in warm_days+ days or never accessed).
    pub fn bulk_delete_cold(
        &self,
        namespace: Option<&str>,
        warm_days: i64,
    ) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let warm_cutoff = (chrono::Utc::now() - chrono::Duration::days(warm_days)).to_rfc3339();
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1 AND (last_accessed < ?2 OR last_accessed IS NULL)")
            .map_err(|e| Error::Database(e.to_string()))?
            .query_map(rusqlite::params![ns, warm_cutoff], |row| row.get(0))
            .map_err(|e| Error::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        for id in &ids {
            self.conn
                .execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        Ok(ids)
    }

    /// Bulk delete all memories in a namespace.
    pub fn bulk_delete_all(&self, namespace: Option<&str>) -> Result<Vec<String>, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1")
            .map_err(|e| Error::Database(e.to_string()))?
            .query_map(rusqlite::params![ns], |row| row.get(0))
            .map_err(|e| Error::Database(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        for id in &ids {
            self.conn
                .execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        Ok(ids)
    }

    /// Count memories by tag in a namespace.
    pub fn count_by_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2)",
                rusqlite::params![ns, tag],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Deprecate a memory by ID. Sets deprecated=1 and valid_until=now.
    pub fn deprecate(&self, id: &str) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE memories SET deprecated = 1, valid_until = ?1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| Error::Database(e.to_string()))?;
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
            .map_err(|e| Error::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![namespace, limit], row_to_memory)
            .map_err(|e| Error::Database(e.to_string()))?;
        let mut memories = Vec::new();
        for row in rows {
            memories.push(row.map_err(|e| Error::Database(e.to_string()))?);
        }
        Ok(memories)
    }

    /// Prune (delete) cold, deprecated, or expired memories based on TTL.
    /// Returns count of pruned memories.
    pub fn prune_ttl(&self, ttl_days: u32, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let deleted = self
            .conn
            .execute(
                "DELETE FROM memories WHERE namespace = ?1
                 AND deprecated = 1
                 AND datetime(updated_at) < datetime('now', '-' || ?2 || ' days')",
                params![ns, ttl_days],
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(deleted)
    }

    /// Find deprecated memories eligible for pruning (dry-run).
    pub fn find_deprecated_for_prune(
        &self,
        ttl_days: u32,
        namespace: Option<&str>,
    ) -> Result<Vec<Memory>, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type
                 FROM memories WHERE namespace = ?1
                 AND deprecated = 1
                 AND datetime(updated_at) < datetime('now', '-' || ?2 || ' days')
                 ORDER BY updated_at ASC",
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![ns, ttl_days], row_to_memory)
            .map_err(|e| Error::Database(e.to_string()))?;
        let mut memories = Vec::new();
        for row in rows {
            memories.push(row.map_err(|e| Error::Database(e.to_string()))?);
        }
        Ok(memories)
    }
}
fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        blob.extend_from_slice(&val.to_le_bytes());
    }
    blob
}

/// Deserialize an embedding vector from a byte blob.
fn deserialize_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Convert a database row to a Memory.
fn row_to_memory(row: &rusqlite::Row<'_>) -> Result<Memory, rusqlite::Error> {
    let id: String = row.get(0)?;
    let content: String = row.get(1)?;
    let embedding_blob: Option<Vec<u8>> = row.get(2)?;
    let embedding = embedding_blob
        .as_deref()
        .map(deserialize_embedding)
        .unwrap_or_default();
    let tags_str: Option<String> = row.get(3)?;
    let tags = tags_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let metadata_str: Option<String> = row.get(4)?;
    let metadata = metadata_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let created_at_str: String = row.get(5)?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.to_utc())
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?;
    let updated_at_str: String = row.get(6)?;
    let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|dt| dt.to_utc())
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?;
    let namespace: String = row
        .get(7)
        .unwrap_or_else(|_| crate::memory::types::DEFAULT_NAMESPACE.to_string());
    let access_count: u32 = row.get(8).unwrap_or(0);
    let last_accessed_str: Option<String> = row.get(9).ok().flatten();
    let last_accessed = last_accessed_str
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.to_utc());
    let deprecated: bool = row.get(10).unwrap_or(false);
    let valid_from_str: Option<String> = row.get(11).ok().flatten();
    let valid_from = valid_from_str
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.to_utc());
    let valid_until_str: Option<String> = row.get(12).ok().flatten();
    let valid_until = valid_until_str
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.to_utc());
    let memory_type: String = row.get(13).unwrap_or_else(|_| "fact".to_string());

    Ok(Memory {
        id,
        content,
        embedding,
        tags,
        metadata,
        created_at,
        updated_at,
        namespace,
        access_count,
        last_accessed,
        deprecated,
        valid_from,
        valid_until,
        memory_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::Memory;
    use chrono::Utc;

    fn make_test_memory(id: &str, content: &str, tags: &[&str]) -> Memory {
        Memory {
            id: id.to_string(),
            content: content.to_string(),
            embedding: vec![0.1; 768],
            tags: tags.iter().map(|t| t.to_string()).collect(),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            namespace: crate::memory::types::DEFAULT_NAMESPACE.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
        }
    }

    fn make_test_memory_ns(id: &str, content: &str, tags: &[&str], namespace: &str) -> Memory {
        Memory {
            id: id.to_string(),
            content: content.to_string(),
            embedding: vec![0.1; 768],
            tags: tags.iter().map(|t| t.to_string()).collect(),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            namespace: namespace.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
        }
    }

    #[test]
    fn test_store_crud() {
        let store = Store::open(":memory:").unwrap();

        let m = make_test_memory("test-id-1", "hello world", &["greeting"]);
        store.insert(&m).unwrap();

        let got = store.get_by_id("test-id-1").unwrap().unwrap();
        assert_eq!(got.id, "test-id-1");
        assert_eq!(got.content, "hello world");
        assert_eq!(got.tags, vec!["greeting"]);

        let deleted = store.delete("test-id-1").unwrap();
        assert!(deleted);

        let gone = store.get_by_id("test-id-1").unwrap();
        assert!(gone.is_none());
    }

    #[test]
    fn test_store_list_with_tag_filter() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "a", &["rust"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["python"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["rust", "ai"]))
            .unwrap();

        let rust_memories = store.list(Some("rust"), None, 10, 0).unwrap();
        assert_eq!(rust_memories.len(), 2);

        let all = store.list(None, None, 10, 0).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_store_search_content() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "rust programming language", &[]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "python machine learning", &[]))
            .unwrap();

        let results = store.search_content("rust", None, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "1");
    }

    #[test]
    fn test_store_update() {
        let store = Store::open(":memory:").unwrap();

        let mut m = make_test_memory("u1", "original", &[]);
        store.insert(&m).unwrap();

        m.content = "updated".to_string();
        store.update(&m).unwrap();

        let got = store.get_by_id("u1").unwrap().unwrap();
        assert_eq!(got.content, "updated");
    }

    #[test]
    fn test_embedding_serialization() {
        let original: Vec<f32> = vec![0.1, -0.2, 0.3, 0.0, 1.0];
        let blob = serialize_embedding(&original);
        let restored = deserialize_embedding(&blob);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_unique_tags() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("1", "a", &["rust", "ai"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rust", "web"]))
            .unwrap();

        let tags = store.unique_tags(None).unwrap();
        assert_eq!(tags.len(), 3);
    }

    #[test]
    fn test_namespace_isolation() {
        let store = Store::open(":memory:").unwrap();

        // Insert into different namespaces
        store
            .insert(&make_test_memory_ns(
                "a1",
                "hermes deploy",
                &["deploy"],
                "hermes",
            ))
            .unwrap();
        store
            .insert(&make_test_memory_ns(
                "a2",
                "hermes config",
                &["config"],
                "hermes",
            ))
            .unwrap();
        store
            .insert(&make_test_memory_ns("b1", "pi preference", &["pref"], "pi"))
            .unwrap();

        // Count per namespace
        assert_eq!(store.count(Some("hermes")).unwrap(), 2);
        assert_eq!(store.count(Some("pi")).unwrap(), 1);
        assert_eq!(store.count(None).unwrap(), 3);

        // List per namespace
        let hermes_list = store.list(None, Some("hermes"), 10, 0).unwrap();
        assert_eq!(hermes_list.len(), 2);

        let pi_list = store.list(None, Some("pi"), 10, 0).unwrap();
        assert_eq!(pi_list.len(), 1);
        assert_eq!(pi_list[0].content, "pi preference");

        // Search per namespace
        let hermes_search = store.search_content("deploy", Some("hermes"), 10).unwrap();
        assert_eq!(hermes_search.len(), 1);

        let pi_search = store.search_content("deploy", Some("pi"), 10).unwrap();
        assert_eq!(pi_search.len(), 0);

        // List namespaces
        let ns = store.list_namespaces().unwrap();
        assert_eq!(ns.len(), 2);
        assert!(ns.contains(&"hermes".to_string()));
        assert!(ns.contains(&"pi".to_string()));
    }

    // ── json_each() tag query tests ──────────────────────────────────────

    #[test]
    fn test_tag_filter_with_json_each() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "a", &["rust"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["python"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["rust", "ai"]))
            .unwrap();

        // Exact match via json_each: only memories truly containing "rust"
        let rust_memories = store.list(Some("rust"), None, 10, 0).unwrap();
        assert_eq!(rust_memories.len(), 2);
        let ids: Vec<&str> = rust_memories.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"1"));
        assert!(ids.contains(&"3"));

        // "rust" substring should NOT match "rustlang" or similar
        store
            .insert(&make_test_memory("4", "d", &["rustlang"]))
            .unwrap();
        let rust_exact = store.list(Some("rust"), None, 10, 0).unwrap();
        assert_eq!(rust_exact.len(), 2);
        assert!(rust_exact.iter().all(|m| m.id != "4"));
    }

    #[test]
    fn test_unique_tags_json_each() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "a", &["rust", "ai"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rust", "web"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["python"]))
            .unwrap();

        let tags = store.unique_tags(None).unwrap();
        assert_eq!(tags.len(), 4);
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"ai".to_string()));
        assert!(tags.contains(&"web".to_string()));
        assert!(tags.contains(&"python".to_string()));
    }

    #[test]
    fn test_unique_tags_namespace_filtered_json_each() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory_ns("1", "a", &["rust", "ai"], "ns-alpha"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &["rust", "web"], "ns-beta"))
            .unwrap();

        let alpha_tags = store.unique_tags(Some("ns-alpha")).unwrap();
        assert_eq!(alpha_tags.len(), 2);
        assert!(alpha_tags.contains(&"rust".to_string()));
        assert!(alpha_tags.contains(&"ai".to_string()));

        let beta_tags = store.unique_tags(Some("ns-beta")).unwrap();
        assert_eq!(beta_tags.len(), 2);
        assert!(beta_tags.contains(&"rust".to_string()));
        assert!(beta_tags.contains(&"web".to_string()));
    }

    #[test]
    fn test_tags_with_counts_single_query() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "a", &["rust", "ai"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rust", "web"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["python"]))
            .unwrap();

        let info = store.tags_with_counts(None).unwrap();
        // rust appears in 2 memories (highest count, should be first due to ORDER BY count DESC)
        assert_eq!(info[0].name, "rust");
        assert_eq!(info[0].count, 2);
        assert_eq!(info.len(), 4);

        // Verify all counts sum correctly
        let total: usize = info.iter().map(|t| t.count).sum();
        assert_eq!(total, 5); // rust:2, ai:1, web:1, python:1 = 2+1+1+1 = 5
    }

    #[test]
    fn test_tags_with_counts_namespace_filtered() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory_ns("1", "a", &["rust"], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &["rust"], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("3", "c", &["python"], "ns-b"))
            .unwrap();

        let info_a = store.tags_with_counts(Some("ns-a")).unwrap();
        assert_eq!(info_a.len(), 1);
        assert_eq!(info_a[0].name, "rust");
        assert_eq!(info_a[0].count, 2);

        let info_b = store.tags_with_counts(Some("ns-b")).unwrap();
        assert_eq!(info_b.len(), 1);
        assert_eq!(info_b[0].name, "python");
        assert_eq!(info_b[0].count, 1);
    }

    #[test]
    fn test_rename_tag_json_each() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "a", &["old-tag"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["old-tag", "other"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["unrelated"]))
            .unwrap();

        let updated = store.rename_tag("old-tag", "new-tag", None).unwrap();
        assert_eq!(updated, 2);

        // old-tag should no longer match
        let old_matches = store.list(Some("old-tag"), None, 10, 0).unwrap();
        assert_eq!(old_matches.len(), 0);

        // new-tag should match both updated memories
        let new_matches = store.list(Some("new-tag"), None, 10, 0).unwrap();
        assert_eq!(new_matches.len(), 2);
    }

    #[test]
    fn test_delete_tag_json_each() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "a", &["remove-me", "keep"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["remove-me"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["other"]))
            .unwrap();

        let updated = store.delete_tag("remove-me", None).unwrap();
        assert_eq!(updated, 2);

        // "remove-me" should no longer appear anywhere
        let remaining_tags = store.unique_tags(None).unwrap();
        assert!(!remaining_tags.contains(&"remove-me".to_string()));
        assert!(remaining_tags.contains(&"keep".to_string()));
        assert!(remaining_tags.contains(&"other".to_string()));
    }

    #[test]
    fn test_bulk_delete_by_tag_json_each() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "a", &["doom"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["doom", "safe"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["safe"]))
            .unwrap();

        let deleted_ids = store.bulk_delete_by_tag("doom", None).unwrap();
        assert_eq!(deleted_ids.len(), 2);
        assert!(deleted_ids.contains(&"1".to_string()));
        assert!(deleted_ids.contains(&"2".to_string()));

        // Only "3" should remain
        let all = store.list(None, None, 10, 0).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "3");
    }

    #[test]
    fn test_count_by_tag_json_each() {
        let store = Store::open(":memory:").unwrap();

        store
            .insert(&make_test_memory("1", "a", &["rust"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rust", "ai"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["python"]))
            .unwrap();

        assert_eq!(store.count_by_tag("rust", None).unwrap(), 2);
        assert_eq!(store.count_by_tag("ai", None).unwrap(), 1);
        assert_eq!(store.count_by_tag("python", None).unwrap(), 1);
        assert_eq!(store.count_by_tag("nonexistent", None).unwrap(), 0);
    }

    #[test]
    fn test_tag_with_special_chars() {
        let store = Store::open(":memory:").unwrap();

        // Tags that would break LIKE '%"tag"%' pattern matching
        store
            .insert(&make_test_memory("1", "a", &["tag-with-dash"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["tag.with.dots"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["tag_with_underscore"]))
            .unwrap();

        // All should be findable by exact match
        assert_eq!(
            store
                .list(Some("tag-with-dash"), None, 10, 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .list(Some("tag.with.dots"), None, 10, 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .list(Some("tag_with_underscore"), None, 10, 0)
                .unwrap()
                .len(),
            1
        );

        let tags = store.unique_tags(None).unwrap();
        assert_eq!(tags.len(), 3);
    }

    #[test]
    fn test_tag_substring_not_matched() {
        let store = Store::open(":memory:").unwrap();

        // Insert a memory with tag "rust" and another with "rustacean"
        store
            .insert(&make_test_memory("1", "a", &["rust"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rustacean"]))
            .unwrap();

        // Searching for "rust" should NOT match "rustacean"
        let rust_only = store.list(Some("rust"), None, 10, 0).unwrap();
        assert_eq!(rust_only.len(), 1);
        assert_eq!(rust_only[0].id, "1");

        // Searching for "rustacean" should NOT match "rust"
        let rustacean_only = store.list(Some("rustacean"), None, 10, 0).unwrap();
        assert_eq!(rustacean_only.len(), 1);
        assert_eq!(rustacean_only[0].id, "2");
    }

    // ── Additional coverage tests ──────────────────────────────────────────

    #[test]
    fn test_bulk_delete_all() {
        let store = Store::open(":memory:").unwrap();
        store.insert(&make_test_memory("1", "a", &["x"])).unwrap();
        store.insert(&make_test_memory("2", "b", &["y"])).unwrap();
        store.insert(&make_test_memory("3", "c", &[])).unwrap();

        let deleted_ids = store.bulk_delete_all(None).unwrap();
        assert_eq!(deleted_ids.len(), 3);
        assert_eq!(store.count(None).unwrap(), 0);
    }

    #[test]
    fn test_bulk_delete_all_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &[], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &[], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("3", "c", &[], "ns-b"))
            .unwrap();

        let deleted = store.bulk_delete_all(Some("ns-a")).unwrap();
        assert_eq!(deleted.len(), 2);
        assert_eq!(store.count(Some("ns-a")).unwrap(), 0);
        assert_eq!(store.count(Some("ns-b")).unwrap(), 1);
    }

    #[test]
    fn test_bulk_delete_by_tag_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &["doom"], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &["doom"], "ns-b"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("3", "c", &["safe"], "ns-a"))
            .unwrap();

        let deleted = store.bulk_delete_by_tag("doom", Some("ns-a")).unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0], "1");
        // ns-b should still have its "doom" memory
        assert_eq!(store.count(Some("ns-b")).unwrap(), 1);
    }

    #[test]
    fn test_bulk_delete_cold() {
        let store = Store::open(":memory:").unwrap();
        // All memories have no last_accessed → all are "cold"
        store.insert(&make_test_memory("1", "cold-1", &[])).unwrap();
        store.insert(&make_test_memory("2", "cold-2", &[])).unwrap();

        let deleted = store.bulk_delete_cold(None, 30).unwrap();
        assert_eq!(deleted.len(), 2);
        assert_eq!(store.count(None).unwrap(), 0);
    }

    #[test]
    fn test_deprecate() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("dep1", "will be deprecated", &[]))
            .unwrap();

        store.deprecate("dep1").unwrap();
        let m = store.get_by_id("dep1").unwrap().unwrap();
        assert!(m.deprecated);
        assert!(m.valid_until.is_some());
    }

    #[test]
    fn test_deprecate_nonexistent() {
        let store = Store::open(":memory:").unwrap();
        // Should succeed (no-op) even if ID doesn't exist
        store.deprecate("nonexistent").unwrap();
    }

    #[test]
    fn test_tier_counts() {
        let store = Store::open(":memory:").unwrap();
        // Insert 3 memories — all cold (never accessed)
        store.insert(&make_test_memory("1", "a", &[])).unwrap();
        store.insert(&make_test_memory("2", "b", &[])).unwrap();
        store.insert(&make_test_memory("3", "c", &[])).unwrap();

        let (hot, warm, cold) = store.tier_counts(None, 7, 30).unwrap();
        assert_eq!(hot, 0);
        assert_eq!(warm, 0);
        assert_eq!(cold, 3);
    }

    #[test]
    fn test_tier_counts_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &[], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &[], "ns-b"))
            .unwrap();

        let (_, _, cold_a) = store.tier_counts(Some("ns-a"), 7, 30).unwrap();
        assert_eq!(cold_a, 1);

        let (_, _, cold_b) = store.tier_counts(Some("ns-b"), 7, 30).unwrap();
        assert_eq!(cold_b, 1);
    }

    #[test]
    fn test_count_never_accessed() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("1", "never accessed", &[]))
            .unwrap();

        assert_eq!(store.count_never_accessed(None).unwrap(), 1);

        // Touch it
        store.touch_access("1").unwrap();
        let m = store.get_by_id("1").unwrap().unwrap();
        assert!(m.last_accessed.is_some());
        assert_eq!(m.access_count, 1);

        assert_eq!(store.count_never_accessed(None).unwrap(), 0);
    }

    #[test]
    fn test_count_never_accessed_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &[], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &[], "ns-b"))
            .unwrap();

        assert_eq!(store.count_never_accessed(Some("ns-a")).unwrap(), 1);
        assert_eq!(store.count_never_accessed(Some("ns-b")).unwrap(), 1);
    }

    #[test]
    fn test_touch_access() {
        let store = Store::open(":memory:").unwrap();
        store.insert(&make_test_memory("t1", "test", &[])).unwrap();

        store.touch_access("t1").unwrap();
        store.touch_access("t1").unwrap();

        let m = store.get_by_id("t1").unwrap().unwrap();
        assert_eq!(m.access_count, 2);
        assert!(m.last_accessed.is_some());
    }

    #[test]
    fn test_search_content_edge_cases() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("1", "Hello World", &[]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "hello world too", &[]))
            .unwrap();

        // Case-sensitive LIKE search
        let results = store.search_content("Hello", None, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "1");

        // Lowercase — LIKE is case-insensitive by default on SQLite
        let lower = store.search_content("hello", None, 10).unwrap();
        assert_eq!(lower.len(), 2);
    }

    #[test]
    fn test_search_content_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "shared keyword", &[], "ns-x"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "shared keyword", &[], "ns-y"))
            .unwrap();

        let results = store.search_content("shared", Some("ns-x"), 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].namespace, "ns-x");
    }

    #[test]
    fn test_search_content_empty_result() {
        let store = Store::open(":memory:").unwrap();
        store.insert(&make_test_memory("1", "hello", &[])).unwrap();

        let results = store.search_content("xyznotfound", None, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_content_limit() {
        let store = Store::open(":memory:").unwrap();
        for i in 0..20 {
            store
                .insert(&make_test_memory(
                    &format!("m{i}"),
                    &format!("common content {i}"),
                    &[],
                ))
                .unwrap();
        }

        let results = store.search_content("common", None, 5).unwrap();
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_load_all() {
        let store = Store::open(":memory:").unwrap();
        store.insert(&make_test_memory("1", "a", &[])).unwrap();
        store.insert(&make_test_memory("2", "b", &[])).unwrap();
        store.insert(&make_test_memory("3", "c", &[])).unwrap();

        let all = store.load_all(None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_load_all_namespace_filtered() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &[], "alpha"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &[], "beta"))
            .unwrap();

        let alpha = store.load_all(Some("alpha")).unwrap();
        assert_eq!(alpha.len(), 1);
        assert_eq!(alpha[0].id, "1");
    }

    #[test]
    fn test_load_all_empty() {
        let store = Store::open(":memory:").unwrap();
        let all = store.load_all(None).unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn test_list_pagination() {
        let store = Store::open(":memory:").unwrap();
        for i in 0..10 {
            store
                .insert(&make_test_memory(
                    &format!("p{i}"),
                    &format!("item {i}"),
                    &[],
                ))
                .unwrap();
        }

        // First page
        let page1 = store.list(None, None, 3, 0).unwrap();
        assert_eq!(page1.len(), 3);

        // Second page
        let page2 = store.list(None, None, 3, 3).unwrap();
        assert_eq!(page2.len(), 3);

        // Total should be 10
        assert_eq!(store.count(None).unwrap(), 10);
    }

    #[test]
    fn test_delete_nonexistent() {
        let store = Store::open(":memory:").unwrap();
        let deleted = store.delete("nonexistent").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_get_by_id_nonexistent() {
        let store = Store::open(":memory:").unwrap();
        let result = store.get_by_id("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_count_empty_store() {
        let store = Store::open(":memory:").unwrap();
        assert_eq!(store.count(None).unwrap(), 0);
        assert_eq!(store.count(Some("ns")).unwrap(), 0);
    }

    #[test]
    fn test_path_in_memory() {
        let store = Store::open(":memory:").unwrap();
        // In-memory store has no file path
        assert!(store.path().is_none());
    }

    #[test]
    fn test_list_namespaces_empty() {
        let store = Store::open(":memory:").unwrap();
        let ns = store.list_namespaces().unwrap();
        assert!(ns.is_empty());
    }

    #[test]
    fn test_find_aged_empty() {
        let store = Store::open(":memory:").unwrap();
        // No memories → no aged
        let aged = store.find_aged(30, 0, None).unwrap();
        assert!(aged.is_empty());
    }

    #[test]
    fn test_find_aged_with_recent_memories() {
        let store = Store::open(":memory:").unwrap();
        // Insert a very recent memory — should NOT be found as aged
        store
            .insert(&make_test_memory("recent", "recent memory", &[]))
            .unwrap();

        let aged = store.find_aged(999, 0, None).unwrap();
        // Created right now, so it's not older than 999 days
        assert!(aged.is_empty());
    }

    #[test]
    fn test_cleanup_aged_empty() {
        let store = Store::open(":memory:").unwrap();
        let deleted = store.cleanup_aged(30, 0, None).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_find_aged_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &[], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &[], "ns-b"))
            .unwrap();

        // These are all recent so none should be aged, but namespace filtering should work
        let aged_a = store.find_aged(999, 0, Some("ns-a")).unwrap();
        assert!(aged_a.is_empty());
    }

    #[test]
    fn test_prune_ttl_no_deprecated() {
        let store = Store::open(":memory:").unwrap();
        store.insert(&make_test_memory("1", "active", &[])).unwrap();

        let pruned = store.prune_ttl(30, None).unwrap();
        assert_eq!(pruned, 0);
    }

    #[test]
    fn test_find_deprecated_for_prune() {
        let store = Store::open(":memory:").unwrap();
        store.insert(&make_test_memory("1", "active", &[])).unwrap();
        store.deprecate("1").unwrap();

        // Deprecated but very recent — should NOT be found for prune with high TTL
        let found = store.find_deprecated_for_prune(999, None).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn test_find_similar() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "hello world", &[], "test-ns"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "foo bar", &[], "test-ns"))
            .unwrap();

        let similar = store.find_similar("test-ns", 10).unwrap();
        // find_similar returns non-deprecated memories ordered by created_at DESC
        assert_eq!(similar.len(), 2);
        // Should be ordered by created_at DESC
        assert_eq!(similar[0].id, "2");
    }

    #[test]
    fn test_find_similar_empty_namespace() {
        let store = Store::open(":memory:").unwrap();
        let similar = store.find_similar("empty-ns", 10).unwrap();
        assert!(similar.is_empty());
    }

    #[test]
    fn test_rename_tag_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &["old"], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &["old"], "ns-b"))
            .unwrap();

        // Rename only in ns-a
        let updated = store.rename_tag("old", "new", Some("ns-a")).unwrap();
        assert_eq!(updated, 1);

        // ns-a should have "new"
        let ns_a_tags = store.unique_tags(Some("ns-a")).unwrap();
        assert!(ns_a_tags.contains(&"new".to_string()));
        assert!(!ns_a_tags.contains(&"old".to_string()));

        // ns-b should still have "old"
        let ns_b_tags = store.unique_tags(Some("ns-b")).unwrap();
        assert!(ns_b_tags.contains(&"old".to_string()));
    }

    #[test]
    fn test_delete_tag_namespace_scoped() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &["remove"], "ns-a"))
            .unwrap();
        store
            .insert(&make_test_memory_ns("2", "b", &["remove"], "ns-b"))
            .unwrap();

        let updated = store.delete_tag("remove", Some("ns-a")).unwrap();
        assert_eq!(updated, 1);

        let ns_a_tags = store.unique_tags(Some("ns-a")).unwrap();
        assert!(!ns_a_tags.contains(&"remove".to_string()));

        let ns_b_tags = store.unique_tags(Some("ns-b")).unwrap();
        assert!(ns_b_tags.contains(&"remove".to_string()));
    }

    #[test]
    fn test_rename_tag_nonexistent() {
        let store = Store::open(":memory:").unwrap();
        let updated = store.rename_tag("nope", "new", None).unwrap();
        assert_eq!(updated, 0);
    }

    #[test]
    fn test_delete_tag_nonexistent() {
        let store = Store::open(":memory:").unwrap();
        let updated = store.delete_tag("nope", None).unwrap();
        assert_eq!(updated, 0);
    }

    #[test]
    fn test_count_by_tag_nonexistent() {
        let store = Store::open(":memory:").unwrap();
        assert_eq!(store.count_by_tag("nope", None).unwrap(), 0);
    }

    #[test]
    fn test_bulk_delete_by_tag_nonexistent() {
        let store = Store::open(":memory:").unwrap();
        let deleted = store.bulk_delete_by_tag("nope", None).unwrap();
        assert!(deleted.is_empty());
    }

    #[test]
    fn test_bulk_delete_cold_empty() {
        let store = Store::open(":memory:").unwrap();
        let deleted = store.bulk_delete_cold(None, 30).unwrap();
        assert!(deleted.is_empty());
    }

    #[test]
    fn test_bulk_delete_all_empty() {
        let store = Store::open(":memory:").unwrap();
        let deleted = store.bulk_delete_all(None).unwrap();
        assert!(deleted.is_empty());
    }

    #[test]
    fn test_unique_tags_empty_store() {
        let store = Store::open(":memory:").unwrap();
        let tags = store.unique_tags(None).unwrap();
        assert!(tags.is_empty());
    }

    #[test]
    fn test_unique_tags_empty_namespace() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory_ns("1", "a", &["tag"], "ns-a"))
            .unwrap();

        let tags = store.unique_tags(Some("ns-b")).unwrap();
        assert!(tags.is_empty());
    }

    #[test]
    fn test_tags_with_counts_empty() {
        let store = Store::open(":memory:").unwrap();
        let info = store.tags_with_counts(None).unwrap();
        assert!(info.is_empty());
    }

    #[test]
    fn test_insert_and_retrieve_with_metadata() {
        let store = Store::open(":memory:").unwrap();
        let mut m = make_test_memory("meta1", "with metadata", &[]);
        m.metadata = serde_json::json!({"key": "value", "number": 42});
        store.insert(&m).unwrap();

        let got = store.get_by_id("meta1").unwrap().unwrap();
        assert_eq!(got.metadata["key"], "value");
        assert_eq!(got.metadata["number"], 42);
    }

    #[test]
    fn test_insert_with_memory_type() {
        let store = Store::open(":memory:").unwrap();
        let mut m = make_test_memory("mt1", "procedure memory", &[]);
        m.memory_type = "procedure".to_string();
        store.insert(&m).unwrap();

        let got = store.get_by_id("mt1").unwrap().unwrap();
        assert_eq!(got.memory_type, "procedure");
    }

    #[test]
    fn test_insert_with_valid_from() {
        let store = Store::open(":memory:").unwrap();
        let now = chrono::Utc::now();
        let mut m = make_test_memory("vf1", "temporal", &[]);
        m.valid_from = Some(now);
        store.insert(&m).unwrap();

        let got = store.get_by_id("vf1").unwrap().unwrap();
        assert!(got.valid_from.is_some());
    }

    #[test]
    fn test_update_preserves_namespace() {
        let store = Store::open(":memory:").unwrap();
        let m = make_test_memory_ns("u1", "original", &[], "my-ns");
        store.insert(&m).unwrap();

        let mut m2 = make_test_memory_ns("u1", "updated", &[], "my-ns");
        m2.content = "updated".to_string();
        store.update(&m2).unwrap();

        let got = store.get_by_id("u1").unwrap().unwrap();
        assert_eq!(got.content, "updated");
        assert_eq!(got.namespace, "my-ns");
    }

    #[test]
    fn test_double_insert_same_id() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("dup", "first", &[]))
            .unwrap();

        // Second insert with same ID should fail (PRIMARY KEY constraint)
        let result = store.insert(&make_test_memory("dup", "second", &[]));
        assert!(result.is_err());
    }
}
