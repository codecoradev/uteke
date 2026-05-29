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
    updated_at TEXT NOT NULL
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
        Ok(())
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
                "INSERT INTO memories (id, content, embedding, tags, metadata, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    memory.id,
                    memory.content,
                    embedding_blob,
                    tags_json,
                    metadata_json,
                    memory.created_at.to_rfc3339(),
                    memory.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        Ok(())
    }

    /// Get a memory by its ID.
    pub fn get_by_id(&self, id: &str) -> Result<Option<Memory>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, content, embedding, tags, metadata, created_at, updated_at FROM memories WHERE id = ?1")
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
                "UPDATE memories SET content = ?2, embedding = ?3, tags = ?4, metadata = ?5, updated_at = ?6
                 WHERE id = ?1",
                params![
                    memory.id,
                    memory.content,
                    embedding_blob,
                    tags_json,
                    metadata_json,
                    memory.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// List memories with optional tag filter and pagination.
    pub fn list(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Memory>, Error> {
        let sql = match tag {
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at FROM memories WHERE tags LIKE ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at FROM memories ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
        };

        let mut memories = Vec::new();
        match tag {
            Some(t) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::Database(e.to_string()))?;
                let rows = stmt
                    .query_map(params![format!("%\"{t}\"%"), limit, offset], row_to_memory)
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
                    .query_map(params![limit, offset], row_to_memory)
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
    pub fn search_content(&self, query: &str, limit: usize) -> Result<Vec<Memory>, Error> {
        let pattern = format!("%{query}%");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at
                 FROM memories WHERE content LIKE ?1
                 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![pattern, limit], row_to_memory)
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut memories = Vec::new();
        for row in rows {
            let m = row.map_err(|e| Error::Database(e.to_string()))?;
            memories.push(m);
        }
        Ok(memories)
    }

    /// Load all memories (for index rebuilding).
    pub fn load_all(&self) -> Result<Vec<Memory>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at FROM memories ORDER BY created_at",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], row_to_memory)
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut memories = Vec::new();
        for row in rows {
            let m = row.map_err(|e| Error::Database(e.to_string()))?;
            memories.push(m);
        }
        Ok(memories)
    }

    /// Count total memories.
    pub fn count(&self) -> Result<usize, Error> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(count)
    }

    /// Get all unique tags across all memories.
    pub fn unique_tags(&self) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT tags FROM memories WHERE tags IS NOT NULL")
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row: &rusqlite::Row| -> rusqlite::Result<String> {
                row.get(0)
            })
            .map_err(|e| Error::Database(e.to_string()))?;

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

    /// Get the underlying database path, if file-based.
    pub fn path(&self) -> Option<std::path::PathBuf> {
        self.conn.path().map(std::path::PathBuf::from)
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

    Ok(Memory {
        id,
        content,
        embedding,
        tags,
        metadata,
        created_at,
        updated_at,
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

        let rust_memories = store.list(Some("rust"), 10, 0).unwrap();
        assert_eq!(rust_memories.len(), 2);

        let all = store.list(None, 10, 0).unwrap();
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

        let results = store.search_content("rust", 10).unwrap();
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

        let tags = store.unique_tags().unwrap();
        assert_eq!(tags.len(), 3);
    }
}
