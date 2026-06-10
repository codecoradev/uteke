//! FTS5 full-text search for memories.

use crate::memory::types::Memory;
use crate::Error;
use rusqlite::params;

use super::store::row_to_memory;

impl super::Store {
    /// Check if the FTS5 virtual table exists.
    pub fn fts5_exists(&self) -> Result<bool, Error> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memories_fts'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("check FTS5 table", e))?;
        Ok(exists)
    }

    /// Create the FTS5 virtual table and sync triggers.
    /// Idempotent — safe to call on existing databases.
    pub fn init_fts5(&self) -> Result<(), Error> {
        // Create FTS5 virtual table using content=memories (shadow table)
        self.conn
            .execute_batch(
                r#"
                CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                    content,
                    tags,
                    namespace,
                    content='memories',
                    content_rowid='rowid'
                );
                "#,
            )
            .map_err(|e| Error::db("create FTS5 table", e))?;

        // Triggers to keep FTS5 in sync with the memories table.
        // Using INSERT INTO memories_fts(memories_fts, ...) for delete
        // as required by content= FTS5 tables.
        self.conn
            .execute_batch(
                r#"
                CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
                    INSERT INTO memories_fts(rowid, content, tags, namespace)
                    VALUES (new.rowid, new.content, new.tags, new.namespace);
                END;

                CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
                    INSERT INTO memories_fts(memories_fts, rowid, content, tags, namespace)
                    VALUES ('delete', old.rowid, old.content, old.tags, old.namespace);
                END;

                CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
                    INSERT INTO memories_fts(memories_fts, rowid, content, tags, namespace)
                    VALUES ('delete', old.rowid, old.content, old.tags, old.namespace);
                    INSERT INTO memories_fts(rowid, content, tags, namespace)
                    VALUES (new.rowid, new.content, new.tags, new.namespace);
                END;
                "#,
            )
            .map_err(|e| Error::db("create FTS5 triggers", e))?;

        Ok(())
    }

    /// Rebuild FTS5 index from existing memories (for migration).
    pub fn rebuild_fts5(&self) -> Result<(), Error> {
        self.conn
            .execute_batch("INSERT INTO memories_fts(memories_fts) VALUES ('rebuild');")
            .map_err(|e| Error::db("rebuild FTS5 index", e))?;
        Ok(())
    }

    /// Full-text search using FTS5.
    /// Returns memories matching the query, ranked by FTS5 rank (bm25).
    pub fn search_fts5(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(Memory, f64)>, Error> {
        // Sanitize query for FTS5 — wrap in quotes to treat as phrase
        // and escape double quotes in user input
        let sanitized = query.replace('"', "\"\"");
        let fts_query = format!("\"{sanitized}\"");

        let sql = match namespace {
            Some(_) => {
                r#"SELECT m.id, m.content, m.embedding, m.tags, m.metadata, m.created_at, m.updated_at, m.namespace, m.access_count, m.last_accessed, m.deprecated, m.valid_from, m.valid_until, m.memory_type, f.rank
                   FROM memories_fts f JOIN memories m ON f.rowid = m.rowid
                   WHERE memories_fts MATCH ?1 AND m.namespace = ?2 AND m.deprecated = 0
                   ORDER BY f.rank
                   LIMIT ?3"#
            }
            None => {
                r#"SELECT m.id, m.content, m.embedding, m.tags, m.metadata, m.created_at, m.updated_at, m.namespace, m.access_count, m.last_accessed, m.deprecated, m.valid_from, m.valid_until, m.memory_type, f.rank
                   FROM memories_fts f JOIN memories m ON f.rowid = m.rowid
                   WHERE memories_fts MATCH ?1 AND m.deprecated = 0
                   ORDER BY f.rank
                   LIMIT ?2"#
            }
        };

        let mut results = Vec::new();
        match namespace {
            Some(ns) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("prepare FTS5 search", e))?;
                let rows = stmt
                    .query_map(params![fts_query, ns, limit], |row| {
                        let memory = row_to_memory(row)?;
                        let rank: f64 = row.get(14)?;
                        Ok((memory, rank))
                    })
                    .map_err(|e| Error::db("execute FTS5 search", e))?;
                for row in rows {
                    let (memory, rank) = row.map_err(|e| Error::db("read FTS5 row", e))?;
                    results.push((memory, rank));
                }
            }
            None => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("prepare FTS5 search", e))?;
                let rows = stmt
                    .query_map(params![fts_query, limit], |row| {
                        let memory = row_to_memory(row)?;
                        let rank: f64 = row.get(14)?;
                        Ok((memory, rank))
                    })
                    .map_err(|e| Error::db("execute FTS5 search", e))?;
                for row in rows {
                    let (memory, rank) = row.map_err(|e| Error::db("read FTS5 row", e))?;
                    results.push((memory, rank));
                }
            }
        }

        Ok(results)
    }

    /// Token-based FTS5 search — splits query into tokens for broader matching.
    /// Use when phrase search returns no results.
    pub fn search_fts5_tokens(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(Memory, f64)>, Error> {
        // Sanitize and join tokens with OR for broader matching
        let tokens: Vec<String> = query
            .split_whitespace()
            .filter(|t| t.len() >= 2)
            .map(|t| {
                let sanitized = t.replace('"', "");
                format!("\"{sanitized}\"")
            })
            .take(10) // Limit to prevent overly complex queries
            .collect();

        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = tokens.join(" OR ");

        let sql = match namespace {
            Some(_) => {
                r#"SELECT m.id, m.content, m.embedding, m.tags, m.metadata, m.created_at, m.updated_at, m.namespace, m.access_count, m.last_accessed, m.deprecated, m.valid_from, m.valid_until, m.memory_type, f.rank
                   FROM memories_fts f JOIN memories m ON f.rowid = m.rowid
                   WHERE memories_fts MATCH ?1 AND m.namespace = ?2 AND m.deprecated = 0
                   ORDER BY f.rank
                   LIMIT ?3"#
            }
            None => {
                r#"SELECT m.id, m.content, m.embedding, m.tags, m.metadata, m.created_at, m.updated_at, m.namespace, m.access_count, m.last_accessed, m.deprecated, m.valid_from, m.valid_until, m.memory_type, f.rank
                   FROM memories_fts f JOIN memories m ON f.rowid = m.rowid
                   WHERE memories_fts MATCH ?1 AND m.deprecated = 0
                   ORDER BY f.rank
                   LIMIT ?2"#
            }
        };

        let mut results = Vec::new();
        match namespace {
            Some(ns) => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("prepare FTS5 token search", e))?;
                let rows = stmt
                    .query_map(params![fts_query, ns, limit], |row| {
                        let memory = row_to_memory(row)?;
                        let rank: f64 = row.get(14)?;
                        Ok((memory, rank))
                    })
                    .map_err(|e| Error::db("execute FTS5 token search", e))?;
                for row in rows {
                    let (memory, rank) = row.map_err(|e| Error::db("read FTS5 row", e))?;
                    results.push((memory, rank));
                }
            }
            None => {
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("prepare FTS5 token search", e))?;
                let rows = stmt
                    .query_map(params![fts_query, limit], |row| {
                        let memory = row_to_memory(row)?;
                        let rank: f64 = row.get(14)?;
                        Ok((memory, rank))
                    })
                    .map_err(|e| Error::db("execute FTS5 token search", e))?;
                for row in rows {
                    let (memory, rank) = row.map_err(|e| Error::db("read FTS5 row", e))?;
                    results.push((memory, rank));
                }
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use crate::memory::types::Memory;
    use crate::memory::Store;
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

    #[test]
    fn test_fts5_search() {
        let store = Store::open(":memory:").unwrap();
        store.init_fts5().unwrap();

        store
            .insert(&make_test_memory(
                "1",
                "PT Maju Jaya adalah perusahaan teknologi",
                &["company"],
            ))
            .unwrap();
        store
            .insert(&make_test_memory(
                "2",
                "Deploy menggunakan Docker dan Kubernetes",
                &["deploy"],
            ))
            .unwrap();
        store
            .insert(&make_test_memory(
                "3",
                "PT Maju Bersama bergerak di bidang finansial",
                &["company"],
            ))
            .unwrap();

        // Phrase search
        let results = store.search_fts5("PT Maju Jaya", None, 10).unwrap();
        assert!(
            !results.is_empty(),
            "Should find at least 1 result for 'PT Maju Jaya'"
        );
        assert_eq!(results[0].0.id, "1");

        // Broader search
        let results = store.search_fts5("PT Maju", None, 10).unwrap();
        assert!(
            results.len() >= 2,
            "Should find results for both PT Maju entries"
        );
    }

    #[test]
    fn test_fts5_token_fallback() {
        let store = Store::open(":memory:").unwrap();
        store.init_fts5().unwrap();

        store
            .insert(&make_test_memory(
                "1",
                "rust programming language",
                &[],
            ))
            .unwrap();
        store
            .insert(&make_test_memory(
                "2",
                "python machine learning",
                &[],
            ))
            .unwrap();

        // Token search with OR
        let results = store
            .search_fts5_tokens("rust python", None, 10)
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_fts5_namespace_filter() {
        let store = Store::open(":memory:").unwrap();
        store.init_fts5().unwrap();

        let mut m1 = make_test_memory("1", "shared keyword test", &[]);
        m1.namespace = "ns-alpha".to_string();
        store.insert(&m1).unwrap();

        let mut m2 = make_test_memory("2", "shared keyword test", &[]);
        m2.namespace = "ns-beta".to_string();
        store.insert(&m2).unwrap();

        let alpha = store
            .search_fts5("shared keyword", Some("ns-alpha"), 10)
            .unwrap();
        assert_eq!(alpha.len(), 1);
        assert_eq!(alpha[0].0.namespace, "ns-alpha");
    }

    #[test]
    fn test_fts5_rebuild() {
        let store = Store::open(":memory:").unwrap();
        // Insert BEFORE FTS5 init — simulates migration
        store
            .insert(&make_test_memory("1", "migration test content", &[]))
            .unwrap();

        // Init FTS5 after data exists
        store.init_fts5().unwrap();
        store.rebuild_fts5().unwrap();

        let results = store.search_fts5("migration test", None, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.content, "migration test content");
    }

    #[test]
    fn test_fts5_deprecated_excluded() {
        let store = Store::open(":memory:").unwrap();
        store.init_fts5().unwrap();

        store
            .insert(&make_test_memory("1", "active memory content", &[]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "deprecated memory content", &[]))
            .unwrap();
        store.deprecate("2").unwrap();

        let results = store
            .search_fts5("memory content", None, 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.id, "1");
    }

    #[test]
    fn test_fts5_exists() {
        let store = Store::open(":memory:").unwrap();
        assert!(!store.fts5_exists().unwrap());
        store.init_fts5().unwrap();
        assert!(store.fts5_exists().unwrap());
    }
}
