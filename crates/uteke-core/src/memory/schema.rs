//! Schema management — initialization, migrations, versioning.

use crate::Error;
use rusqlite::{params, OptionalExtension};

use super::store::{CURRENT_SCHEMA_VERSION, SCHEMA, SCHEMA_INDEXES};

impl super::Store {
    /// Run initial schema creation + legacy column migrations.
    pub(super) fn init_schema(&self) -> Result<(), Error> {
        self.conn
            .execute_batch(SCHEMA)
            .map_err(|e| Error::db("database operation", e))?;

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
                .map_err(|e| Error::db("database operation", e))?;
        }

        // Migration: add access tracking columns
        if !self.column_exists("access_count") {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(|e| Error::db("database operation", e))?;
        }
        if !self.column_exists("last_accessed") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN last_accessed TEXT;")
                .map_err(|e| Error::db("database operation", e))?;
        }

        // Migration: add temporal/deprecation columns
        if !self.column_exists("deprecated") {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN deprecated INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(|e| Error::db("database operation", e))?;
        }
        if !self.column_exists("valid_from") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN valid_from TEXT;")
                .map_err(|e| Error::db("database operation", e))?;
        }
        if !self.column_exists("valid_until") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN valid_until TEXT;")
                .map_err(|e| Error::db("database operation", e))?;
        }
        if !self.column_exists("memory_type") {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN memory_type TEXT NOT NULL DEFAULT 'fact';",
                )
                .map_err(|e| Error::db("database operation", e))?;
        }

        // Run versioned migrations (adds slug, source, source_type, etc.)
        self.ensure_schema_version()?;

        // Create indexes on migration-added columns AFTER migrations complete.
        // These are safe now — all columns exist regardless of original DB version.
        for stmt in SCHEMA_INDEXES {
            // Best-effort: ignore errors if index already exists or column
            // somehow still missing (shouldn't happen after migrations).
            if let Err(e) = self.conn.execute_batch(stmt) {
                tracing::debug!("Schema index (best-effort): {stmt} → {e}");
            }
        }

        Ok(())
    }

    /// Ensure the schema_version table exists and the database is at the
    /// correct schema version, running migrations if necessary.
    fn ensure_schema_version(&self) -> Result<(), Error> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL, applied_at TEXT NOT NULL);",
            )
            .map_err(|e| Error::db("database operation", e))?;

        let current: Option<i32> = self
            .conn
            .query_row(
                "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::db("database operation", e))?;

        match current {
            None => {
                // Fresh database — stamp version 1.
                let now = chrono::Utc::now().to_rfc3339();
                self.conn
                    .execute(
                        "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
                        params![CURRENT_SCHEMA_VERSION, now],
                    )
                    .map_err(|e| Error::db("database operation", e))?;
            }
            Some(v) if v == CURRENT_SCHEMA_VERSION => {
                // Already at the correct version — nothing to do.
            }
            Some(v) if v < CURRENT_SCHEMA_VERSION => {
                // Run migrations from v to CURRENT_SCHEMA_VERSION.
                self.run_migrations(v)?;
            }
            Some(v) => {
                return Err(Error::db_msg(format!(
                    "database schema version {v} is newer than supported version {CURRENT_SCHEMA_VERSION}; \
                     please upgrade uteke (current binary: uteke-core v{}, schema v{CURRENT_SCHEMA_VERSION})",
                    env!("CARGO_PKG_VERSION")
                )));
            }
        }

        Ok(())
    }

    /// Run incremental migrations from `from_version` (exclusive) up to
    /// `CURRENT_SCHEMA_VERSION` (inclusive).
    ///
    /// Each migration step + version stamp is wrapped in a transaction for atomicity.
    fn run_migrations(&self, from_version: i32) -> Result<(), Error> {
        let mut version = from_version;
        loop {
            version += 1;
            if version > CURRENT_SCHEMA_VERSION {
                break;
            }
            tracing::warn!("applying schema migration v{version}");

            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|e| Error::db("begin migration transaction", e))?;

            match version {
                // v2: FTS5 full-text search virtual table + sync triggers
                2 => self.migrate_v1_to_v2()?,
                // v3: Room-based collaborative memory tables
                3 => self.migrate_v2_to_v3()?,
                // v4: Importance scoring + pinned memories
                4 => self.migrate_v3_to_v4()?,
                // v5: Tag junction table for O(log n) lookups
                5 => self.migrate_v4_to_v5()?,
                // v6: Content type column (text vs json)
                6 => self.migrate_v5_to_v6()?,
                // v7: Knowledge graph tables (graph_nodes, graph_edges)
                7 => self.migrate_v6_to_v7()?,
                // v8: memory_edges table + slug column (auto-wired graph, #346)
                8 => self.migrate_v7_to_v8()?,
                // v9: timeline_events table (per-memory audit log, #347)
                9 => self.migrate_v8_to_v9()?,
                // v10: source + source_type columns (citation/provenance, #348)
                10 => self.migrate_v9_to_v10()?,
                // v11: Document engine tables (#406)
                11 => self.migrate_v10_to_v11()?,
                // v12: Hierarchical documents (#438)
                12 => self.migrate_v11_to_v12()?,
                _ => {
                    // No-op for future versions.
                }
            }

            let now = chrono::Utc::now().to_rfc3339();
            tx.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
                params![version, now],
            )
            .map_err(|e| Error::db("database operation", e))?;

            tx.commit()
                .map_err(|e| Error::db("commit migration transaction", e))?;
        }
        Ok(())
    }

    /// Return the current schema version recorded in the database.
    ///
    /// Returns an error if no version is recorded (shouldn't happen after init_schema).
    pub fn schema_version(&self) -> Result<i32, Error> {
        let version: Option<i32> = self
            .conn
            .query_row(
                "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::db("database operation", e))?;
        version.ok_or_else(|| Error::db_msg("no schema version recorded"))
    }

    /// Migration v1 → v2: Add FTS5 virtual table for hybrid search.
    fn migrate_v1_to_v2(&self) -> Result<(), Error> {
        self.init_fts5()?;

        // Rebuild FTS5 index from existing memories
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .map_err(|e| Error::db("count memories for FTS5 migration", e))?;

        if count > 0 {
            tracing::info!("Rebuilding FTS5 index for {count} existing memories...");
            self.rebuild_fts5()?;
            tracing::info!("FTS5 index rebuilt successfully.");
        }

        Ok(())
    }

    /// Migration v2 → v3: Add rooms and room_memories tables.
    fn migrate_v2_to_v3(&self) -> Result<(), Error> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS rooms (\n\
                    id TEXT PRIMARY KEY,\n\
                    title TEXT,\n\
                    namespace TEXT NOT NULL,\n\
                    created_at TEXT NOT NULL,\n\
                    updated_at TEXT NOT NULL\n\
                );\n\
                CREATE INDEX IF NOT EXISTS idx_rooms_namespace ON rooms(namespace);\n\
                \n\
                CREATE TABLE IF NOT EXISTS room_memories (\n\
                    room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,\n\
                    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,\n\
                    author TEXT NOT NULL,\n\
                    role TEXT NOT NULL DEFAULT 'participant',\n\
                    joined_at TEXT NOT NULL,\n\
                    PRIMARY KEY (room_id, memory_id)\n\
                );\n\
                CREATE INDEX IF NOT EXISTS idx_room_memories_room ON room_memories(room_id);\n\
                CREATE INDEX IF NOT EXISTS idx_room_memories_author ON room_memories(author);",
            )
            .map_err(|e| Error::db("create rooms tables", e))?;
        Ok(())
    }

    /// Migration v3 → v4: Add importance scoring and pinned columns.
    fn migrate_v3_to_v4(&self) -> Result<(), Error> {
        if !self.column_exists("importance") {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN importance REAL NOT NULL DEFAULT 0.5;",
                )
                .map_err(|e| Error::db("add importance column", e))?;
        }
        if !self.column_exists("pinned") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;")
                .map_err(|e| Error::db("add pinned column", e))?;
        }
        Ok(())
    }

    /// Migration v4 → v5: Add memory_tags junction table for O(log n) tag lookups.
    ///
    /// Creates `memory_tags` table, populates from existing JSON `tags` column.
    /// The JSON column is kept for backward compat — dual-write to both.
    fn migrate_v4_to_v5(&self) -> Result<(), Error> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS memory_tags (\n\
                    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,\n\
                    tag TEXT NOT NULL COLLATE NOCASE,\n\
                    PRIMARY KEY (memory_id, tag)\n\
                );\n\
                CREATE INDEX IF NOT EXISTS idx_memory_tags_tag ON memory_tags(tag);",
            )
            .map_err(|e| Error::db("create memory_tags table", e))?;

        // Populate from existing JSON tags column
        let mut stmt = self
            .conn
            .prepare("SELECT id, tags FROM memories")
            .map_err(|e| Error::db("select memories for tag migration", e))?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| Error::db("tag migration query", e))?
            .filter_map(|r| r.ok())
            .collect();

        let mut insert_stmt = self
            .conn
            .prepare("INSERT OR IGNORE INTO memory_tags (memory_id, tag) VALUES (?1, ?2)")
            .map_err(|e| Error::db("prepare tag insert", e))?;

        for (id, tags_str) in &rows {
            let tags: Vec<String> = serde_json::from_str(tags_str).unwrap_or_default();
            for tag in &tags {
                insert_stmt
                    .execute(rusqlite::params![id, tag])
                    .map_err(|e| Error::db("insert tag row", e))?;
            }
        }

        tracing::info!("Tag junction table populated for {} memories", rows.len());
        Ok(())
    }

    /// Migration v5 → v6: Add content_type column for structured memory support.
    ///
    /// Stores whether memory content is "text" (default) or "json".
    /// Existing memories default to "text".
    fn migrate_v5_to_v6(&self) -> Result<(), Error> {
        if !self.column_exists("content_type") {
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN content_type TEXT NOT NULL DEFAULT 'text';",
                )
                .map_err(|e| Error::db("add content_type column", e))?;
        }
        Ok(())
    }

    pub(super) fn column_exists(&self, column: &str) -> bool {
        self.conn
            .prepare("SELECT * FROM memories LIMIT 0")
            .map(|stmt| stmt.column_names().iter().any(|n| n == &column))
            .unwrap_or(false)
    }

    /// Check if a column exists in a specific table.
    pub(super) fn column_exists_in(&self, table: &str, column: &str) -> bool {
        self.conn
            .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
            .map(|stmt| stmt.column_names().iter().any(|n| n == &column))
            .unwrap_or(false)
    }

    /// v7: Knowledge graph tables (graph_nodes, graph_edges)
    fn migrate_v6_to_v7(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v6 to v7: knowledge graph tables");
        self.conn
            .execute_batch(
                r#"
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
                "#,
            )
            .map_err(|e| Error::db("schema migration v6 to v7", e))?;
        Ok(())
    }

    /// v8: memory_edges table + slug column (auto-wiring graph, #346).
    ///
    /// - Adds `slug TEXT` column to memories (nullable, for opt-in [[slug]] linking).
    /// - Creates `memory_edges` table for typed edges between memories.
    /// - Migrates any existing `metadata.relationships` JSON entries into
    ///   `memory_edges` rows so legacy data is searchable via the new table.
    fn migrate_v7_to_v8(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v7 to v8: memory_edges + slug column");

        // Add slug column if missing.
        if !self.column_exists("slug") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN slug TEXT;")
                .map_err(|e| Error::db("add slug column", e))?;
        }

        // Create memory_edges table + indices. CREATE IF NOT EXISTS is safe.
        self.conn
            .execute_batch(
                r#"
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
                "#,
            )
            .map_err(|e| Error::db("schema migration v7 to v8", e))?;

        // Slug index — hard failure since slug column was just added above.
        // If this somehow fails, the migration must not proceed (slug lookups
        // would break without the index on a non-trivial dataset).
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_memories_slug ON memories(slug) WHERE slug IS NOT NULL;",
            )
            .map_err(|e| Error::db("schema migration v7 to v8 (slug index)", e))?;

        // Backfill edges from legacy metadata.relationships JSON.
        // Shape: { "relationships": [{ "type": "supersedes", "target": "<uuid>" }, ...] }
        let mut stmt = self
            .conn
            .prepare("SELECT id, metadata FROM memories")
            .map_err(|e| Error::db("select memories for edge backfill", e))?;
        let rows: Vec<(String, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| Error::db("edge backfill query", e))?
            .filter_map(|r| r.ok())
            .collect();

        let mut insert_stmt = self
            .conn
            .prepare(
                "INSERT OR IGNORE INTO memory_edges (source_id, target_id, edge_type, created_at) \
                 SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM memories WHERE id = ?2)",
            )
            .map_err(|e| Error::db("prepare edge insert", e))?;

        let now = chrono::Utc::now().to_rfc3339();
        let mut migrated = 0usize;
        for (source_id, metadata_str) in &rows {
            let Some(meta_str) = metadata_str.as_deref() else {
                continue;
            };
            let Ok(meta_value) = serde_json::from_str::<serde_json::Value>(meta_str) else {
                continue;
            };
            let Some(rels) = meta_value.get("relationships").and_then(|v| v.as_array()) else {
                continue;
            };
            for rel in rels {
                let Some(rel_type) = rel.get("type").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(target) = rel.get("target").and_then(|v| v.as_str()) else {
                    continue;
                };
                let rows_changed = insert_stmt
                    .execute(rusqlite::params![source_id, target, rel_type, now])
                    .map_err(|e| Error::db("insert edge row", e))?;
                migrated += rows_changed;
            }
        }

        if migrated > 0 {
            tracing::info!("Backfilled {migrated} edges from legacy metadata.relationships");
        }
        Ok(())
    }

    /// v9: timeline_events table (#347).
    ///
    /// Append-only audit trail per memory (created, updated, recalled,
    /// consolidated, tagged, forgot). Table is created in the SCHEMA constant
    /// for fresh stores; this migration is for upgrading existing v8 stores.
    /// No data backfill — timeline tracking starts from this version forward.
    fn migrate_v8_to_v9(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v8 to v9: timeline_events table");
        self.conn
            .execute_batch(
                r#"
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
                "#,
            )
            .map_err(|e| Error::db("schema migration v8 to v9", e))?;
        Ok(())
    }

    /// v10: source + source_type columns (citation/provenance, #348).
    ///
    /// Adds provenance tracking to every memory. Existing rows get
    /// `source_type = 'unknown'` (legacy data without source info).
    fn migrate_v9_to_v10(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v9 to v10: source columns");
        // Guard with column_exists to handle partially-migrated databases
        // (e.g. fresh v0.3.0 DBs that already have these columns via SCHEMA).
        if !self.column_exists("source") {
            self.conn
                .execute_batch("ALTER TABLE memories ADD COLUMN source TEXT;")
                .map_err(|e| Error::db("schema migration v9 to v10", e))?;
        }
        if !self.column_exists("source_type") {
            // NOTE: DEFAULT 'unknown' for migrated rows — these are legacy
            // memories without source info. Fresh rows (via SCHEMA) use
            // DEFAULT 'user' since the code always sets source_type explicitly.
            self.conn
                .execute_batch(
                    "ALTER TABLE memories ADD COLUMN source_type TEXT NOT NULL DEFAULT 'unknown';",
                )
                .map_err(|e| Error::db("schema migration v9 to v10", e))?;
        }
        Ok(())
    }

    /// v11: Document engine tables (#406).
    ///
    /// Creates `documents` and `document_chunks` tables for wiki/knowledge
    /// base support. Full markdown content → documents table, chunked
    /// summaries → document_chunks with embeddings.
    fn migrate_v10_to_v11(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v10 to v11: document engine tables");
        self.conn
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS documents (
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
                );
                CREATE INDEX IF NOT EXISTS idx_documents_namespace ON documents(namespace);
                CREATE INDEX IF NOT EXISTS idx_documents_slug ON documents(slug);
                CREATE INDEX IF NOT EXISTS idx_documents_updated ON documents(updated_at);

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
                "#,
            )
            .map_err(|e| Error::db("schema migration v10 to v11", e))?;
        Ok(())
    }

    /// v12: Hierarchical documents (#438).
    ///
    /// Adds parent_id, path (materialized), depth, sort_order, has_children
    /// columns to the documents table for tree support with depth 10.
    fn migrate_v11_to_v12(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v11 to v12: hierarchical documents");

        // Add hierarchy columns with column_exists guards for idempotency.
        if !self.column_exists_in("documents", "parent_id") {
            self.conn
                .execute("ALTER TABLE documents ADD COLUMN parent_id TEXT REFERENCES documents(id) ON DELETE CASCADE", [])
                .map_err(|e| Error::db("migration v12: add parent_id", e))?;
        }
        if !self.column_exists_in("documents", "path") {
            self.conn
                .execute(
                    "ALTER TABLE documents ADD COLUMN path TEXT NOT NULL DEFAULT ''",
                    [],
                )
                .map_err(|e| Error::db("migration v12: add path", e))?;
        }
        if !self.column_exists_in("documents", "depth") {
            self.conn
                .execute(
                    "ALTER TABLE documents ADD COLUMN depth INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(|e| Error::db("migration v12: add depth", e))?;
        }
        if !self.column_exists_in("documents", "sort_order") {
            self.conn
                .execute(
                    "ALTER TABLE documents ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(|e| Error::db("migration v12: add sort_order", e))?;
        }
        if !self.column_exists_in("documents", "has_children") {
            self.conn
                .execute(
                    "ALTER TABLE documents ADD COLUMN has_children INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(|e| Error::db("migration v12: add has_children", e))?;
        }

        // Create indexes (best-effort — tolerate "already exists").
        let indexes = [
            "CREATE INDEX IF NOT EXISTS idx_documents_path ON documents(path)",
            "CREATE INDEX IF NOT EXISTS idx_documents_parent ON documents(parent_id)",
            "CREATE INDEX IF NOT EXISTS idx_documents_depth ON documents(depth)",
            "CREATE INDEX IF NOT EXISTS idx_documents_sort ON documents(parent_id, sort_order)",
        ];
        for idx in &indexes {
            let _ = self.conn.execute(idx, []);
        }

        // FTS5 virtual table for documents (title + slug search).
        // Best-effort: skip if FTS5 is not available (e.g., custom SQLite builds).
        let fts = [
            "CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(title, slug, content='documents', content_rowid='rowid')",
            // Triggers to keep FTS in sync with documents table.
            "CREATE TRIGGER IF NOT EXISTS documents_fts_insert AFTER INSERT ON documents BEGIN INSERT INTO documents_fts(rowid, title, slug) VALUES (new.rowid, new.title, new.slug); END",
            "CREATE TRIGGER IF NOT EXISTS documents_fts_update AFTER UPDATE ON documents BEGIN UPDATE documents_fts SET title = new.title, slug = new.slug WHERE rowid = new.rowid; END",
            "CREATE TRIGGER IF NOT EXISTS documents_fts_delete AFTER DELETE ON documents BEGIN DELETE FROM documents_fts WHERE rowid = old.rowid; END",
        ];
        for sql in &fts {
            let _ = self.conn.execute(sql, []);
        }

        Ok(())
    }
}
