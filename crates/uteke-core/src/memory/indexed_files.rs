//! Indexed-file tracking for the code indexer (DB-per-repo).
//!
//! The `indexed_files` table records the content hash and mtime of each
//! source file that `uteke index` has processed, keyed by `(namespace, path)`.
//! This lets re-index runs:
//!   - skip files whose content hash is unchanged (no re-embed), and
//!   - prune memories for files that no longer exist on disk.

use crate::Error;
use rusqlite::{params, OptionalExtension};

/// A tracked source file record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFile {
    /// Repo-relative path (forward-slash normalized).
    pub path: String,
    /// Content hash (hex) at the time of indexing.
    pub content_hash: String,
    /// File mtime (unix seconds) at the time of indexing.
    pub mtime: i64,
    /// Number of chunks/memories produced from this file.
    pub chunk_count: i64,
}

impl super::Store {
    /// Look up the recorded content hash for a file in a namespace.
    /// Returns `None` if the file has never been indexed.
    pub fn indexed_file_hash(
        &self,
        namespace: &str,
        path: &str,
    ) -> Result<Option<String>, Error> {
        self.conn
            .query_row(
                "SELECT content_hash FROM indexed_files WHERE namespace = ?1 AND path = ?2",
                params![namespace, path],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| Error::db("query indexed_file hash", e))
    }

    /// Upsert a file's index record.
    pub fn upsert_indexed_file(
        &self,
        namespace: &str,
        file: &IndexedFile,
    ) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO indexed_files (namespace, path, content_hash, mtime, chunk_count, indexed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(namespace, path) DO UPDATE SET \
                     content_hash = excluded.content_hash, \
                     mtime        = excluded.mtime, \
                     chunk_count  = excluded.chunk_count, \
                     indexed_at   = excluded.indexed_at",
                params![
                    namespace,
                    file.path,
                    file.content_hash,
                    file.mtime,
                    file.chunk_count,
                    now
                ],
            )
            .map_err(|e| Error::db("upsert indexed_file", e))?;
        Ok(())
    }

    /// Return every tracked file path for a namespace.
    pub fn list_indexed_files(&self, namespace: &str) -> Result<Vec<IndexedFile>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, content_hash, mtime, chunk_count \
                 FROM indexed_files WHERE namespace = ?1 ORDER BY path",
            )
            .map_err(|e| Error::db("prepare list_indexed_files", e))?;
        let rows = stmt
            .query_map(params![namespace], |row| {
                Ok(IndexedFile {
                    path: row.get(0)?,
                    content_hash: row.get(1)?,
                    mtime: row.get(2)?,
                    chunk_count: row.get(3)?,
                })
            })
            .map_err(|e| Error::db("query list_indexed_files", e))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| Error::db("read indexed_file row", e))?);
        }
        Ok(out)
    }

    /// Delete a file's index record (used when a file is removed from disk).
    pub fn delete_indexed_file(&self, namespace: &str, path: &str) -> Result<(), Error> {
        self.conn
            .execute(
                "DELETE FROM indexed_files WHERE namespace = ?1 AND path = ?2",
                params![namespace, path],
            )
            .map_err(|e| Error::db("delete indexed_file", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::Store;

    fn mem_store() -> Store {
        Store::open(":memory:").unwrap()
    }

    #[test]
    fn upsert_and_lookup_hash() {
        let store = mem_store();
        assert_eq!(store.indexed_file_hash("default", "src/a.rs").unwrap(), None);

        let f = IndexedFile {
            path: "src/a.rs".to_string(),
            content_hash: "abc123".to_string(),
            mtime: 42,
            chunk_count: 3,
        };
        store.upsert_indexed_file("default", &f).unwrap();
        assert_eq!(
            store.indexed_file_hash("default", "src/a.rs").unwrap(),
            Some("abc123".to_string())
        );

        // Upsert with new hash overwrites.
        let f2 = IndexedFile {
            content_hash: "def456".to_string(),
            ..f.clone()
        };
        store.upsert_indexed_file("default", &f2).unwrap();
        assert_eq!(
            store.indexed_file_hash("default", "src/a.rs").unwrap(),
            Some("def456".to_string())
        );
    }

    #[test]
    fn list_and_delete() {
        let store = mem_store();
        for p in ["src/a.rs", "src/b.rs"] {
            store
                .upsert_indexed_file(
                    "default",
                    &IndexedFile {
                        path: p.to_string(),
                        content_hash: "h".to_string(),
                        mtime: 0,
                        chunk_count: 1,
                    },
                )
                .unwrap();
        }
        let listed = store.list_indexed_files("default").unwrap();
        assert_eq!(listed.len(), 2);

        store.delete_indexed_file("default", "src/a.rs").unwrap();
        let listed = store.list_indexed_files("default").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, "src/b.rs");
    }

    #[test]
    fn namespace_isolation() {
        let store = mem_store();
        store
            .upsert_indexed_file(
                "repo-a",
                &IndexedFile {
                    path: "x.rs".to_string(),
                    content_hash: "1".to_string(),
                    mtime: 0,
                    chunk_count: 1,
                },
            )
            .unwrap();
        assert_eq!(store.indexed_file_hash("repo-b", "x.rs").unwrap(), None);
        assert_eq!(store.list_indexed_files("repo-b").unwrap().len(), 0);
        assert_eq!(store.list_indexed_files("repo-a").unwrap().len(), 1);
    }
}
