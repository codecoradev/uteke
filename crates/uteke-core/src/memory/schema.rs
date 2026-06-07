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
    fn run_migrations(&self, from_version: i32) -> Result<(), Error> {
        let mut version = from_version;
        loop {
            version += 1;
            if version > CURRENT_SCHEMA_VERSION {
                break;
            }
            tracing::warn!("applying schema migration v{version}");
            #[allow(clippy::match_single_binding)]
            match version {
                // Future migrations go here, e.g.:
                // 2 => self.migrate_v1_to_v2()?,
                _ => {
                    // No-op for v1 (first versioned schema).
                }
            }
            let now = chrono::Utc::now().to_rfc3339();
            self.conn
                .execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
                    params![version, now],
                )
                .map_err(|e| Error::db("database operation", e))?;
        }
        Ok(())
    }

    /// Return the current schema version recorded in the database.
    pub fn schema_version(&self) -> Result<i32, Error> {
        let version: i32 = self
            .conn
            .query_row(
                "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("database operation", e))?;
        Ok(version)
    }

    pub(super) fn column_exists(&self, column: &str) -> bool {
        self.conn
            .prepare("SELECT * FROM memories LIMIT 0")
            .map(|stmt| stmt.column_names().iter().any(|n| n == &column))
            .unwrap_or(false)
    }
}
