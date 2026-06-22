//! Document engine — wiki/knowledge base support (#406).
//!
//! Full markdown content lives in the `documents` table. Content is chunked
//! via the markdown chunker (#405) and each chunk gets its own embedding for
//! semantic search at the section level.
//!
//! #438: Hierarchical documents with depth-10 support.
//! Uses hybrid adjacency list + materialized path for O(1) subtree queries.

use crate::Error;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Maximum depth for document hierarchy.
pub const MAX_DEPTH: i64 = 10;

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
    /// Parent document ID (NULL = root).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Materialized path for O(1) subtree queries (e.g. "/uuid/uuid/").
    #[serde(default)]
    pub path: String,
    /// Depth in tree (0 = root).
    #[serde(default)]
    pub depth: i64,
    /// Manual ordering within siblings.
    #[serde(default)]
    pub sort_order: i64,
    /// Whether this document has children (denormalized).
    #[serde(default)]
    pub has_children: bool,
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

/// Summary of a document (for list views).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub namespace: String,
    pub version: i64,
    pub updated_at: String,
    /// Parent document ID (NULL = root).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Depth in tree (0 = root).
    #[serde(default)]
    pub depth: i64,
    /// Whether this document has children.
    #[serde(default)]
    pub has_children: bool,
    /// Manual ordering within siblings.
    #[serde(default)]
    pub sort_order: i64,
}

/// Row mapper for Document (full document queries).
fn row_to_document(row: &rusqlite::Row) -> rusqlite::Result<Document> {
    let tags_str: String = row.get(5)?;
    let meta_str: String = row.get(6)?;
    Ok(Document {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        namespace: row.get(4)?,
        tags: serde_json::from_str(&tags_str).unwrap_or_default(),
        metadata: serde_json::from_str(&meta_str).unwrap_or(serde_json::Value::Null),
        version: row.get(7)?,
        content_type: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        parent_id: row.get(11)?,
        path: row.get(12)?,
        depth: row.get(13)?,
        sort_order: row.get(14)?,
        has_children: row.get::<_, i64>(15).map(|v| v != 0).unwrap_or(false),
    })
}

/// Row mapper for DocumentSummary (list/tree queries).
fn row_to_summary(row: &rusqlite::Row) -> rusqlite::Result<DocumentSummary> {
    Ok(DocumentSummary {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        namespace: row.get(3)?,
        version: row.get(4)?,
        updated_at: row.get(5)?,
        parent_id: row.get(6)?,
        depth: row.get(7)?,
        has_children: row.get::<_, i64>(8).map(|v| v != 0).unwrap_or(false),
        sort_order: row.get(9)?,
    })
}

