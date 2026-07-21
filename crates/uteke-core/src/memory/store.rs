//! SQLite-backed persistence for memories.

use crate::memory::types::Memory;
use crate::Error;
use rusqlite::Connection;

/// Schema SQL for initial table creation.
/// Base schema — CREATE TABLE statements only.
/// Indexes on migration-added columns are in SCHEMA_INDEXES, run after migrations.
pub(super) const SCHEMA: &str = r#"
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
    memory_type TEXT NOT NULL DEFAULT 'fact',
    importance REAL NOT NULL DEFAULT 0.5,
    pinned INTEGER NOT NULL DEFAULT 0,
    content_type TEXT NOT NULL DEFAULT 'text',
    slug TEXT,
    source TEXT,
    source_type TEXT NOT NULL DEFAULT 'user'
);
CREATE INDEX IF NOT EXISTS idx_memories_tags ON memories(tags);
CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);

CREATE TABLE IF NOT EXISTS memory_tags (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    tag TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (memory_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_memory_tags_tag ON memory_tags(tag);

CREATE TABLE IF NOT EXISTS graph_nodes (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL COLLATE NOCASE,
    entity_type TEXT,
    properties_json TEXT DEFAULT '{}',
    memory_id TEXT REFERENCES memories(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS graph_edges (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
    target_id TEXT NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
    relation TEXT NOT NULL COLLATE NOCASE,
    weight REAL NOT NULL DEFAULT 1.0,
    created_at TEXT NOT NULL,
    UNIQUE(source_id, target_id, relation)
);
CREATE INDEX IF NOT EXISTS idx_graph_nodes_label ON graph_nodes(label);
CREATE INDEX IF NOT EXISTS idx_graph_edges_source ON graph_edges(source_id);
CREATE INDEX IF NOT EXISTS idx_graph_edges_target ON graph_edges(target_id);

-- memory_edges: typed auto-wired edges between memories (v8, #346).
CREATE TABLE IF NOT EXISTS memory_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    target_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    edge_type TEXT NOT NULL COLLATE NOCASE,
    created_at TEXT NOT NULL,
    UNIQUE(source_id, target_id, edge_type)
);
CREATE INDEX IF NOT EXISTS idx_memory_edges_source ON memory_edges(source_id);
CREATE INDEX IF NOT EXISTS idx_memory_edges_target ON memory_edges(target_id);
CREATE INDEX IF NOT EXISTS idx_memory_edges_type ON memory_edges(edge_type);

-- v9: Timeline events log (#347).
CREATE TABLE IF NOT EXISTS timeline_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    event_data TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_timeline_memory ON timeline_events(memory_id);
CREATE INDEX IF NOT EXISTS idx_timeline_type ON timeline_events(event_type);
CREATE INDEX IF NOT EXISTS idx_timeline_created ON timeline_events(created_at);

-- v11: Document engine (#406, #438).
CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    slug TEXT NOT NULL COLLATE NOCASE,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    namespace TEXT DEFAULT NULL,
    tags TEXT DEFAULT '[]',
    metadata TEXT DEFAULT '{}',
    version INTEGER NOT NULL DEFAULT 1,
    content_type TEXT NOT NULL DEFAULT 'markdown',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    parent_id TEXT REFERENCES documents(id) ON DELETE CASCADE,
    path TEXT NOT NULL DEFAULT '',
    depth INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    has_children INTEGER NOT NULL DEFAULT 0,
    author TEXT DEFAULT NULL,
    UNIQUE(slug)
);
-- Base indexes on columns that always exist (present in CREATE TABLE above).
CREATE INDEX IF NOT EXISTS idx_documents_slug ON documents(slug);
CREATE INDEX IF NOT EXISTS idx_documents_updated ON documents(updated_at);
-- NOTE: idx_documents_path/parent/depth/sort are NOT here.
-- They depend on columns added by migration v11→v12 (migrate_v11_to_v12),
-- so they live exclusively in that migration function to avoid referencing
-- columns that don't exist in DBs upgrading from v0.4.x (see #492).

CREATE TABLE IF NOT EXISTS document_chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    heading TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL,
    embedding BLOB,
    char_start INTEGER NOT NULL DEFAULT 0,
    char_end INTEGER NOT NULL DEFAULT 0,
    tags TEXT DEFAULT '[]',
    created_at TEXT NOT NULL,
    UNIQUE(document_id, chunk_index)
);
CREATE INDEX IF NOT EXISTS idx_doc_chunks_doc ON document_chunks(document_id);
CREATE INDEX IF NOT EXISTS idx_doc_chunks_heading ON document_chunks(heading);

-- v3: Room-based collaborative memory tables.
CREATE TABLE IF NOT EXISTS rooms (
    id TEXT PRIMARY KEY,
    title TEXT,
    namespace TEXT NOT NULL DEFAULT 'default',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rooms_namespace ON rooms(namespace);

CREATE TABLE IF NOT EXISTS room_memories (
    room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    author TEXT NOT NULL DEFAULT 'unknown',
    role TEXT NOT NULL DEFAULT 'participant',
    joined_at TEXT NOT NULL,
    PRIMARY KEY (room_id, memory_id)
);
CREATE INDEX IF NOT EXISTS idx_room_memories_room ON room_memories(room_id);
CREATE INDEX IF NOT EXISTS idx_room_memories_author ON room_memories(author);

-- v15: Room→document junction table (#689).
CREATE TABLE IF NOT EXISTS room_documents (
    room_id  TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    doc_slug TEXT NOT NULL,
    added_at TEXT NOT NULL,
    PRIMARY KEY (room_id, doc_slug)
);
CREATE INDEX IF NOT EXISTS idx_room_documents_room ON room_documents(room_id);
CREATE INDEX IF NOT EXISTS idx_room_documents_slug ON room_documents(doc_slug);

-- v16: Code indexer file tracking (DB-per-repo). Records the content hash and
-- mtime of each indexed source file so re-index can skip unchanged files and
-- prune memories for files that were deleted. Keyed by (namespace, path).
CREATE TABLE IF NOT EXISTS indexed_files (
    namespace   TEXT NOT NULL DEFAULT 'default',
    path        TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    mtime       INTEGER NOT NULL DEFAULT 0,
    chunk_count INTEGER NOT NULL DEFAULT 0,
    indexed_at  TEXT NOT NULL,
    PRIMARY KEY (namespace, path)
);
CREATE INDEX IF NOT EXISTS idx_indexed_files_ns ON indexed_files(namespace);
"#;

/// Indexes that depend on migration-added columns.
/// Run AFTER migrations complete so columns like `slug` exist.
/// Each statement is executed individually with error tolerance.
pub(super) const SCHEMA_INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_memories_namespace ON memories(namespace);",
    "CREATE INDEX IF NOT EXISTS idx_memories_deprecated ON memories(deprecated);",
    "CREATE INDEX IF NOT EXISTS idx_memories_slug ON memories(slug) WHERE slug IS NOT NULL;",
];

/// Current schema version. Increment when adding migrations.
pub(super) const CURRENT_SCHEMA_VERSION: i32 = 16;

/// Persistent SQLite store for memories.
pub struct Store {
    pub conn: Connection,
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
        .map_err(|e| Error::db("Failed to open database", e))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| Error::db("Failed to set WAL mode", e))?;
        conn.execute_batch("PRAGMA busy_timeout=5000;")
            .map_err(|e| Error::db("Failed to set busy timeout", e))?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| Error::db("Failed to enable foreign keys", e))?;

        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Wrap an existing connection as a Store and run init_schema.
    /// Used in tests to set up a pre-seeded database before migration.
    #[cfg(test)]
    fn from_conn(conn: Connection) -> Result<Self, Error> {
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Set a memory's importance score directly (0.0-1.0).
    /// Returns false if the memory does not exist.
    pub fn set_importance(&self, id: &str, importance: f64) -> Result<bool, Error> {
        if !(0.0..=1.0).contains(&importance) {
            return Err(Error::validation("importance must be between 0.0 and 1.0"));
        }
        let rows = self
            .conn
            .execute(
                "UPDATE memories SET importance = ?1 WHERE id = ?2",
                rusqlite::params![importance, id],
            )
            .map_err(|e| Error::db("set importance", e))?;
        Ok(rows > 0)
    }

    /// Pin a memory (never decays).
    pub fn pin(&self, id: &str) -> Result<bool, Error> {
        let rows = self
            .conn
            .execute(
                "UPDATE memories SET pinned = 1 WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(|e| Error::db("pin memory", e))?;
        Ok(rows > 0)
    }

    /// Unpin a memory.
    pub fn unpin(&self, id: &str) -> Result<bool, Error> {
        let rows = self
            .conn
            .execute(
                "UPDATE memories SET pinned = 0 WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(|e| Error::db("unpin memory", e))?;
        Ok(rows > 0)
    }

    /// Set source provenance on a memory (#348).
    pub fn set_source(
        &self,
        id: &str,
        source: Option<&str>,
        source_type: &str,
    ) -> Result<bool, Error> {
        let rows = self
            .conn
            .execute(
                "UPDATE memories SET source = ?1, source_type = ?2 WHERE id = ?3",
                rusqlite::params![source, source_type, id],
            )
            .map_err(|e| Error::db("set source", e))?;
        Ok(rows > 0)
    }

    /// Recalculate importance for all memories.
    /// importance = 0.3*access_score + 0.3*recency_score + 0.2*connectivity + 0.2*is_pinned
    pub fn recompute_importance(&self) -> Result<usize, Error> {
        let memories = self.load_all(None)?;
        let mut updated = 0;
        let now = chrono::Utc::now();

        for m in &memories {
            // Skip pinned — they stay at 1.0
            if m.pinned {
                if (m.importance - 1.0).abs() > f64::EPSILON {
                    self.conn
                        .execute(
                            "UPDATE memories SET importance = 1.0 WHERE id = ?1",
                            rusqlite::params![m.id],
                        )
                        .map_err(|e| Error::db("update importance", e))?;
                    updated += 1;
                }
                continue;
            }

            // access_score: normalized by count (cap at 10 accesses = 1.0)
            let access_score = (m.access_count as f64 / 10.0).min(1.0);

            // recency_score: exponential decay (half-life 30 days)
            let days_since = m
                .last_accessed
                .map(|la| (now - la).num_days().max(0) as f64)
                .unwrap_or(365.0); // Never accessed = very old
            let recency_score = (-0.693_f64 * days_since / 30.0_f64).exp(); // e^(-ln2 * days/30)

            // connectivity: count relationships in metadata
            let rel_count = m
                .metadata
                .get("relationships")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0) as f64;
            let connectivity = (rel_count / 5.0).min(1.0);

            let importance =
                0.3 * access_score + 0.3 * recency_score + 0.2 * connectivity + 0.2 * 0.0;
            let importance = importance.clamp(0.0_f64, 1.0_f64);

            if (m.importance - importance).abs() > f64::EPSILON {
                self.conn
                    .execute(
                        "UPDATE memories SET importance = ?1 WHERE id = ?2",
                        rusqlite::params![importance, m.id],
                    )
                    .map_err(|e| Error::db("update importance", e))?;
                updated += 1;
            }
        }
        Ok(updated)
    }
}

/// Serialize an embedding vector to a byte blob (little-endian f32).
pub(crate) fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        blob.extend_from_slice(&val.to_le_bytes());
    }
    blob
}

/// Deserialize an embedding vector from a byte blob.
pub(super) fn deserialize_embedding(blob: &[u8]) -> Vec<f32> {
    if blob.len() % 4 != 0 {
        tracing::warn!(
            "Embedding blob length ({}) is not a multiple of 4, trailing bytes will be skipped",
            blob.len()
        );
    }
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Parse a datetime string with fallback for missing timezone suffix.
///
/// Accepts both RFC3339 (`2026-07-09T19:53:45.493962+00:00`) and
/// ISO 8601 without timezone (`2026-07-09T19:53:45.493962`),
/// treating the latter as UTC.
fn parse_datetime_flexible(
    s: &str,
    col_index: usize,
) -> Result<chrono::DateTime<chrono::Utc>, rusqlite::Error> {
    // Fast path: strict RFC3339 (the normal case)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.to_utc());
    }
    // Fallback: try appending UTC offset for ISO 8601 without timezone
    let with_tz = format!("{s}+00:00");
    chrono::DateTime::parse_from_rfc3339(&with_tz)
        .map(|dt| dt.to_utc())
        .map_err(|e| {
            tracing::warn!(
                col = col_index,
                value = s,
                "Failed to parse datetime string (tried RFC3339 and ISO8601+UTC)"
            );
            rusqlite::Error::FromSqlConversionFailure(
                col_index,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
}

/// Optional variant of [parse_datetime_flexible] — returns None on parse failure
/// instead of an error. Used for nullable datetime columns (last_accessed, valid_from, etc).
fn parse_datetime_opt(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    parse_datetime_flexible(s, 0).ok()
}

/// Convert a database row to a Memory.
pub(crate) fn row_to_memory(row: &rusqlite::Row<'_>) -> Result<Memory, rusqlite::Error> {
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
        .and_then(|s| match serde_json::from_str::<Vec<String>>(s) {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!("Corrupted tags JSON for memory (will use empty): {e}");
                None
            }
        })
        .unwrap_or_default();
    let metadata_str: Option<String> = row.get(4)?;
    let metadata = metadata_str
        .as_deref()
        .and_then(|s| match serde_json::from_str::<serde_json::Value>(s) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("Corrupted metadata JSON for memory (will use null): {e}");
                None
            }
        })
        .unwrap_or(serde_json::Value::Null);
    let created_at_str: String = row.get(5)?;
    let created_at = parse_datetime_flexible(&created_at_str, 5)?;
    let updated_at_str: String = row.get(6)?;
    let updated_at = parse_datetime_flexible(&updated_at_str, 6)?;
    let namespace: String = row
        .get(7)
        .unwrap_or_else(|_| crate::memory::types::DEFAULT_NAMESPACE.to_string());
    let access_count: u32 = row.get(8).unwrap_or(0);
    let last_accessed_str: Option<String> = row.get(9).ok().flatten();
    let last_accessed = last_accessed_str.as_deref().and_then(parse_datetime_opt);
    let deprecated: bool = row.get(10).unwrap_or_else(|e| {
        tracing::debug!("Failed to read deprecated field: {e}, defaulting to false");
        false
    });
    let valid_from_str: Option<String> = row.get(11).ok().flatten();
    let valid_from = valid_from_str.as_deref().and_then(parse_datetime_opt);
    let valid_until_str: Option<String> = row.get(12).ok().flatten();
    let valid_until = valid_until_str.as_deref().and_then(parse_datetime_opt);
    let memory_type: String = row.get(13).unwrap_or_else(|_| "fact".to_string());
    let importance: f64 = row.get(14).unwrap_or(0.5);
    let pinned: bool = row.get(15).unwrap_or(false);
    let content_type: String = row.get(16).unwrap_or_else(|_| "text".to_string());
    let slug: Option<String> = row.get(17).ok().flatten();
    let source: Option<String> = row.get(18).ok().flatten();
    let source_type: String = row.get(19).unwrap_or_else(|_| "unknown".to_string());

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
        importance,
        pinned,
        content_type,
        slug,
        source,
        source_type,
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
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
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
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
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

        store
            .insert(&make_test_memory_ns(
                "a1",
                "app deploy",
                &["deploy"],
                "app-alpha",
            ))
            .unwrap();
        store
            .insert(&make_test_memory_ns(
                "a2",
                "app config",
                &["config"],
                "app-alpha",
            ))
            .unwrap();
        store
            .insert(&make_test_memory_ns(
                "b1",
                "cli preference",
                &["pref"],
                "cli-beta",
            ))
            .unwrap();

        assert_eq!(store.count(Some("app-alpha")).unwrap(), 2);
        assert_eq!(store.count(Some("cli-beta")).unwrap(), 1);
        assert_eq!(store.count(None).unwrap(), 3);

        let alpha_list = store.list(None, Some("app-alpha"), 10, 0).unwrap();
        assert_eq!(alpha_list.len(), 2);

        let beta_list = store.list(None, Some("cli-beta"), 10, 0).unwrap();
        assert_eq!(beta_list.len(), 1);
        assert_eq!(beta_list[0].content, "cli preference");

        let alpha_search = store
            .search_content("deploy", Some("app-alpha"), 10)
            .unwrap();
        assert_eq!(alpha_search.len(), 1);

        let beta_search = store
            .search_content("deploy", Some("cli-beta"), 10)
            .unwrap();
        assert_eq!(beta_search.len(), 0);

        let ns = store.list_namespaces().unwrap();
        assert_eq!(ns.len(), 2);
        assert!(ns.contains(&"app-alpha".to_string()));
        assert!(ns.contains(&"cli-beta".to_string()));
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

        let rust_memories = store.list(Some("rust"), None, 10, 0).unwrap();
        assert_eq!(rust_memories.len(), 2);
        let ids: Vec<&str> = rust_memories.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"1"));
        assert!(ids.contains(&"3"));

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
        assert_eq!(info[0].name, "rust");
        assert_eq!(info[0].count, 2);
        assert_eq!(info.len(), 4);

        let total: usize = info.iter().map(|t| t.count).sum();
        assert_eq!(total, 5);
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

        let old_matches = store.list(Some("old-tag"), None, 10, 0).unwrap();
        assert_eq!(old_matches.len(), 0);

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

        store
            .insert(&make_test_memory("1", "a", &["tag-with-dash"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["tag.with.dots"]))
            .unwrap();
        store
            .insert(&make_test_memory("3", "c", &["tag_with_underscore"]))
            .unwrap();

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

        store
            .insert(&make_test_memory("1", "a", &["rust"]))
            .unwrap();
        store
            .insert(&make_test_memory("2", "b", &["rustacean"]))
            .unwrap();

        let rust_only = store.list(Some("rust"), None, 10, 0).unwrap();
        assert_eq!(rust_only.len(), 1);
        assert_eq!(rust_only[0].id, "1");

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
        assert_eq!(store.count(Some("ns-b")).unwrap(), 1);
    }

    #[test]
    fn test_bulk_delete_cold() {
        let store = Store::open(":memory:").unwrap();
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
        store.deprecate("nonexistent").unwrap();
    }

    #[test]
    fn test_tier_counts() {
        let store = Store::open(":memory:").unwrap();
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

        let results = store.search_content("Hello", None, 10).unwrap();
        assert_eq!(results.len(), 2);

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

        let page1 = store.list(None, None, 3, 0).unwrap();
        assert_eq!(page1.len(), 3);

        let page2 = store.list(None, None, 3, 3).unwrap();
        assert_eq!(page2.len(), 3);

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
        let path = store.path();
        if let Some(p) = path {
            assert!(p.to_string_lossy().is_empty() || p.to_string_lossy() == ":memory:");
        }
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
        let aged = store.find_aged(30, 0, None).unwrap();
        assert!(aged.is_empty());
    }

    #[test]
    fn test_find_aged_with_recent_memories() {
        let store = Store::open(":memory:").unwrap();
        store
            .insert(&make_test_memory("recent", "recent memory", &[]))
            .unwrap();

        let aged = store.find_aged(999, 0, None).unwrap();
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
        assert_eq!(similar.len(), 2);
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

        let updated = store.rename_tag("old", "new", Some("ns-a")).unwrap();
        assert_eq!(updated, 1);

        let ns_a_tags = store.unique_tags(Some("ns-a")).unwrap();
        assert!(ns_a_tags.contains(&"new".to_string()));
        assert!(!ns_a_tags.contains(&"old".to_string()));

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

        let result = store.insert(&make_test_memory("dup", "second", &[]));
        assert!(result.is_err());
    }

    fn make_temporal_memory(
        id: &str,
        content: &str,
        created_at: chrono::DateTime<Utc>,
        valid_from: Option<chrono::DateTime<Utc>>,
        valid_until: Option<chrono::DateTime<Utc>>,
        deprecated: bool,
    ) -> Memory {
        Memory {
            id: id.to_string(),
            content: content.to_string(),
            embedding: vec![0.1; 768],
            tags: vec![],
            metadata: serde_json::json!({}),
            created_at,
            updated_at: created_at,
            namespace: crate::memory::types::DEFAULT_NAMESPACE.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated,
            valid_from,
            valid_until,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
        }
    }

    #[test]
    fn test_list_at_time_basic() {
        let store = Store::open(":memory:").unwrap();
        let t0 = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t2 = chrono::DateTime::parse_from_rfc3339("2026-12-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Memory created at t0, still valid → exists at t1
        store
            .insert(&make_temporal_memory("m1", "old", t0, None, None, false))
            .unwrap();
        // Memory created at t2 → did NOT exist at t1
        store
            .insert(&make_temporal_memory("m2", "future", t2, None, None, false))
            .unwrap();

        let results = store.list_at_time(None, None, 100, 0, t1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "m1");
    }

    #[test]
    fn test_list_at_time_excludes_expired() {
        let store = Store::open(":memory:").unwrap();
        let t0 = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t2 = chrono::DateTime::parse_from_rfc3339("2026-12-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // valid_until = t1 → expired AT t1 (valid_until > pit required)
        store
            .insert(&make_temporal_memory(
                "expired",
                "gone",
                t0,
                None,
                Some(t1),
                false,
            ))
            .unwrap();
        // valid_until = t2 → still valid at t1
        store
            .insert(&make_temporal_memory(
                "active",
                "here",
                t0,
                None,
                Some(t2),
                false,
            ))
            .unwrap();

        let results = store.list_at_time(None, None, 100, 0, t1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "active");
    }

    #[test]
    fn test_list_at_time_excludes_deprecated() {
        let store = Store::open(":memory:").unwrap();
        let t0 = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .insert(&make_temporal_memory(
                "dep",
                "deprecated",
                t0,
                None,
                None,
                true,
            ))
            .unwrap();
        store
            .insert(&make_temporal_memory("ok", "active", t0, None, None, false))
            .unwrap();

        let results = store.list_at_time(None, None, 100, 0, t1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "ok");
    }

    #[test]
    fn test_list_at_time_excludes_future_valid_from() {
        let store = Store::open(":memory:").unwrap();
        let t0 = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t2 = chrono::DateTime::parse_from_rfc3339("2026-12-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // valid_from = t2 → not yet valid at t1
        store
            .insert(&make_temporal_memory(
                "future_vf",
                "scheduled",
                t0,
                Some(t2),
                None,
                false,
            ))
            .unwrap();
        // valid_from = t0 → valid at t1
        store
            .insert(&make_temporal_memory(
                "past_vf",
                "ready",
                t0,
                Some(t0),
                None,
                false,
            ))
            .unwrap();

        let results = store.list_at_time(None, None, 100, 0, t1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "past_vf");
    }

    #[test]
    fn test_set_source() {
        let store = Store::open(":memory:").unwrap();
        let m = make_test_memory("src1", "test source", &["t"]);
        store.insert(&m).unwrap();

        // Set source.
        assert!(store
            .set_source("src1", Some("https://rust-lang.org"), "url")
            .unwrap());

        // Verify via direct SQL.
        let (source, source_type): (Option<String>, String) = store
            .conn
            .query_row(
                "SELECT source, source_type FROM memories WHERE id = ?1",
                rusqlite::params!["src1"],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(source.as_deref(), Some("https://rust-lang.org"));
        assert_eq!(source_type, "url");
    }

    // ────────────────────────────────────────────────────────────
    // Migration upgrade-path tests (regression guard for #492)
    // ────────────────────────────────────────────────────────────

    /// Simulate a v0.4.x database (schema_version=11) with a documents table
    /// that lacks the hierarchical columns (parent_id, path, depth, sort_order,
    /// has_children). Verify that Store::open succeeds — meaning init_schema +
    /// migration v11→v12 complete without error.
    #[test]
    fn test_migration_v11_to_v12_from_v04x_db() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        // 1. Create the base memories table (present in all versions).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                embedding BLOB,
                tags TEXT DEFAULT '[]',
                metadata TEXT DEFAULT '{}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                namespace TEXT NOT NULL DEFAULT 'default',
                access_count INTEGER NOT NULL DEFAULT 0,
                last_accessed TEXT,
                deprecated INTEGER NOT NULL DEFAULT 0,
                valid_from TEXT,
                valid_until TEXT,
                memory_type TEXT NOT NULL DEFAULT 'fact',
                importance REAL NOT NULL DEFAULT 0.5,
                pinned INTEGER NOT NULL DEFAULT 0,
                content_type TEXT NOT NULL DEFAULT 'text',
                slug TEXT,
                source TEXT,
                source_type TEXT NOT NULL DEFAULT 'user'
            );",
        )
        .unwrap();

        // 2. Create the documents table as it existed in v0.4.x — WITHOUT
        //    hierarchy columns (parent_id, path, depth, sort_order, has_children).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                slug TEXT NOT NULL COLLATE NOCASE,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                namespace TEXT NOT NULL DEFAULT 'default',
                tags TEXT DEFAULT '[]',
                metadata TEXT DEFAULT '{}',
                version INTEGER NOT NULL DEFAULT 1,
                content_type TEXT NOT NULL DEFAULT 'markdown',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(namespace, slug)
            );",
        )
        .unwrap();

        // 3. Set schema_version to 11 (v0.4.x state).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL
            );
            INSERT INTO schema_version (version, applied_at) VALUES (11, '2026-06-27T00:00:00Z');",
        )
        .unwrap();

        // Insert one memory so the DB isn't empty.
        conn.execute(
            "INSERT INTO memories (id, content, embedding, created_at, updated_at) VALUES (?1, ?2, X'', ?3, ?4)",
            rusqlite::params!["mem-1", "hello", "2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z"],
        )
        .unwrap();

        // 4. Verify the hierarchy columns do NOT exist yet.
        let has_parent: bool = conn
            .prepare("SELECT parent_id FROM documents LIMIT 0")
            .is_ok();
        assert!(!has_parent, "parent_id should not exist before migration");

        // 5. Now open the Store — this triggers init_schema → ensure_schema_version → migrate_v11_to_v12.
        let store = Store::from_conn(conn).unwrap();

        // 6. Verify migration succeeded: schema_version should now be 12.
        let version: i32 = store
            .conn
            .query_row(
                "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 16, "schema_version should be 16 after migration");

        // 7. Verify hierarchy columns now exist (in documents table).
        let cols = ["parent_id", "path", "depth", "sort_order", "has_children"];
        for col in &cols {
            let exists = store.column_exists_in("documents", col);
            assert!(exists, "column {col} should exist after migration");
        }

        // 8. Verify indexes were created.
        let indexes = [
            "idx_documents_path",
            "idx_documents_parent",
            "idx_documents_depth",
            "idx_documents_sort",
        ];
        for idx in &indexes {
            let count: i32 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    rusqlite::params![idx],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "index {idx} should exist after migration");
        }
    }
}
