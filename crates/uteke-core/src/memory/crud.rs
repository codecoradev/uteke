//! Core CRUD operations — insert, get, delete, update, list, search, count.

use crate::memory::types::{Memory, DEFAULT_NAMESPACE};
use crate::Error;
use rusqlite::{params, OptionalExtension};

use super::store::{row_to_memory, serialize_embedding};

/// Detect whether content is valid JSON (object or array).
/// Returns "json" if it parses as JSON and starts with `{` or `[`, otherwise "text".
pub fn detect_content_type(content: &str) -> &'static str {
    let trimmed = content.trim();
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(content).is_ok()
    {
        "json"
    } else {
        "text"
    }
}

/// Flatten JSON content to a text representation for embedding.
/// Example: `{"name": "Alice", "role": "CTO"}` → "name: Alice, role: CTO"
pub(crate) fn flatten_json_for_embedding(content: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(obj) = v.as_object() {
            let pairs: Vec<String> = obj
                .iter()
                .map(|(k, v)| format!("{}: {}", k, flatten_value(v)))
                .collect();
            return pairs.join(", ");
        }
        // For arrays, flatten each element
        if let Some(arr) = v.as_array() {
            let items: Vec<String> = arr.iter().map(flatten_value).collect();
            return items.join(", ");
        }
    }
    content.to_string()
}

