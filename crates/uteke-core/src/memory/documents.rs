//! Document engine — wiki/knowledge base support (#406).
//!
//! Full markdown content lives in the `documents` table. Content is chunked
//! via the markdown chunker (#405) and each chunk gets its own embedding for
//! semantic search at the section level.

use crate::Error;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// A document in the knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Unique UUID.
    pub id: String,
    /// URL-friendly identifier (unique per namespace).
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Full markdown content.
    pub content: String,
    /// Namespace for isolation.
    pub namespace: String,
    /// JSON array of tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// JSON metadata object.
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Version number (incremented on each edit).
    pub version: i64,
    /// Content type: "markdown" or "text".
    pub content_type: String,
    /// Creation timestamp (RFC3339).
    pub created_at: String,
    /// Last update timestamp (RFC3339).
    pub updated_at: String,
}

/// A chunk of a document — used for semantic search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentChunk {
    /// Unique UUID.
    pub id: String,
    /// Parent document ID.
    pub document_id: String,
    /// Index within the document (0-based).
    pub chunk_index: i64,
    /// Section heading (empty if no heading).
    pub heading: String,
    /// Chunk text content.
    pub content: String,
    /// Character offset from start of document content.
    pub char_start: i64,
    /// Character offset end (exclusive).
    pub char_end: i64,
    /// Tags inherited from parent document.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl super::Store {
    /// Create or replace a document (#406).
    ///
    /// If a document with the same slug+namespace exists, it is updated
    /// (version incremented, content replaced, chunks rebuilt).
    /// Returns the document ID.
    pub fn upsert_document(&self, doc: &Document) -> Result<String, Error> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin document upsert transaction", e))?;

        // Check if document exists (by slug + namespace).
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM documents WHERE namespace = ?1 AND slug = ?2",
                params![doc.namespace, doc.slug],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::db("query existing document", e))?;

        let doc_id = if let Some(id) = existing {
            // Update existing document.
            let version = doc.version + 1;
            tx.execute(
                "UPDATE documents SET title = ?1, content = ?2, tags = ?3, metadata = ?4, \
                 version = ?5, updated_at = ?6 WHERE id = ?7",
                params![
                    doc.title,
                    doc.content,
                    serde_json::to_string(&doc.tags).unwrap_or_else(|_| "[]".into()),
                    doc.metadata.to_string(),
                    version,
                    doc.updated_at,
                    id
                ],
            )
            .map_err(|e| Error::db("update document", e))?;
            // Delete old chunks.
            tx.execute(
                "DELETE FROM document_chunks WHERE document_id = ?1",
                params![id],
            )
            .map_err(|e| Error::db("delete old document chunks", e))?;
            id
        } else {
            // Insert new document.
            tx.execute(
                "INSERT INTO documents (id, slug, title, content, namespace, tags, metadata, \
                 version, content_type, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    doc.id,
                    doc.slug,
                    doc.title,
                    doc.content,
                    doc.namespace,
                    serde_json::to_string(&doc.tags).unwrap_or_else(|_| "[]".into()),
                    doc.metadata.to_string(),
                    doc.version,
                    doc.content_type,
                    doc.created_at,
                    doc.updated_at,
                ],
            )
            .map_err(|e| Error::db("insert document", e))?;
            doc.id.clone()
        };

        tx.commit()
            .map_err(|e| Error::db("commit document upsert", e))?;

        Ok(doc_id)
    }

    /// Insert a document chunk with embedding (#406).
    pub fn insert_document_chunk(
        &self,
        chunk: &DocumentChunk,
        embedding: &[f32],
    ) -> Result<(), Error> {
        let embedding_blob = super::store::serialize_embedding(embedding);
        self.conn
            .execute(
                "INSERT INTO document_chunks (id, document_id, chunk_index, heading, content, \
                 embedding, char_start, char_end, tags, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    chunk.id,
                    chunk.document_id,
                    chunk.chunk_index,
                    chunk.heading,
                    chunk.content,
                    embedding_blob,
                    chunk.char_start,
                    chunk.char_end,
                    serde_json::to_string(&chunk.tags).unwrap_or_else(|_| "[]".into()),
                    chrono::Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| Error::db("insert document chunk", e))?;
        Ok(())
    }

    /// Get a document by ID.
    pub fn get_document(&self, id: &str) -> Result<Option<Document>, Error> {
        let doc = self
            .conn
            .query_row(
                "SELECT id, slug, title, content, namespace, tags, metadata, version, \
                 content_type, created_at, updated_at FROM documents WHERE id = ?1",
                params![id],
                |row| {
                    let tags_str: String = row.get(5)?;
                    let meta_str: String = row.get(6)?;
                    Ok(Document {
                        id: row.get(0)?,
                        slug: row.get(1)?,
                        title: row.get(2)?,
                        content: row.get(3)?,
                        namespace: row.get(4)?,
                        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                        metadata: serde_json::from_str(&meta_str)
                            .unwrap_or(serde_json::Value::Null),
                        version: row.get(7)?,
                        content_type: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|e| Error::db("get document", e))?;
        Ok(doc)
    }

    /// Get a document by slug + namespace.
    pub fn get_document_by_slug(
        &self,
        slug: &str,
        namespace: &str,
    ) -> Result<Option<Document>, Error> {
        let doc = self
            .conn
            .query_row(
                "SELECT id, slug, title, content, namespace, tags, metadata, version, \
                 content_type, created_at, updated_at FROM documents \
                 WHERE slug = ?1 AND namespace = ?2",
                params![slug, namespace],
                |row| {
                    let tags_str: String = row.get(5)?;
                    let meta_str: String = row.get(6)?;
                    Ok(Document {
                        id: row.get(0)?,
                        slug: row.get(1)?,
                        title: row.get(2)?,
                        content: row.get(3)?,
                        namespace: row.get(4)?,
                        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                        metadata: serde_json::from_str(&meta_str)
                            .unwrap_or(serde_json::Value::Null),
                        version: row.get(7)?,
                        content_type: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|e| Error::db("get document by slug", e))?;
        Ok(doc)
    }

    /// List documents in a namespace.
    pub fn list_documents(
        &self,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DocumentSummary>, Error> {
        let limit = limit.min(1000) as i64;

        let mut stmt = if namespace.is_some() {
            self.conn.prepare(
                "SELECT id, slug, title, namespace, version, updated_at FROM documents WHERE namespace = ?1 ORDER BY updated_at DESC LIMIT ?2"
            ).map_err(|e| Error::db("prepare list documents", e))?
        } else {
            self.conn.prepare(
                "SELECT id, slug, title, namespace, version, updated_at FROM documents ORDER BY updated_at DESC LIMIT ?1"
            ).map_err(|e| Error::db("prepare list documents", e))?
        };

        let rows = match namespace {
            Some(ns) => stmt
                .query_map(params![ns, limit], row_to_summary)
                .map_err(|e| Error::db("list documents query", e))?,
            None => stmt
                .query_map(params![limit], row_to_summary)
                .map_err(|e| Error::db("list documents query", e))?,
        };

        let docs: Vec<DocumentSummary> = rows.filter_map(|r| r.ok()).collect();
        Ok(docs)
    }

    /// Delete a document by ID (cascades to chunks).
    pub fn delete_document(&self, id: &str) -> Result<bool, Error> {
        let n = self
            .conn
            .execute("DELETE FROM documents WHERE id = ?1", params![id])
            .map_err(|e| Error::db("delete document", e))?;
        Ok(n > 0)
    }

    /// Count documents in a namespace.
    pub fn count_documents(&self, namespace: Option<&str>) -> Result<usize, Error> {
        let count: i64 = match namespace {
            Some(ns) => self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM documents WHERE namespace = ?1",
                    params![ns],
                    |row| row.get(0),
                )
                .map_err(|e| Error::db("count documents", e))?,
            None => self
                .conn
                .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
                .map_err(|e| Error::db("count documents", e))?,
        };
        Ok(count as usize)
    }
}

/// Summary of a document (for list views).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub namespace: String,
    pub version: i64,
    pub updated_at: String,
}

/// Row mapper for DocumentSummary (used by list_documents).
fn row_to_summary(row: &rusqlite::Row) -> rusqlite::Result<DocumentSummary> {
    Ok(DocumentSummary {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        namespace: row.get(3)?,
        version: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::Store;

    fn open_test_store() -> Store {
        Store::open(":memory:").unwrap()
    }

    #[test]
    fn test_document_crud() {
        let store = open_test_store();
        let now = chrono::Utc::now().to_rfc3339();
        let doc = Document {
            id: "doc-1".to_string(),
            slug: "getting-started".to_string(),
            title: "Getting Started".to_string(),
            content: "# Hello\n\nWorld".to_string(),
            namespace: "default".to_string(),
            tags: vec!["guide".to_string()],
            metadata: serde_json::Value::Null,
            version: 1,
            content_type: "markdown".to_string(),
            created_at: now.clone(),
            updated_at: now,
        };

        // Create.
        let id = store.upsert_document(&doc).unwrap();
        assert_eq!(id, "doc-1");

        // Get by ID.
        let got = store.get_document("doc-1").unwrap().unwrap();
        assert_eq!(got.title, "Getting Started");
        assert_eq!(got.content, "# Hello\n\nWorld");

        // Get by slug.
        let got2 = store
            .get_document_by_slug("getting-started", "default")
            .unwrap()
            .unwrap();
        assert_eq!(got2.id, "doc-1");

        // List.
        let list = store.list_documents(Some("default"), 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].slug, "getting-started");

        // Delete.
        assert!(store.delete_document("doc-1").unwrap());
        assert!(store.get_document("doc-1").unwrap().is_none());
    }

    #[test]
    fn test_document_upsert_increments_version() {
        let store = open_test_store();
        let now = chrono::Utc::now().to_rfc3339();
        let doc = Document {
            id: "doc-1".to_string(),
            slug: "test".to_string(),
            title: "V1".to_string(),
            content: "Content 1".to_string(),
            namespace: "default".to_string(),
            tags: vec![],
            metadata: serde_json::Value::Null,
            version: 1,
            content_type: "markdown".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        store.upsert_document(&doc).unwrap();

        // Upsert with same slug but new title.
        let doc2 = Document {
            title: "V2".to_string(),
            content: "Content 2".to_string(),
            updated_at: now.clone(),
            ..doc
        };
        store.upsert_document(&doc2).unwrap();

        let got = store
            .get_document_by_slug("test", "default")
            .unwrap()
            .unwrap();
        assert_eq!(got.title, "V2");
        assert_eq!(got.version, 2); // Incremented
    }

    #[test]
    fn test_document_namespace_isolation() {
        let store = open_test_store();
        let now = chrono::Utc::now().to_rfc3339();

        let doc1 = Document {
            id: "d1".to_string(),
            slug: "guide".to_string(),
            title: "NS1 Guide".to_string(),
            content: "ns1".to_string(),
            namespace: "ns1".to_string(),
            tags: vec![],
            metadata: serde_json::Value::Null,
            version: 1,
            content_type: "markdown".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let doc2 = Document {
            id: "d2".to_string(),
            namespace: "ns2".to_string(),
            title: "NS2 Guide".to_string(),
            content: "ns2".to_string(),
            ..doc1.clone()
        };

        store.upsert_document(&doc1).unwrap();
        store.upsert_document(&doc2).unwrap();

        // Same slug, different namespace → both exist.
        assert!(store
            .get_document_by_slug("guide", "ns1")
            .unwrap()
            .is_some());
        assert!(store
            .get_document_by_slug("guide", "ns2")
            .unwrap()
            .is_some());
    }
}
