//! Tag operations — unique tags, counts, rename, delete, namespaces.
//!
//! Uses the `memory_tags` junction table (schema v5) for O(log n) lookups.
//! The JSON `tags` column in `memories` is kept in sync for backward compat.

use crate::Error;
use crate::memory::types::TagInfo;
use rusqlite::{OptionalExtension, params};

impl super::Store {
    /// Get all unique tags, optionally filtered by namespace.
    ///
    /// Queries the `memory_tags` junction table directly — O(log n).
    pub fn unique_tags(&self, namespace: Option<&str>) -> Result<Vec<String>, Error> {
        let sql = match namespace {
            Some(_) => {
                "SELECT DISTINCT mt.tag FROM memory_tags mt \
                 INNER JOIN memories m ON mt.memory_id = m.id \
                 WHERE m.namespace = ?1 \
                 ORDER BY mt.tag"
            }
            None => "SELECT DISTINCT tag FROM memory_tags ORDER BY tag",
        };

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("database operation", e))?;

        let rows = match namespace {
            Some(ns) => stmt
                .query_map(params![ns], |row: &rusqlite::Row| row.get::<_, String>(0))
                .map_err(|e| Error::db("database operation", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("database operation", e))?,
            None => stmt
                .query_map([], |row: &rusqlite::Row| row.get::<_, String>(0))
                .map_err(|e| Error::db("database operation", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("database operation", e))?,
        };

        Ok(rows)
    }

    /// List all tags with their usage counts.
    ///
    /// Single GROUP BY query on `memory_tags` — much faster than json_each().
    pub fn tags_with_counts(&self, namespace: Option<&str>) -> Result<Vec<TagInfo>, Error> {
        let mut result = Vec::new();
        match namespace {
            Some(ns) => {
                let sql = "SELECT mt.tag AS name, COUNT(*) AS count \
                           FROM memory_tags mt \
                           INNER JOIN memories m ON mt.memory_id = m.id \
                           WHERE m.namespace = ?1 \
                           GROUP BY mt.tag \
                           ORDER BY count DESC";
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("database operation", e))?;
                let rows = stmt
                    .query_map(params![ns], |row| {
                        Ok(TagInfo {
                            name: row.get(0)?,
                            count: row.get::<_, i64>(1)? as usize,
                        })
                    })
                    .map_err(|e| Error::db("database operation", e))?;
                for row in rows {
                    result.push(row.map_err(|e| Error::db("database operation", e))?);
                }
            }
            None => {
                let sql = "SELECT tag AS name, COUNT(*) AS count \
                           FROM memory_tags \
                           GROUP BY tag \
                           ORDER BY count DESC";
                let mut stmt = self
                    .conn
                    .prepare(sql)
                    .map_err(|e| Error::db("database operation", e))?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(TagInfo {
                            name: row.get(0)?,
                            count: row.get::<_, i64>(1)? as usize,
                        })
                    })
                    .map_err(|e| Error::db("database operation", e))?;
                for row in rows {
                    result.push(row.map_err(|e| Error::db("database operation", e))?);
                }
            }
        }
        Ok(result)
    }

    /// Rename a tag across all memories. Returns number updated.
    ///
    /// Updates both `memory_tags` junction table AND JSON `tags` column.
    /// Wrapped in a transaction for atomicity.
    pub fn rename_tag(
        &self,
        old: &str,
        new: &str,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        // Start transaction BEFORE read to prevent TOCTOU race.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin transaction", e))?;

        // Find affected memory IDs from junction table
        let (sql, ns_param): (&str, Option<&str>) = match namespace {
            Some(_) => (
                "SELECT mt.memory_id FROM memory_tags mt \
                 INNER JOIN memories m ON mt.memory_id = m.id \
                 WHERE mt.tag = ?1 AND m.namespace = ?2",
                namespace,
            ),
            None => ("SELECT memory_id FROM memory_tags WHERE tag = ?1", None),
        };
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("database operation", e))?;

        let memory_ids: Vec<String> = match ns_param {
            Some(ns) => stmt
                .query_map(params![old, ns], |row| row.get::<_, String>(0))
                .map_err(|e| Error::db("database operation", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("database operation", e))?,
            None => stmt
                .query_map(params![old], |row| row.get::<_, String>(0))
                .map_err(|e| Error::db("database operation", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("database operation", e))?,
        };

        let mut updated = 0;

        // Update junction table: delete old, insert new for each memory
        let mut del_stmt = self
            .conn
            .prepare("DELETE FROM memory_tags WHERE memory_id = ?1 AND tag = ?2")
            .map_err(|e| Error::db("database operation", e))?;
        let mut ins_stmt = self
            .conn
            .prepare("INSERT OR IGNORE INTO memory_tags (memory_id, tag) VALUES (?1, ?2)")
            .map_err(|e| Error::db("database operation", e))?;

        for mid in &memory_ids {
            del_stmt
                .execute(params![mid, old])
                .map_err(|e| Error::db("database operation", e))?;
            ins_stmt
                .execute(params![mid, new])
                .map_err(|e| Error::db("database operation", e))?;

            // Also update JSON tags column for backward compat
            let tags_str: Option<String> = self
                .conn
                .query_row(
                    "SELECT tags FROM memories WHERE id = ?1",
                    params![mid],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| Error::db("database operation", e))?;

            if let Some(ts) = tags_str {
                let mut tags: Vec<String> = match serde_json::from_str(&ts) {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(
                            "Skipping rename for memory {mid}: corrupted tags JSON: {e}"
                        );
                        continue;
                    }
                };
                let mut changed = false;
                for t in &mut tags {
                    if t == old {
                        *t = new.to_string();
                        changed = true;
                    }
                }
                if changed {
                    // A memory that already carried `new` must not end up with it
                    // twice in the JSON column (#1332). Keep first occurrences.
                    let mut seen = std::collections::HashSet::new();
                    tags.retain(|t| seen.insert(t.clone()));
                    let new_tags_json = serde_json::to_string(&tags)
                        .map_err(|e| Error::db("database operation", e))?;
                    let now = chrono::Utc::now().to_rfc3339();
                    self.conn
                        .execute(
                            "UPDATE memories SET tags = ?1, updated_at = ?2 WHERE id = ?3",
                            params![new_tags_json, now, mid],
                        )
                        .map_err(|e| Error::db("database operation", e))?;
                    updated += 1;
                } else {
                    // Junction table updated but JSON was already correct — no net change.
                    // Revert junction table change to keep both in sync.
                    del_stmt
                        .execute(params![mid, new])
                        .map_err(|e| Error::db("database operation", e))?;
                }
            } else {
                // No JSON tags row — shouldn't happen, but junction table is already correct.
                updated += 1;
            }
        }

        tx.commit()
            .map_err(|e| Error::db("commit transaction", e))?;
        Ok(updated)
    }

    /// Delete a tag from all memories. Returns number updated.
    ///
    /// Deletes from `memory_tags` junction table AND updates JSON `tags` column.
    /// Wrapped in a transaction for atomicity.
    pub fn delete_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        // Start transaction BEFORE read to prevent TOCTOU race.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin transaction", e))?;

        // Find affected memory IDs from junction table
        let (sql, ns_param): (&str, Option<&str>) = match namespace {
            Some(_) => (
                "SELECT mt.memory_id FROM memory_tags mt \
                 INNER JOIN memories m ON mt.memory_id = m.id \
                 WHERE mt.tag = ?1 AND m.namespace = ?2",
                namespace,
            ),
            None => ("SELECT memory_id FROM memory_tags WHERE tag = ?1", None),
        };
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("database operation", e))?;

        let memory_ids: Vec<String> = match ns_param {
            Some(ns) => stmt
                .query_map(params![tag, ns], |row| row.get::<_, String>(0))
                .map_err(|e| Error::db("database operation", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("database operation", e))?,
            None => stmt
                .query_map(params![tag], |row| row.get::<_, String>(0))
                .map_err(|e| Error::db("database operation", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("database operation", e))?,
        };

        let mut updated = 0;

        for mid in &memory_ids {
            // Delete from junction table
            self.conn
                .execute(
                    "DELETE FROM memory_tags WHERE memory_id = ?1 AND tag = ?2",
                    params![mid, tag],
                )
                .map_err(|e| Error::db("database operation", e))?;

            // Also update JSON tags column for backward compat
            let tags_str: Option<String> = self
                .conn
                .query_row(
                    "SELECT tags FROM memories WHERE id = ?1",
                    params![mid],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| Error::db("database operation", e))?;

            if let Some(ts) = tags_str {
                let mut tags: Vec<String> = match serde_json::from_str(&ts) {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(
                            "Skipping delete_tag for memory {mid}: corrupted tags JSON: {e}"
                        );
                        continue;
                    }
                };
                let before_len = tags.len();
                tags.retain(|t| t != tag);
                if tags.len() != before_len {
                    let new_tags_json = serde_json::to_string(&tags)
                        .map_err(|e| Error::db("database operation", e))?;
                    let now = chrono::Utc::now().to_rfc3339();
                    self.conn
                        .execute(
                            "UPDATE memories SET tags = ?1, updated_at = ?2 WHERE id = ?3",
                            params![new_tags_json, now, mid],
                        )
                        .map_err(|e| Error::db("database operation", e))?;
                    updated += 1;
                }
            }
        }

        tx.commit()
            .map_err(|e| Error::db("commit transaction", e))?;
        Ok(updated)
    }

    /// Count memories by tag in a namespace.
    pub fn count_by_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let count: i64 = match namespace {
            Some(ns) => self
                .conn
                .query_row(
                    "SELECT COUNT(DISTINCT mt.memory_id) \
                     FROM memory_tags mt \
                     INNER JOIN memories m ON mt.memory_id = m.id \
                     WHERE mt.tag = ?1 AND m.namespace = ?2",
                    params![tag, ns],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| Error::db("database operation", e))?,
            None => self
                .conn
                .query_row(
                    "SELECT COUNT(DISTINCT memory_id) FROM memory_tags WHERE tag = ?1",
                    params![tag],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| Error::db("database operation", e))?,
        };
        Ok(count as usize)
    }

    /// List all distinct namespaces.
    pub fn list_namespaces(&self) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT namespace FROM memories ORDER BY namespace")
            .map_err(|e| Error::db("database operation", e))?;

        let rows = stmt
            .query_map([], |row: &rusqlite::Row| row.get(0))
            .map_err(|e| Error::db("database operation", e))?;

        let mut namespaces = Vec::new();
        for row in rows {
            namespaces.push(row.map_err(|e| Error::db("database operation", e))?);
        }
        Ok(namespaces)
    }

    /// List namespaces with memory counts.
    ///
    /// Returns `[(namespace, count)]` — e.g. `[("default", 432), ("cto", 28)]`.
    /// Used by `/namespaces?with_counts=true` endpoint (#527).
    pub fn list_namespaces_with_counts(&self) -> Result<Vec<(String, usize)>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT namespace, COUNT(*) as cnt \
                 FROM memories \
                 GROUP BY namespace \
                 ORDER BY namespace",
            )
            .map_err(|e| Error::db("database operation", e))?;

        let rows = stmt
            .query_map([], |row: &rusqlite::Row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)? as usize))
            })
            .map_err(|e| Error::db("database operation", e))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| Error::db("database operation", e))?);
        }
        Ok(result)
    }

    /// List namespaces with counts split by lifecycle state (#1181).
    ///
    /// Returns `[(namespace, active, deprecated)]`. Deprecated-only "ghost"
    /// namespaces currently look identical to active ones in listings;
    /// splitting the counts lets clients show honest breakdowns.
    pub fn list_namespaces_with_lifecycle_counts(
        &self,
    ) -> Result<Vec<(String, usize, usize)>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT namespace, \
                        SUM(CASE WHEN deprecated = 0 THEN 1 ELSE 0 END), \
                        SUM(CASE WHEN deprecated = 1 THEN 1 ELSE 0 END) \
                 FROM memories \
                 GROUP BY namespace \
                 ORDER BY namespace",
            )
            .map_err(|e| Error::db("database operation", e))?;

        let rows = stmt
            .query_map([], |row: &rusqlite::Row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?.max(0) as usize,
                    row.get::<_, i64>(2)?.max(0) as usize,
                ))
            })
            .map_err(|e| Error::db("database operation", e))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| Error::db("database operation", e))?);
        }
        Ok(result)
    }

    /// Count a namespace's memories split by lifecycle state (#1181).
    ///
    /// Returns `(active, deprecated, total)` — `total` includes deprecated.
    pub fn namespace_lifecycle_counts(&self, name: &str) -> Result<(usize, usize, usize), Error> {
        self.conn
            .query_row(
                "SELECT \
                    COALESCE(SUM(CASE WHEN deprecated = 0 THEN 1 ELSE 0 END), 0), \
                    COALESCE(SUM(CASE WHEN deprecated = 1 THEN 1 ELSE 0 END), 0), \
                    COUNT(*) \
                 FROM memories WHERE namespace = ?1",
                params![name],
                |row: &rusqlite::Row| {
                    Ok((
                        row.get::<_, i64>(0)?.max(0) as usize,
                        row.get::<_, i64>(1)?.max(0) as usize,
                        row.get::<_, i64>(2)?.max(0) as usize,
                    ))
                },
            )
            .map_err(|e| Error::db("namespace lifecycle counts", e))
    }

    /// Move a single memory to another namespace (#1181).
    ///
    /// Returns `false` when the memory does not exist. Embeddings are
    /// content-based, so no re-embed is needed — namespace is a plain column.
    pub fn move_memory_namespace(
        &self,
        id: &str,
        namespace: &str,
        now: &str,
    ) -> Result<bool, Error> {
        let rows = self
            .conn
            .execute(
                "UPDATE memories SET namespace = ?1, updated_at = ?2 WHERE id = ?3",
                params![namespace, now, id],
            )
            .map_err(|e| Error::db("move memory namespace", e))?;
        Ok(rows > 0)
    }

    /// Atomically rename a namespace (merge when `to` already exists) (#1181).
    ///
    /// Returns the number of memories moved. The old name vanishes naturally:
    /// namespaces are a derived view over the `memories.namespace` column.
    pub fn rename_namespace(&self, from: &str, to: &str, now: &str) -> Result<usize, Error> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin namespace rename", e))?;
        let rows = tx
            .execute(
                "UPDATE memories SET namespace = ?1, updated_at = ?2 WHERE namespace = ?3",
                params![to, now, from],
            )
            .map_err(|e| Error::db("rename namespace", e))?;
        tx.commit()
            .map_err(|e| Error::db("commit namespace rename", e))?;
        Ok(rows)
    }

    /// All memory IDs in a namespace, including deprecated (#1181).
    pub fn namespace_ids(&self, name: &str) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM memories WHERE namespace = ?1")
            .map_err(|e| Error::db("prepare namespace ids", e))?;
        let rows = stmt
            .query_map(params![name], |row: &rusqlite::Row| row.get::<_, String>(0))
            .map_err(|e| Error::db("namespace ids", e))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| Error::db("namespace ids", e))?);
        }
        Ok(ids)
    }

    /// Soft-delete (deprecate) every active memory in a namespace (#1181).
    ///
    /// Never hard-deletes — deprecated memories stay restorable via
    /// `promote()`, consistent with the lifecycle philosophy (#929).
    /// Returns the number of newly deprecated rows.
    pub fn deprecate_by_namespace(&self, name: &str, reason: &str) -> Result<usize, Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let rows = self
            .conn
            .execute(
                "UPDATE memories SET deprecated = 1, valid_until = ?1, deprecate_reason = ?2, \
                 updated_at = ?1, deprecated_at = ?1 \
                 WHERE namespace = ?3 AND deprecated = 0",
                params![now, reason, name],
            )
            .map_err(|e| Error::db("deprecate namespace", e))?;
        Ok(rows)
    }
}
