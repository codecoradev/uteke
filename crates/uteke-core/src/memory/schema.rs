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

        // Post-migration consistency checks: repair partially-migrated databases
        // where a migration stamped the version but failed partway through (#500).
        self.ensure_documents_has_children()?;
        self.ensure_documents_fts()?;

        // Ensure FTS5 virtual table and sync triggers exist (#544).
        //
        // Fresh databases skip all migrations (version is stamped directly at
        // CURRENT_SCHEMA_VERSION), so migrate_v1_to_v2() — which calls init_fts5()
        // — never runs. Existing databases at the correct version also skip
        // migrations. In both cases FTS5 may be missing even though memories
        // exist, causing FTS5 recall to return empty results.
        //
        // init_fts5() is idempotent (CREATE IF NOT EXISTS / TRIGGER IF NOT EXISTS),
        // so it's safe to call on every open.
        if !self.fts5_exists()? {
            tracing::info!("FTS5 virtual table missing — initializing (#544)");
            self.init_fts5()?;
            let count: i64 = self
                .conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
                .unwrap_or(0);
            if count > 0 {
                tracing::info!("Rebuilding FTS5 index for {count} existing memories");
                self.rebuild_fts5()?;
            }
        }

        // Repair datetime strings that lack timezone suffix.
        //
        // Older binaries (or manual inserts) may have written ISO 8601 strings
        // without the mandatory RFC3339 timezone offset (e.g.
        // "2026-07-09T19:53:45.493962" instead of
        // "2026-07-09T19:53:45.493962+00:00"). This causes
        // chrono::DateTime::parse_from_rfc3339() to fail with "premature end
        // of input", crashing load_all().
        self.repair_datetime_timezones()?;

        Ok(())
    }

    /// Fix datetime columns that are missing the RFC3339 timezone suffix.
    ///
    /// Affected columns: `created_at`, `updated_at` in `memories` and
    /// `created_at`, `updated_at` in `documents`. The fix appends `+00:00`
    /// (assumes UTC) to any datetime string that doesn't contain a timezone
    /// offset. This is idempotent — already-correct rows are untouched.
    fn repair_datetime_timezones(&self) -> Result<(), Error> {
        let datetime_cols: &[(&str, &str)] = &[
            ("memories", "created_at"),
            ("memories", "updated_at"),
            ("documents", "created_at"),
            ("documents", "updated_at"),
        ];

        let mut total_fixed = 0u64;

        for &(table, column) in datetime_cols {
            // Find rows where the datetime string doesn't contain '+' or 'Z' or
            // a timezone-like offset pattern. Valid RFC3339 always ends with
            // +HH:MM, -HH:MM, Z, or +00:00.
            let sql = format!(
                "SELECT id, {column} FROM {table} \
                 WHERE {column} IS NOT NULL \
                 AND {column} NOT LIKE '%+00:00' \
                 AND {column} NOT LIKE '%+%' \
                 AND {column} NOT LIKE '%Z'"
            );

            let rows: Vec<(String, String)> = {
                let mut stmt = self
                    .conn
                    .prepare(&sql)
                    .map_err(|e| Error::db("repair datetime: query", e))?;
                let iter = stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(|e| Error::db("repair datetime: iterate", e))?
                    .filter_map(|r| r.ok());
                // Drop stmt before the iterator temporary is dropped,
                // avoiding a borrow-checker conflict (#E0597).
                let rows: Vec<(String, String)> = iter.collect();
                drop(stmt);
                rows
            };

            if !rows.is_empty() {
                tracing::info!(
                    table,
                    column,
                    count = rows.len(),
                    "Repairing datetime strings missing timezone suffix"
                );
            }

            for (id, val) in &rows {
                let fixed = format!("{val}+00:00");
                let update_sql = format!("UPDATE {table} SET {column} = ?1 WHERE id = ?2");
                self.conn
                    .execute(&update_sql, params![fixed, id])
                    .map_err(|e| Error::db("repair datetime: update", e))?;
                total_fixed += 1;
            }
        }

        if total_fixed > 0 {
            tracing::info!(total_fixed, "Datetime timezone repair complete");
        }

        Ok(())
    }

    /// Ensure the `has_children` column exists on the `documents` table.
    ///
    /// Schema v12 migration (`migrate_v11_to_v12`) adds this column, but a
    /// partially-migrated DB may have schema_version=12 without the column.
    /// This check runs after every `ensure_schema_version()` call (including on
    /// open) so the column is repaired on next access.
    fn ensure_documents_has_children(&self) -> Result<(), Error> {
        // Only relevant for schema v12+ databases.
        let version = self.schema_version().unwrap_or(0);
        if version < 12 {
            return Ok(());
        }
        if !self.column_exists_in("documents", "has_children") {
            tracing::warn!(
                "documents.has_children column missing on schema v{version} DB — repairing (#500)"
            );
            self.conn
                .execute(
                    "ALTER TABLE documents ADD COLUMN has_children INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(|e| Error::db("repair has_children column (#500)", e))?;
        }
        Ok(())
    }

    /// Ensure the `documents_fts` FTS5 virtual table exists and is populated (#549).
    ///
    /// Schema v12 migration creates this table with best-effort (`let _ = execute(...)`)
    /// which means a silent failure leaves the table absent. Existing documents are never
    /// backfilled. This check runs after every `ensure_schema_version()` call so the
    /// table is created and populated on next access.
    fn ensure_documents_fts(&self) -> Result<(), Error> {
        // Only relevant for schema v12+ databases.
        let version = self.schema_version().unwrap_or(0);
        if version < 12 {
            return Ok(());
        }

        // Check if the FTS table already exists.
        let exists: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='documents_fts')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("check documents_fts exists (#549)", e))?;

        if !exists {
            tracing::warn!(
                "documents_fts table missing on schema v{} DB — creating (#549)",
                version
            );
            // Create FTS5 table.
            self.conn
                .execute_batch(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(title, slug, content='documents', content_rowid='rowid')",
                )
                .map_err(|e| Error::db("create documents_fts (#549)", e))?;

            // Create sync triggers.
            self.conn.execute_batch(
                "CREATE TRIGGER IF NOT EXISTS documents_fts_insert AFTER INSERT ON documents BEGIN                  INSERT INTO documents_fts(rowid, title, slug) VALUES (new.rowid, new.title, new.slug); END;                  CREATE TRIGGER IF NOT EXISTS documents_fts_update AFTER UPDATE ON documents BEGIN                  UPDATE documents_fts SET title = new.title, slug = new.slug WHERE rowid = new.rowid; END;                  CREATE TRIGGER IF NOT EXISTS documents_fts_delete AFTER DELETE ON documents BEGIN                  DELETE FROM documents_fts WHERE rowid = old.rowid; END;",
            ).map_err(|e| Error::db("create documents_fts triggers (#549)", e))?;

            // Backfill existing documents into FTS index.
            self.conn
                .execute_batch(
                    "INSERT INTO documents_fts(rowid, title, slug)                      SELECT rowid, title, slug FROM documents",
                )
                .map_err(|e| Error::db("backfill documents_fts (#549)", e))?;

            tracing::info!(
                "documents_fts table created and backfilled from existing documents (#549)"
            );
        }

        Ok(())
    }

    /// Ensure all expected columns exist for the current schema version.
    ///
    /// Called by `uteke repair` to fix partially-migrated databases.
    pub fn ensure_schema_consistency(&self) -> Result<(), Error> {
        self.ensure_documents_has_children()?;
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
                // v13: Global documents — remove namespace isolation (#614)
                13 => self.migrate_v12_to_v13()?,
                // v14: Add memory_type to FTS5 index (#662)
                14 => self.migrate_v13_to_v14()?,
                // v15: room_documents junction table (#689)
                15 => self.migrate_v14_to_v15()?,
                // v16: indexed_files table (code indexer, DB-per-repo)
                16 => self.migrate_v15_to_v16()?,
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

    /// v13: Global documents — remove namespace isolation (#614).
    ///
    /// - Drop old UNIQUE(namespace, slug) index and create UNIQUE(slug) index
    /// - Add `author TEXT DEFAULT NULL` column
    /// - Migrate duplicate slugs across namespaces: rename to `slug-nsname`
    fn migrate_v12_to_v13(&self) -> Result<(), Error> {
        tracing::info!(
            "Applying schema migration v12 to v13: global documents (remove namespace isolation)"
        );

        // Add author column if missing.
        if !self.column_exists_in("documents", "author") {
            self.conn
                .execute(
                    "ALTER TABLE documents ADD COLUMN author TEXT DEFAULT NULL",
                    [],
                )
                .map_err(|e| Error::db("migration v13: add author column", e))?;
        }

        // Migrate duplicate slugs: find slugs that exist in multiple namespaces.
        // Rename conflicting ones to `slug-<namespace>`.
        let dup_slugs: Vec<(String, String)> = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT slug, namespace FROM documents \
                     WHERE slug IN (SELECT slug FROM documents GROUP BY slug HAVING COUNT(DISTINCT namespace) > 1) \
                     ORDER BY slug, namespace",
                )
                .map_err(|e| Error::db("migration v13: find duplicate slugs", e))?;
            let rows: Vec<(String, String)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| Error::db("migration v13: query duplicate slugs", e))?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        if !dup_slugs.is_empty() {
            tracing::info!(
                "Migration v13: renaming {} duplicate slug entries across namespaces",
                dup_slugs.len()
            );
        }

        // Keep the first occurrence of each slug (from 'default' namespace if present),
        // rename the rest. We track which slugs we've already "kept".
        let mut kept: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (slug, ns) in &dup_slugs {
            if kept.contains(slug) {
                let new_slug = format!("{}-{}", slug, ns);
                tracing::info!(
                    "Migration v13: renaming slug '{}' in namespace '{}' to '{}'",
                    slug,
                    ns,
                    new_slug
                );
                self.conn
                    .execute(
                        "UPDATE documents SET slug = ?1 WHERE slug = ?2 AND namespace = ?3",
                        params![new_slug, slug, ns],
                    )
                    .map_err(|e| Error::db("migration v13: rename duplicate slug", e))?;
            } else {
                kept.insert(slug.clone());
            }
        }

        // Drop old unique index on (namespace, slug) if it exists.
        // SQLite auto-creates an index for UNIQUE constraints; the name may vary.
        // We try common names and ignore errors (index may not exist).
        for idx_name in &["sqlite_autoindex_documents_1", "idx_documents_ns_slug"] {
            let _ = self
                .conn
                .execute(&format!("DROP INDEX IF EXISTS {}", idx_name), []);
        }

        // Drop the per-namespace index if it exists.
        let _ = self
            .conn
            .execute("DROP INDEX IF EXISTS idx_documents_namespace", []);

        // Create new unique index on slug only.
        // We need to handle the case where duplicates might still exist (race safety).
        self.conn
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_documents_slug_unique ON documents(slug)",
                [],
            )
            .map_err(|e| Error::db("migration v13: create unique slug index", e))?;

        tracing::info!("Migration v12 to v13 complete: documents are now global");
        Ok(())
    }

    /// v14: Add memory_type column to FTS5 index (#662).
    ///
    /// FTS5 virtual tables cannot be ALTERed, so we must:
    /// 1. Drop existing triggers (they reference old column set)
    /// 2. Drop existing FTS5 table
    /// 3. Recreate with memory_type column
    /// 4. Rebuild index from existing memories
    fn migrate_v13_to_v14(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v13 to v14: add memory_type to FTS5 index");

        // Drop old triggers first (they reference the old column set).
        for trigger_name in &["memories_fts_ai", "memories_fts_ad", "memories_fts_au"] {
            let _ = self
                .conn
                .execute(&format!("DROP TRIGGER IF EXISTS {}", trigger_name), []);
        }

        // Drop old FTS5 table.
        let _ = self.conn.execute("DROP TABLE IF EXISTS memories_fts", []);

        // Recreate FTS5 with memory_type column.
        self.init_fts5()?;

        // Rebuild FTS5 index from existing memories.
        self.rebuild_fts5()?;

        tracing::info!("Migration v13 to v14 complete: memory_type now in FTS5 index");
        Ok(())
    }

    /// v15: Add room_documents junction table for room→document associations (#689).
    fn migrate_v14_to_v15(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v14 to v15: room_documents junction table");

        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS room_documents (
                     room_id  TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                     doc_slug TEXT NOT NULL,
                     added_at TEXT NOT NULL,
                     PRIMARY KEY (room_id, doc_slug)
                 )",
            )
            .map_err(|e| Error::db("create room_documents table", e))?;

        tracing::info!("Migration v14 to v15 complete: room_documents table created");
        Ok(())
    }

    /// v16: indexed_files table for the code indexer (DB-per-repo).
    ///
    /// Tracks content hash + mtime per source file so `uteke index` can skip
    /// unchanged files and prune memories for deleted files. Table is also in
    /// the SCHEMA constant for fresh stores; this migration upgrades existing
    /// v15 stores. No backfill — tracking starts from first index run.
    fn migrate_v15_to_v16(&self) -> Result<(), Error> {
        tracing::info!("Applying schema migration v15 to v16: indexed_files table");
        self.conn
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS indexed_files (
                    namespace    TEXT NOT NULL DEFAULT 'default',
                    path         TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    mtime        INTEGER NOT NULL DEFAULT 0,
                    chunk_count  INTEGER NOT NULL DEFAULT 0,
                    indexed_at   TEXT NOT NULL,
                    PRIMARY KEY (namespace, path)
                );
                CREATE INDEX IF NOT EXISTS idx_indexed_files_ns ON indexed_files(namespace);
                "#,
            )
            .map_err(|e| Error::db("schema migration v15 to v16", e))?;
        Ok(())
    }
}
