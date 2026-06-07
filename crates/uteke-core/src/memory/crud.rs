//! Core CRUD operations — insert, get, delete, update, list, search, count.

use crate::memory::types::{Memory, DEFAULT_NAMESPACE};
use crate::Error;
use rusqlite::{params, OptionalExtension};

use super::store::{row_to_memory, serialize_embedding};

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
            .map_err(|e| Error::db("Failed to prepare statement for insert", e))?;

        Ok(())
    }

    /// Get a memory by its ID.
    pub fn get_by_id(&self, id: &str) -> Result<Option<Memory>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories WHERE id = ?1")
            .map_err(|e| Error::db("Failed to prepare statement for get_by_id", e))?;

        let result = stmt
            .query_row(params![id], row_to_memory)
            .optional()
            .map_err(|e| Error::db("Failed to get memory by ID", e))?;

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
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2) ORDER BY created_at DESC LIMIT ?3 OFFSET ?4",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories WHERE namespace = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
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
                "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type
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
            Some(_) => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories WHERE namespace = ?1 ORDER BY created_at",
            None => "SELECT id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type FROM memories ORDER BY created_at",
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

    /// Get the underlying database path, if file-based.
    pub fn path(&self) -> Option<std::path::PathBuf> {
        self.conn.path().map(std::path::PathBuf::from)
    }
}