impl super::Store {
    /// Create or replace a document (#406, #438).
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
                 version = ?5, updated_at = ?6, parent_id = ?7, path = ?8, depth = ?9, \
                 sort_order = ?10 WHERE id = ?11",
                params![
                    doc.title,
                    doc.content,
                    serde_json::to_string(&doc.tags).unwrap_or_else(|_| "[]".into()),
                    doc.metadata.to_string(),
                    version,
                    doc.updated_at,
                    doc.parent_id,
                    doc.path,
                    doc.depth,
                    doc.sort_order,
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
                 version, content_type, created_at, updated_at, parent_id, path, depth, sort_order) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
                    doc.parent_id,
                    doc.path,
                    doc.depth,
                    doc.sort_order,
                ],
            )
            .map_err(|e| Error::db("insert document", e))?;
            // Update parent's has_children flag.
            if let Some(ref pid) = doc.parent_id {
                tx.execute(
                    "UPDATE documents SET has_children = 1 WHERE id = ?1",
                    params![pid],
                )
                .map_err(|e| Error::db("update parent has_children", e))?;
            }
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
                 content_type, created_at, updated_at, parent_id, path, depth, \
                 sort_order, has_children FROM documents WHERE id = ?1",
                params![id],
                row_to_document,
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
                 content_type, created_at, updated_at, parent_id, path, depth, \
                 sort_order, has_children FROM documents \
                 WHERE slug = ?1 AND namespace = ?2",
                params![slug, namespace],
                row_to_document,
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
                "SELECT id, slug, title, namespace, version, updated_at, \
                 parent_id, depth, has_children, sort_order \
                 FROM documents WHERE namespace = ?1 ORDER BY updated_at DESC LIMIT ?2",
            ).map_err(|e| Error::db("prepare list documents", e))?
        } else {
            self.conn.prepare(
                "SELECT id, slug, title, namespace, version, updated_at, \
                 parent_id, depth, has_children, sort_order \
                 FROM documents ORDER BY updated_at DESC LIMIT ?1",
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

    /// List root documents (parent_id IS NULL) in a namespace.
    pub fn list_root_documents(
        &self,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DocumentSummary>, Error> {
        let limit = limit.min(1000) as i64;

        let mut stmt = if namespace.is_some() {
            self.conn.prepare(
                "SELECT id, slug, title, namespace, version, updated_at, \
                 parent_id, depth, has_children, sort_order \
                 FROM documents WHERE namespace = ?1 AND parent_id IS NULL \
                 ORDER BY sort_order, updated_at DESC LIMIT ?2",
            ).map_err(|e| Error::db("prepare list root documents", e))?
        } else {
            self.conn.prepare(
                "SELECT id, slug, title, namespace, version, updated_at, \
                 parent_id, depth, has_children, sort_order \
                 FROM documents WHERE parent_id IS NULL \
                 ORDER BY sort_order, updated_at DESC LIMIT ?1",
            ).map_err(|e| Error::db("prepare list root documents", e))?
        };

        let rows = match namespace {
            Some(ns) => stmt
                .query_map(params![ns, limit], row_to_summary)
                .map_err(|e| Error::db("list root documents query", e))?,
            None => stmt
                .query_map(params![limit], row_to_summary)
                .map_err(|e| Error::db("list root documents query", e))?,
        };

        let docs: Vec<DocumentSummary> = rows.filter_map(|r| r.ok()).collect();
        Ok(docs)
    }

    /// List direct children of a document.
    pub fn list_document_children(
        &self,
        parent_id: &str,
        limit: usize,
    ) -> Result<Vec<DocumentSummary>, Error> {
        let limit = limit.min(1000) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT id, slug, title, namespace, version, updated_at, \
             parent_id, depth, has_children, sort_order \
             FROM documents WHERE parent_id = ?1 \
             ORDER BY sort_order, updated_at DESC LIMIT ?2",
        ).map_err(|e| Error::db("prepare list children", e))?;

        let rows = stmt
            .query_map(params![parent_id, limit], row_to_summary)
            .map_err(|e| Error::db("list children query", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get all descendants of a document (subtree) via path prefix scan.
    pub fn list_descendants(
        &self,
        id_or_slug: &str,
        namespace: &str,
        max_depth: Option<i64>,
        limit: usize,
    ) -> Result<Vec<DocumentSummary>, Error> {
        let limit = limit.min(1000) as i64;
        let doc = match self.get_document_by_slug(id_or_slug, namespace)? {
            Some(d) => d,
            None => self
                .get_document(id_or_slug)?
                .ok_or_else(|| Error::validation("document not found for descendants query"))?,
        };
        let path_prefix = format!("{}/", doc.path);

        let rows = if let Some(max) = max_depth {
            let mut stmt = self.conn.prepare(
                "SELECT id, slug, title, namespace, version, updated_at, \
                 parent_id, depth, has_children, sort_order \
                 FROM documents WHERE path LIKE ?1 AND depth <= ?2 \
                 ORDER BY path, sort_order LIMIT ?3",
            ).map_err(|e| Error::db("prepare list descendants", e))?;
            stmt.query_map(params![path_prefix, max, limit], row_to_summary)
                .map_err(|e| Error::db("list descendants query", e))?
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, slug, title, namespace, version, updated_at, \
                 parent_id, depth, has_children, sort_order \
                 FROM documents WHERE path LIKE ?1 \
                 ORDER BY path, sort_order LIMIT ?2",
            ).map_err(|e| Error::db("prepare list descendants", e))?;
            stmt.query_map(params![path_prefix, limit], row_to_summary)
                .map_err(|e| Error::db("list descendants query", e))?
        };

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get breadcrumbs from root to a document.
    pub fn get_breadcrumbs(
        &self,
        id_or_slug: &str,
        namespace: &str,
    ) -> Result<Vec<DocumentSummary>, Error> {
        let doc = match self.get_document_by_slug(id_or_slug, namespace)? {
            Some(d) => d,
            None => self
                .get_document(id_or_slug)?
                .ok_or_else(|| Error::validation("document not found for breadcrumbs query"))?,
        };

        // Extract UUIDs from path: "/uuid1/uuid2/uuid3/"
        let uuids: Vec<&str> = doc.path.split('/').filter(|s| !s.is_empty()).collect();

        if uuids.is_empty() {
            return Ok(vec![DocumentSummary {
                id: doc.id,
                slug: doc.slug,
                title: doc.title,
                namespace: doc.namespace,
                version: doc.version,
                updated_at: doc.updated_at,
                parent_id: None,
                depth: 0,
                has_children: doc.has_children,
                sort_order: doc.sort_order,
            }]);
        }

        let placeholders: String = uuids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!(
            "SELECT id, slug, title, namespace, version, updated_at, \
             parent_id, depth, has_children, sort_order \
             FROM documents WHERE id IN ({}) ORDER BY depth",
            placeholders
        );
        let mut stmt = self
            .conn
            .prepare(&query)
            .map_err(|e| Error::db("prepare breadcrumbs", e))?;
        let params: Vec<&dyn rusqlite::types::ToSql> =
            uuids.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), row_to_summary)
            .map_err(|e| Error::db("breadcrumbs query", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Count subtree size (number of descendants) for a document.
    pub fn count_descendants(&self, doc_id: &str) -> Result<usize, Error> {
        let path: String = self
            .conn
            .query_row(
                "SELECT path FROM documents WHERE id = ?1",
                params![doc_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::db("get path for count", e))?
            .unwrap_or_default();
        if path.is_empty() {
            return Ok(0);
        }
        let prefix = format!("{}/", path);
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE path LIKE ?1",
                params![prefix],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("count descendants", e))?;
        Ok(count as usize)
    }

    /// Move a document to a new parent (re-parent subtree).
    ///
    /// Updates path, depth, and sort_order for the moved node and all descendants.
    /// Returns the number of documents affected.
    pub fn move_document(
        &self,
        doc_id: &str,
        new_parent_id: Option<&str>,
        new_sort_order: Option<i64>,
    ) -> Result<usize, Error> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin move transaction", e))?;

        // Fetch current state BEFORE any mutation.
        let current: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT id, path, depth FROM documents WHERE id = ?1",
                params![doc_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()
            .map_err(|e| Error::db("fetch current document for move", e))?;

        let (cur_id, old_path, old_depth) = match current {
            Some(v) => v,
            None => return Ok(0),
        };

        let old_prefix = format!("{}/", old_path);

        // Safety checks.
        if let Some(parent) = new_parent_id {
            if parent == doc_id {
                return Err(Error::validation("cannot move document into itself"));
            }
            let parent_path: String = tx
                .query_row(
                    "SELECT path FROM documents WHERE id = ?1",
                    params![parent],
                    |row| row.get(0),
                )
                .unwrap_or_default();
            if !old_path.is_empty() && parent_path.starts_with(&old_path) {
                return Err(Error::validation(
                    "cannot move document into its own descendant",
                ));
            }
            // Depth guard: max 10.
            let parent_depth: i64 = tx
                .query_row(
                    "SELECT depth FROM documents WHERE id = ?1",
                    params![parent],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let new_depth = parent_depth + 1;
            let max_child_depth: i64 = tx
                .query_row(
                    "SELECT MAX(depth) FROM documents WHERE path LIKE ?1",
                    params![old_prefix],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .unwrap_or(None)
                .unwrap_or(old_depth);
            let depth_diff = new_depth - old_depth;
            if max_child_depth + depth_diff > MAX_DEPTH {
                return Err(Error::validation(
                    "move would exceed maximum depth of 10",
                ));
            }
        }

        // Compute new path and depth.
        let (new_path, new_depth) = if let Some(parent) = new_parent_id {
            let parent_path: String = tx
                .query_row(
                    "SELECT path FROM documents WHERE id = ?1",
                    params![parent],
                    |row| row.get(0),
                )
                .unwrap_or_default();
            let parent_depth: i64 = tx
                .query_row(
                    "SELECT depth FROM documents WHERE id = ?1",
                    params![parent],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let path = if parent_path.is_empty() {
                format!("/{}/", cur_id)
            } else {
                format!("{}{}/", parent_path, cur_id)
            };
            (path, parent_depth + 1)
        } else {
            (format!("/{}/", cur_id), 0)
        };

        let sort = new_sort_order.unwrap_or(0);
        let depth_diff = new_depth - old_depth;

        // Update the moving document.
        tx.execute(
            "UPDATE documents SET parent_id = ?1, path = ?2, depth = ?3, sort_order = ?4 \
             WHERE id = ?5",
            params![new_parent_id, new_path, new_depth, sort, doc_id],
        )
        .map_err(|e| Error::db("update moved document", e))?;

        // Update all descendants: replace old prefix with new prefix, adjust depth.
        let n = tx
            .execute(
                "UPDATE documents SET path = REPLACE(path, ?1, ?2), depth = depth + ?3 \
                 WHERE path LIKE ?4 AND id != ?5",
                params![
                    old_prefix,
                    new_path,
                    depth_diff,
                    format!("{}/%", cur_id),
                    doc_id,
                ],
            )
            .map_err(|e| Error::db("update descendant paths", e))?;

        // Update new parent's has_children flag.
        if let Some(parent) = new_parent_id {
            tx.execute(
                "UPDATE documents SET has_children = 1 WHERE id = ?1",
                params![parent],
            )
            .map_err(|e| Error::db("update new parent has_children", e))?;
        }

        tx.commit()
            .map_err(|e| Error::db("commit move", e))?;

        Ok((n + 1) as usize)
    }

    /// Delete a document by ID (cascades to children and chunks).
    ///
    /// Returns (deleted, subtree_size).
    pub fn delete_document(&self, id: &str) -> Result<(bool, usize), Error> {
        let subtree_size = self.count_descendants(id)?;
        let n = self
            .conn
            .execute(
                "DELETE FROM documents WHERE id = ?1 OR path LIKE ?2",
                params![id, format!("/{}/%", id)],
            )
            .map_err(|e| Error::db("delete document", e))?;
        Ok((n > 0, subtree_size))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::Store;

    fn open_test_store() -> Store {
        Store::open(":memory:").unwrap()
    }

    fn make_doc(id: &str, slug: &str, title: &str) -> Document {
        let now = chrono::Utc::now().to_rfc3339();
        Document {
            id: id.to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            content: "# Hello\n\nWorld".to_string(),
            namespace: "default".to_string(),
            tags: vec![],
            metadata: serde_json::Value::Null,
            version: 1,
            content_type: "markdown".to_string(),
            created_at: now.clone(),
            updated_at: now,
            parent_id: None,
            path: format!("/{}/", id),
            depth: 0,
            sort_order: 0,
            has_children: false,
        }
    }

    fn make_child_doc(id: &str, slug: &str, title: &str, parent_id: &str, parent_path: &str) -> Document {
        let mut doc = make_doc(id, slug, title);
        doc.parent_id = Some(parent_id.to_string());
        doc.path = format!("{}{}/", parent_path, id);
        doc.depth = parent_path.split('/').filter(|s| !s.is_empty()).count() as i64;
        doc.sort_order = 0;
        doc
    }

    #[test]
    fn test_document_crud() {
        let store = open_test_store();
        let doc = make_doc("doc-1", "getting-started", "Getting Started");

        // Create.
        let id = store.upsert_document(&doc).unwrap();
        assert_eq!(id, "doc-1");

        // Get by ID.
        let got = store.get_document("doc-1").unwrap().unwrap();
        assert_eq!(got.title, "Getting Started");
        assert_eq!(got.depth, 0);
        assert!(got.parent_id.is_none());

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
        let (deleted, subtree) = store.delete_document("doc-1").unwrap();
        assert!(deleted);
        assert_eq!(subtree, 0);
        assert!(store.get_document("doc-1").unwrap().is_none());
    }

    #[test]
    fn test_document_upsert_increments_version() {
        let store = open_test_store();
        let doc = make_doc("doc-1", "test", "V1");

        store.upsert_document(&doc).unwrap();

        let doc2 = Document { title: "V2".to_string(), content: "Content 2".to_string(), ..doc.clone() };
        store.upsert_document(&doc2).unwrap();

        let got = store.get_document_by_slug("test", "default").unwrap().unwrap();
        assert_eq!(got.title, "V2");
        assert_eq!(got.version, 2);
    }

    #[test]
    fn test_document_namespace_isolation() {
        let store = open_test_store();

        let doc1 = make_doc("d1", "guide", "NS1 Guide");
        let doc1 = Document { namespace: "ns1".to_string(), ..doc1 };
        let doc2 = make_doc("d2", "guide", "NS2 Guide");
        let doc2 = Document { id: "d2".to_string(), namespace: "ns2".to_string(), ..doc2 };

        store.upsert_document(&doc1).unwrap();
        store.upsert_document(&doc2).unwrap();

        assert!(store.get_document_by_slug("guide", "ns1").unwrap().is_some());
        assert!(store.get_document_by_slug("guide", "ns2").unwrap().is_some());
    }

    #[test]
    fn test_hierarchy_tree() {
        let store = open_test_store();

        // Root.
        let root = make_doc("root", "company", "Company");
        store.upsert_document(&root).unwrap();

        // Child L1.
        let child1 = make_child_doc("eng", "engineering", "Engineering", "root", "/root/");
        store.upsert_document(&child1).unwrap();

        // Child L2.
        let child2 = make_child_doc("backend", "backend", "Backend", "eng", "/root/eng/");
        store.upsert_document(&child2).unwrap();

        // Verify parent has_children.
        let root_doc = store.get_document("root").unwrap().unwrap();
        assert!(root_doc.has_children);

        // List children.
        let children = store.list_document_children("root", 10).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].slug, "engineering");

        // List descendants.
        let descendants = store.list_descendants("company", "default", None, 100).unwrap();
        assert_eq!(descendants.len(), 2); // eng + backend

        // Breadcrumbs.
        let crumbs = store.get_breadcrumbs("backend", "default").unwrap();
        assert_eq!(crumbs.len(), 3); // company -> engineering -> backend
        assert_eq!(crumbs[0].slug, "company");
        assert_eq!(crumbs[2].slug, "backend");

        // Count descendants.
        assert_eq!(store.count_descendants("root").unwrap(), 2);
        assert_eq!(store.count_descendants("eng").unwrap(), 1);
    }

    #[test]
    fn test_hierarchy_delete_cascade() {
        let store = open_test_store();

        let root = make_doc("root", "root", "Root");
        store.upsert_document(&root).unwrap();
        let child = make_child_doc("child", "child", "Child", "root", "/root/");
        store.upsert_document(&child).unwrap();
        let grandchild = make_child_doc("gc", "grandchild", "GC", "child", "/root/child/");
        store.upsert_document(&grandchild).unwrap();

        // Delete root → cascades to child + grandchild.
        let (deleted, subtree) = store.delete_document("root").unwrap();
        assert!(deleted);
        assert_eq!(subtree, 2);

        assert!(store.get_document("child").unwrap().is_none());
        assert!(store.get_document("gc").unwrap().is_none());
    }

    #[test]
    fn test_hierarchy_move() {
        let store = open_test_store();

        let root = make_doc("root", "root", "Root");
        store.upsert_document(&root).unwrap();
        let parent_a = make_child_doc("pa", "parent-a", "Parent A", "root", "/root/");
        store.upsert_document(&parent_a).unwrap();
        let parent_b = make_child_doc("pb", "parent-b", "Parent B", "root", "/root/");
        store.upsert_document(&parent_b).unwrap();
        let child = make_child_doc("child", "child", "Child", "pa", "/root/pa/");
        store.upsert_document(&child).unwrap();

        // Move child from pa to pb.
        let affected = store.move_document("child", Some("pb"), None).unwrap();
        assert_eq!(affected, 1);

        // Verify new parent.
        let child_doc = store.get_document("child").unwrap().unwrap();
        assert_eq!(child_doc.parent_id.as_deref(), Some("pb"));
        assert_eq!(child_doc.path, "/root/pb/child/");
        assert_eq!(child_doc.depth, 2);

        // Move to root.
        let affected = store.move_document("child", None, Some(99)).unwrap();
        assert_eq!(affected, 1);
        let child_doc = store.get_document("child").unwrap().unwrap();
        assert!(child_doc.parent_id.is_none());
        assert_eq!(child_doc.depth, 0);
        assert_eq!(child_doc.sort_order, 99);
    }

    #[test]
    fn test_hierarchy_move_self_reference() {
        let store = open_test_store();
        let root = make_doc("root", "root", "Root");
        store.upsert_document(&root).unwrap();

        // Can't move into itself.
        let result = store.move_document("root", Some("root"), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_hierarchy_list_root_documents() {
        let store = open_test_store();

        let root1 = make_doc("r1", "root1", "Root 1");
        let root2 = make_doc("r2", "root2", "Root 2");
        store.upsert_document(&root1).unwrap();
        store.upsert_document(&root2).unwrap();

        let child = make_child_doc("c1", "child1", "Child", "r1", "/r1/");
        store.upsert_document(&child).unwrap();

        let roots = store.list_root_documents(Some("default"), 10).unwrap();
        assert_eq!(roots.len(), 2);

        let all = store.list_documents(Some("default"), 10).unwrap();
        assert_eq!(all.len(), 3);
    }
}