fn flatten_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Object(map) => {
            let pairs: Vec<String> = map
                .iter()
                .map(|(k, val)| format!("{}: {}", k, flatten_value(val)))
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(flatten_value).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

impl super::Store {
    /// Insert a new memory. Returns the inserted memory's ID.
    pub fn insert(&self, memory: &Memory) -> Result<(), Error> {
        let embedding_blob = serialize_embedding(&memory.embedding);
        let tags_json =
            serde_json::to_string(&memory.tags).map_err(|e| Error::db("database operation", e))?;
        let metadata_json = serde_json::to_string(&memory.metadata)
            .map_err(|e| Error::db("database operation", e))?;

        self.conn
            .execute(
                "INSERT INTO memories (id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug, source, source_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
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
                    memory.importance,
                    memory.pinned as i32,
                    memory.content_type,
                    memory.slug,
                    memory.source,
                    memory.source_type,
                ],
            )
            .map_err(|e| Error::db("Failed to insert memory", e))?;

        // Dual-write: insert tags into junction table
        for tag in &memory.tags {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO memory_tags (memory_id, tag) VALUES (?1, ?2)",
                    params![memory.id, tag],
                )
                .map_err(|e| Error::db("Failed to insert tag", e))?;
        }

        Ok(())
    }

    /// Get a memory by its ID.
    pub fn get_by_id(&self, id: &str) -> Result<Option<Memory>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug FROM memories WHERE id = ?1")
            .map_err(|e| Error::db("Failed to prepare statement for get_by_id", e))?;

        let result = stmt
            .query_row(params![id], row_to_memory)
            .optional()
            .map_err(|e| Error::db("Failed to get memory by ID", e))?;

        Ok(result)
    }

    /// Get a memory by its ID, only if it belongs to `namespace`.
    ///
    /// Used by edge auto-wiring (#346) to enforce namespace isolation on
    /// `^<uuid>` / `><uuid>` / `rel:*:<id>` references. Returns None when the
    /// memory does not exist OR exists in a different namespace.
    pub fn get_by_id_in_namespace(
        &self,
        id: &str,
        namespace: Option<&str>,
    ) -> Result<Option<Memory>, Error> {
        let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare("SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug FROM memories WHERE id = ?1 AND namespace = ?2")
            .map_err(|e| Error::db("Failed to prepare statement for get_by_id_in_namespace", e))?;

        let result = stmt
            .query_row(params![id, ns], row_to_memory)
            .optional()
            .map_err(|e| Error::db("Failed to get memory by ID in namespace", e))?;

        Ok(result)
    }

    /// Delete a memory by ID. Returns true if a row was deleted.
    pub fn delete(&self, id: &str) -> Result<bool, Error> {
        let deleted = self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|e| Error::db("Failed to delete memory", e))?;
        Ok(deleted > 0)
    }

    /// Update an existing memory.
    ///
    /// Note: Only updates content, embedding, tags, metadata, updated_at, and namespace.
    /// Fields NOT updated: access_count, last_accessed, deprecated, valid_from,
    /// valid_until, memory_type. Use dedicated methods for those.
    #[allow(dead_code)]
    pub fn update(&self, memory: &Memory) -> Result<(), Error> {
        let embedding_blob = serialize_embedding(&memory.embedding);
        let tags_json =
            serde_json::to_string(&memory.tags).map_err(|e| Error::db("database operation", e))?;
        let metadata_json = serde_json::to_string(&memory.metadata)
            .map_err(|e| Error::db("database operation", e))?;

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
            .map_err(|e| Error::db("database operation", e))?;

        // Dual-write: sync junction table tags
        self.conn
            .execute(
                "DELETE FROM memory_tags WHERE memory_id = ?1",
                params![memory.id],
            )
            .map_err(|e| Error::db("delete old tags", e))?;
        for tag in &memory.tags {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO memory_tags (memory_id, tag) VALUES (?1, ?2)",
                    params![memory.id, tag],
                )
                .map_err(|e| Error::db("insert tag", e))?;
        }

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
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);

        let sql = match tag {
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM memory_tags WHERE memory_id = memories.id AND tag = ?2) ORDER BY created_at DESC LIMIT ?3 OFFSET ?4",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug FROM memories WHERE namespace = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        };

        let mut memories = Vec::new();
        match tag {
            Some(t) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("database operation", e))?;
                let rows = stmt
                    .query_map(params![ns, t, limit, offset], row_to_memory)
                    .map_err(|e| Error::db("database operation", e))?;
                for row in rows {
                    let m = row.map_err(|e| Error::db("database operation", e))?;
                    memories.push(m);
                }
            }
            None => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("database operation", e))?;
                let rows = stmt
                    .query_map(params![ns, limit, offset], row_to_memory)
                    .map_err(|e| Error::db("database operation", e))?;
                for row in rows {
                    let m = row.map_err(|e| Error::db("database operation", e))?;
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
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        // Escape SQL LIKE wildcards so user input is treated as literal text
        // Using '!' as escape character — unambiguous on all platforms
        let escaped = query
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_");
        let pattern = format!("%{escaped}%");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug
                 FROM memories WHERE namespace = ?1 AND content LIKE ?2 ESCAPE '!'
                 ORDER BY created_at DESC LIMIT ?3",
            )
            .map_err(|e| Error::db("database operation", e))?;

        let rows = stmt
            .query_map(params![ns, pattern, limit], row_to_memory)
            .map_err(|e| Error::db("database operation", e))?;

        let mut memories = Vec::new();
        for row in rows {
            let m = row.map_err(|e| Error::db("database operation", e))?;
            memories.push(m);
        }
        Ok(memories)
    }

    /// Load all memories for index rebuilding, optionally filtered by namespace.
    pub fn load_all(&self, namespace: Option<&str>) -> Result<Vec<Memory>, Error> {
        let sql = match namespace {
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug FROM memories WHERE namespace = ?1 ORDER BY created_at",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug FROM memories ORDER BY created_at",
        };

        let mut memories = Vec::new();
        match namespace {
            Some(ns) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("database operation", e))?;
                let rows = stmt
                    .query_map(params![ns], row_to_memory)
                    .map_err(|e| Error::db("database operation", e))?;
                for row in rows {
                    let m = row.map_err(|e| Error::db("database operation", e))?;
                    memories.push(m);
                }
            }
            None => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("database operation", e))?;
                let rows = stmt
                    .query_map([], row_to_memory)
                    .map_err(|e| Error::db("database operation", e))?;
                for row in rows {
                    let m = row.map_err(|e| Error::db("database operation", e))?;
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
                .map_err(|e| Error::db("database operation", e))?,
            None => self
                .conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
                .map_err(|e| Error::db("database operation", e))?,
        };
        Ok(count)
    }

    /// Get all memory IDs in a namespace (#401).
    /// Used for namespace-scoped cosine auto-linking.
    pub fn memories_in_namespace(&self, namespace: &str) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1")
            .map_err(|e| Error::db("prepare memories_in_namespace", e))?;
        let ids: Vec<String> = stmt
            .query_map(params![namespace], |row| row.get(0))
            .map_err(|e| Error::db("query memories_in_namespace", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// List memories that existed at a specific point in time.
    ///
    /// A memory existed at `point_in_time` if:
    /// - `created_at <= point_in_time`
    /// - `valid_from IS NULL OR valid_from <= point_in_time`
    /// - `valid_until IS NULL OR valid_until > point_in_time`
    /// - `deprecated = 0`
    pub fn list_at_time(
        &self,
        tag: Option<&str>,
        namespace: Option<&str>,
        limit: usize,
        offset: usize,
        point_in_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Memory>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let pit = point_in_time.to_rfc3339();

        let sql = match tag {
            Some(_) => {
                "SELECT m.id, m.content, m.embedding, m.tags, m.metadata, \
                 m.created_at, m.updated_at, m.namespace, m.access_count, \
                 m.last_accessed, m.deprecated, m.valid_from, m.valid_until, \
                 m.memory_type, m.importance, m.pinned \
                 FROM memories m \
                 INNER JOIN memory_tags mt ON mt.memory_id = m.id \
                 WHERE m.namespace = ?1 \
                   AND m.created_at <= ?2 \
                   AND (m.valid_from IS NULL OR m.valid_from <= ?2) \
                   AND (m.valid_until IS NULL OR m.valid_until > ?2) \
                   AND m.deprecated = 0 \
                   AND mt.tag = ?3 \
                 ORDER BY m.created_at DESC LIMIT ?4 OFFSET ?5"
            }
            None => {
                "SELECT id, content, embedding, tags, metadata, \
                 created_at, updated_at, namespace, access_count, \
                 last_accessed, deprecated, valid_from, valid_until, \
                 memory_type, importance, pinned \
                 FROM memories \
                 WHERE namespace = ?1 \
                   AND created_at <= ?2 \
                   AND (valid_from IS NULL OR valid_from <= ?2) \
                   AND (valid_until IS NULL OR valid_until > ?2) \
                   AND deprecated = 0 \
                 ORDER BY created_at DESC LIMIT ?3 OFFSET ?4"
            }
        };

        let mut memories = Vec::new();
        match tag {
            Some(t) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("database operation", e))?;
                let rows = stmt
                    .query_map(params![ns, pit, t, limit, offset], row_to_memory)
                    .map_err(|e| Error::db("database operation", e))?;
                for row in rows {
                    let m = row.map_err(|e| Error::db("database operation", e))?;
                    memories.push(m);
                }
            }
            None => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("database operation", e))?;
                let rows = stmt
                    .query_map(params![ns, pit, limit, offset], row_to_memory)
                    .map_err(|e| Error::db("database operation", e))?;
                for row in rows {
                    let m = row.map_err(|e| Error::db("database operation", e))?;
                    memories.push(m);
                }
            }
        }
        Ok(memories)
    }

    /// Get the underlying database path, if file-based.
    pub fn path(&self) -> Option<std::path::PathBuf> {
        self.conn.path().map(std::path::PathBuf::from)
    }
}

