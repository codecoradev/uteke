//! Schema management — initialization, migrations, versioning.

use crate::Error;
use rusqlite::{params, OptionalExtension};

use super::store::{CURRENT_SCHEMA_VERSION, SCHEMA};

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

        // Create namespace index (safe after column exists)
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_memories_namespace ON memories(namespace);",
            )
            .map_err(|e| Error::db("database operation", e))?;

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

        // Create deprecation index
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_memories_deprecated ON memories(deprecated);",
            )
            .map_err(|e| Error::db("database operation", e))?;

        self.ensure_schema_version()?;

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
                     please upgrade uteke"
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

            #[allow(clippy::match_single_binding)]
            match version {
                // v2: FTS5 full-text search virtual table + sync triggers
                2 => self.migrate_v1_to_v2()?,
                // v3: Room-based collaborative memory tables
                3 => self.migrate_v2_to_v3()?,
                // v4: Importance scoring + pinned memories
                4 => self.migrate_v3_to_v4()?,
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

    pub(super) fn column_exists(&self, column: &str) -> bool {
        self.conn
            .prepare("SELECT * FROM memories LIMIT 0")
            .map(|stmt| stmt.column_names().iter().any(|n| n == &column))
            .unwrap_or(false)
    }
}
