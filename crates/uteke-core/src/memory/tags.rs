//! Tag operations — unique tags, counts, rename, delete, namespaces.

use crate::memory::types::{TagInfo, DEFAULT_NAMESPACE};
use crate::Error;

impl super::Store {
    /// Get all unique tags, optionally filtered by namespace.
    ///
    /// Uses `json_each()` to unnest the JSON array stored in `tags` so SQLite
    /// returns individual tag values directly — no in-Rust JSON parsing needed.
    pub fn unique_tags(&self, namespace: Option<&str>) -> Result<Vec<String>, Error> {
        let sql = match namespace {
            Some(_) => {
                "SELECT DISTINCT je.value FROM memories, json_each(memories.tags) AS je WHERE namespace = ?1"
            }
            None => "SELECT DISTINCT je.value FROM memories, json_each(memories.tags) AS je",
        };

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("database operation", e))?;

        let rows = match namespace {
            Some(ns) => stmt
                .query_map(rusqlite::params![ns], |row: &rusqlite::Row| {
                    row.get::<_, String>(0)
                })
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
    /// Single-query approach using `json_each()` — replaces the old N+1 pattern
    /// that fetched each tag then ran a separate COUNT query per tag.
    pub fn tags_with_counts(&self, namespace: Option<&str>) -> Result<Vec<TagInfo>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let sql = "SELECT je.value AS name, COUNT(*) AS count FROM memories, json_each(memories.tags) AS je WHERE namespace = ?1 GROUP BY je.value ORDER BY count DESC";
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("database operation", e))?;
        let rows = stmt
            .query_map(rusqlite::params![ns], |row| {
                Ok(TagInfo {
                    name: row.get(0)?,
                    count: row.get(1)?,
                })
            })
            .map_err(|e| Error::db("database operation", e))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| Error::db("database operation", e))?);
        }
        Ok(result)
    }

    /// Rename a tag across all memories. Returns number updated.
    ///
    /// Uses `json_each()` to find affected rows precisely, then updates the
    /// JSON tags column with the renamed tag.
    pub fn rename_tag(
        &self,
        old: &str,
        new: &str,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare("SELECT id, tags FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2)")
            .map_err(|e| Error::db("database operation", e))?;

        let rows: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![ns, old], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| r.ok())
            .collect();

        let mut updated = 0;
        for (id, tags_str) in &rows {
            let mut tags: Vec<String> = serde_json::from_str(tags_str).unwrap_or_default();
            let mut changed = false;
            for t in &mut tags {
                if t == old {
                    *t = new.to_string();
                    changed = true;
                }
            }
            if changed {
                let new_tags_json =
                    serde_json::to_string(&tags).map_err(|e| Error::db("database operation", e))?;
                let now = chrono::Utc::now().to_rfc3339();
                self.conn
                    .execute(
                        "UPDATE memories SET tags = ?1, updated_at = ?2 WHERE id = ?3",
                        rusqlite::params![new_tags_json, now, id],
                    )
                    .map_err(|e| Error::db("database operation", e))?;
                updated += 1;
            }
        }
        Ok(updated)
    }

    /// Delete a tag from all memories. Returns number updated.
    ///
    /// Uses `json_each()` to find affected rows precisely.
    pub fn delete_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let mut stmt = self
            .conn
            .prepare("SELECT id, tags FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2)")
            .map_err(|e| Error::db("database operation", e))?;

        let rows: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![ns, tag], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| Error::db("database operation", e))?
            .filter_map(|r| r.ok())
            .collect();

        let mut updated = 0;
        for (id, tags_str) in &rows {
            let mut tags: Vec<String> = serde_json::from_str(tags_str).unwrap_or_default();
            let before_len = tags.len();
            tags.retain(|t| t != tag);
            if tags.len() != before_len {
                let new_tags_json =
                    serde_json::to_string(&tags).map_err(|e| Error::db("database operation", e))?;
                let now = chrono::Utc::now().to_rfc3339();
                self.conn
                    .execute(
                        "UPDATE memories SET tags = ?1, updated_at = ?2 WHERE id = ?3",
                        rusqlite::params![new_tags_json, now, id],
                    )
                    .map_err(|e| Error::db("database operation", e))?;
                updated += 1;
            }
        }
        Ok(updated)
    }

    /// Count memories by tag in a namespace.
    pub fn count_by_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND EXISTS (SELECT 1 FROM json_each(memories.tags) WHERE value = ?2)",
                rusqlite::params![ns, tag],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("database operation", e))
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
}