#[cfg(test)]
mod content_type_tests {
    use super::*;

    #[test]
    fn test_detect_json_object() {
        assert_eq!(
            detect_content_type(r#"{"name": "Alice", "role": "CTO"}"#),
            "json"
        );
    }

    #[test]
    fn test_detect_json_array() {
        assert_eq!(detect_content_type(r#"[1, 2, 3]"#), "json");
    }

    #[test]
    fn test_detect_text_not_json() {
        assert_eq!(detect_content_type("hello world"), "text");
    }

    #[test]
    fn test_detect_text_looks_like_json_but_invalid() {
        assert_eq!(detect_content_type("{not valid json}"), "text");
    }

    #[test]
    fn test_flatten_simple_object() {
        let result = flatten_json_for_embedding(r#"{"name": "Alice", "role": "CTO"}"#);
        assert!(result.contains("name: Alice"));
        assert!(result.contains("role: CTO"));
    }

    #[test]
    fn test_flatten_with_numbers() {
        let result = flatten_json_for_embedding(r#"{"count": 42, "active": true}"#);
        assert!(result.contains("count: 42"));
        assert!(result.contains("active: true"));
    }

    #[test]
    fn test_flatten_array() {
        let result = flatten_json_for_embedding(r#"["rust", "python", "go"]"#);
        assert_eq!(result, "rust, python, go");
    }

    #[test]
    fn test_flatten_nested_object() {
        let result =
            flatten_json_for_embedding(r#"{"name": "Alice", "addr": {"city": "Jakarta"}}"#);
        assert!(result.contains("name: Alice"));
        assert!(result.contains("addr: {city: Jakarta}"));
    }

    #[test]
    fn test_flatten_invalid_json_returns_raw() {
        let result = flatten_json_for_embedding("not json at all");
        assert_eq!(result, "not json at all");
    }

    #[test]
    fn test_json_content_stored_with_correct_type() {
        let store = super::super::store::Store::open(":memory:").unwrap();
        let json_content = r#"{"name": "Alice", "role": "CTO", "timezone": "UTC+7"}"#;
        let memory = crate::memory::types::Memory {
            id: "json-test-1".to_string(),
            content: json_content.to_string(),
            embedding: vec![0.1; 768],
            tags: vec!["person".to_string()],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "json".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
        };
        store.insert(&memory).unwrap();

        let retrieved = store.get_by_id("json-test-1").unwrap().unwrap();
        assert_eq!(retrieved.content_type, "json");
        assert_eq!(retrieved.content, json_content);
    }

    #[test]
    fn test_text_content_still_works() {
        let store = super::super::store::Store::open(":memory:").unwrap();
        let memory = crate::memory::types::Memory {
            id: "text-test-1".to_string(),
            content: "plain text memory".to_string(),
            embedding: vec![0.1; 768],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
        };
        store.insert(&memory).unwrap();

        let retrieved = store.get_by_id("text-test-1").unwrap().unwrap();
        assert_eq!(retrieved.content_type, "text");
        assert_eq!(retrieved.content, "plain text memory");
    }

    #[test]
    fn test_schema_migration_v5_to_v6_adds_content_type() {
        // Fresh DB should have content_type column
        let store = super::super::store::Store::open(":memory:").unwrap();
        assert!(store.column_exists("content_type"));
        let version = store.schema_version().unwrap();
        assert_eq!(version, 12); // v12 = hierarchical docs (#438); v11 = document engine (#406); v10 = source columns (#348); v9 = timeline (#347); v8 = edges + slug; v7 = graph
    }
}
