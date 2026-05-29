//! SQLite-backed persistence for memories.

use crate::memory::types::{Memory, TagInfo};
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
    last_accessed TEXT
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
                "INSERT INTO memories (id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                ],
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        Ok(())
    }

    /// Get a memory by its ID.
    pub fn get_by_id(&self, id: &str) -> Result<Option<Memory>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed FROM memories WHERE id = ?1")
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
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed FROM memories WHERE namespace = ?1 AND tags LIKE ?2 ORDER BY created_at DESC LIMIT ?3 OFFSET ?4",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed FROM memories WHERE namespace = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        };

        let mut memories = Vec::new();
        match tag {
            Some(t) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::Database(e.to_string()))?;
                let rows = stmt
                    .query_map(
                        params![ns, format!("%\"{t}\"%"), limit, offset],
                        row_to_memory,
                    )
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
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace
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
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed FROM memories WHERE namespace = ?1 ORDER BY created_at",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed FROM memories ORDER BY created_at",
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
    pub fn unique_tags(&self, namespace: Option<&str>) -> Result<Vec<String>, Error> {
        let sql = match namespace {
            Some(_) => {
                "SELECT DISTINCT tags FROM memories WHERE tags IS NOT NULL AND namespace = ?1"
            }
            None => "SELECT DISTINCT tags FROM memories WHERE tags IS NOT NULL",
        };

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows: Vec<Result<String, rusqlite::Error>> = match namespace {
            Some(ns) => stmt
                .query_map(params![ns], |row: &rusqlite::Row| row.get(0))
                .map_err(|e| Error::Database(e.to_string()))?
                .collect(),
            None => stmt
                .query_map([], |row: &rusqlite::Row| row.get(0))
                .map_err(|e| Error::Database(e.to_string()))?
                .collect(),
        };

        let mut all_tags = std::collections::HashSet::new();
        for row in rows {
            let tags_str = row.map_err(|e| Error::Database(e.to_string()))?;
            if let Ok(tags) = serde_json::from_str::<Vec<String>>(&tags_str) {
                for tag in tags {
                    all_tags.insert(tag);
                }
            }
        }
        Ok(all_tags.into_iter().collect())
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

    /// Get all tags with their usage counts, optionally filtered by namespace.
    pub fn tags_with_counts(&self, namespace: Option<&str>) -> Result<Vec<TagInfo>, Error> {
        let tags = self.unique_tags(namespace)?;
        let mut result = Vec::new();
        for tag in &tags {
            let count = self.count_tag(tag, namespace)?;
            result.push(TagInfo {
                name: tag.clone(),
                count,
            });
        }
        Ok(result)
    }

    /// Count how many memories use a specific tag.
    fn count_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let pattern = format!("%\"{tag}\"%");
        let count: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND tags LIKE ?2",
                rusqlite::params![ns, pattern],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(count)
    }

    /// Rename a tag across all memories (optionally filtered by namespace).
    /// Returns the number of memories updated.
    pub fn rename_tag(
        &self,
        old: &str,
        new: &str,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let pattern = format!("%\"{old}\"%");
        let sql = "SELECT id, tags FROM memories WHERE namespace = ?1 AND tags LIKE ?2";
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![ns, pattern], |row| {
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

    /// Delete a tag from all memories (optionally filtered by namespace).
    /// Returns the number of memories updated.
    pub fn delete_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let pattern = format!("%\"{tag}\"%");
        let sql = "SELECT id, tags FROM memories WHERE namespace = ?1 AND tags LIKE ?2";
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![ns, pattern], |row| {
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

    /// Count memories by tier (hot/warm/cold) for a namespace.
    pub fn tier_counts(&self, namespace: Option<&str>) -> Result<(usize, usize, usize), Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let now = chrono::Utc::now();
        let hot_cutoff = (now - chrono::Duration::days(7)).to_rfc3339();
        let warm_cutoff = (now - chrono::Duration::days(30)).to_rfc3339();

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
}

/// Serialize an embedding vector to a byte blob.
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

    #[test]
    fn test_tags_with_counts() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("1", "a", &["rust", "ai"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rust", "web"]))
            .unwrap();
        store.insert(&make_test_memory("3", "c", &["ai"])).unwrap();

        let mut tags = store.tags_with_counts(None).unwrap();
        tags.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].name, "ai");
        assert_eq!(tags[0].count, 2);
        assert_eq!(tags[1].name, "rust");
        assert_eq!(tags[1].count, 2);
        assert_eq!(tags[2].name, "web");
        assert_eq!(tags[2].count, 1);
    }

    #[test]
    fn test_rename_tag() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("1", "a", &["rust", "ai"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rust"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["python"]))
            .unwrap();

        let count = store.rename_tag("rust", "systems", None).unwrap();
        assert_eq!(count, 2);

        let m1 = store.get_by_id("1").unwrap().unwrap();
        assert_eq!(m1.tags, vec!["systems", "ai"]);

        let m2 = store.get_by_id("2").unwrap().unwrap();
        assert_eq!(m2.tags, vec!["systems"]);

        let m3 = store.get_by_id("3").unwrap().unwrap();
        assert_eq!(m3.tags, vec!["python"]);
    }

    #[test]
    fn test_delete_tag() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("1", "a", &["rust", "ai"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rust"]))
            .unwrap();

        let count = store.delete_tag("rust", None).unwrap();
        assert_eq!(count, 2);

        let m1 = store.get_by_id("1").unwrap().unwrap();
        assert_eq!(m1.tags, vec!["ai"]);

        let m2 = store.get_by_id("2").unwrap().unwrap();
        assert!(m2.tags.is_empty());
    }
}
