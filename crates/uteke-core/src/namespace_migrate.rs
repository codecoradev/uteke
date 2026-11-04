//! `uteke namespace migrate --map <file>` (#1427 phase 3, task I).
//!
//! Moves the memories of each mapped namespace into `default`, carrying the
//! namespace's meaning over to a room, a tag, an author and/or a source, or
//! soft-deletes them (`drop`). A dry run reports and writes nothing; the real
//! run writes a full export first, so the export is the way back.
//!
//! Safety rules (docs/plans/2026-10-29-namespace-deprecation-next-1427.md):
//! - `default` can never be named in the map, so it is never touched.
//! - Only live rows move. Soft-deleted rows are not in an export, so they
//!   stay where they are and are only counted.
//! - An existing, different `author` or `source` is kept and counted as a
//!   conflict; the map never overwrites a value a memory already has.
//! - `drop` soft-deletes (restorable with promote); it needs `allow_drop`.
//! - One transaction per namespace: a failure leaves earlier namespaces
//!   migrated and later ones untouched. Re-running the same map is a no-op.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::index_sync::{self, SyncPolicy};
use crate::memory::types::{DEFAULT_NAMESPACE, validate_author};

/// Where one namespace's memories go. Targets compose, except `drop`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drop: bool,
}

/// The map file: `[map."<namespace>"]` tables, one per namespace.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceMigrationMap {
    #[serde(default)]
    pub map: BTreeMap<String, NamespaceTarget>,
}

impl NamespaceMigrationMap {
    /// Parse and validate a TOML map file.
    pub fn parse(input: &str) -> Result<Self, Error> {
        let mut parsed: Self = toml::from_str(input)
            .map_err(|e| Error::Validation(format!("invalid namespace map: {e}")))?;
        for (ns, target) in &mut parsed.map {
            validate_entry(ns, target)?;
        }
        Ok(parsed)
    }
}

fn validate_entry(ns: &str, target: &mut NamespaceTarget) -> Result<(), Error> {
    let bad = |msg: String| Error::Validation(format!("namespace map [map.\"{ns}\"]: {msg}"));
    if ns.trim().is_empty() || ns.trim().len() > 128 {
        return Err(bad("namespace name must be 1-128 characters".into()));
    }
    if ns == DEFAULT_NAMESPACE {
        return Err(bad(
            "'default' is the migration target and cannot be mapped".into(),
        ));
    }
    let has_target = target.room.is_some()
        || target.tag.is_some()
        || target.author.is_some()
        || target.source.is_some();
    if target.drop && has_target {
        return Err(bad("drop cannot be combined with another target".into()));
    }
    if !target.drop && !has_target {
        return Err(bad(
            "needs a target: room, tag, author, source or drop".into()
        ));
    }
    for (field, value) in [
        ("room", &mut target.room),
        ("tag", &mut target.tag),
        ("source", &mut target.source),
    ] {
        if let Some(v) = value {
            let t = v.trim();
            if t.is_empty() || t.chars().any(char::is_control) {
                return Err(bad(format!("{field} must be non-empty text")));
            }
            if field == "tag" && t.len() > crate::MAX_TAG_LENGTH {
                return Err(bad(format!(
                    "tag is longer than {} characters",
                    crate::MAX_TAG_LENGTH
                )));
            }
            *v = t.to_string();
        }
    }
    if let Some(a) = &mut target.author {
        *a = validate_author(a)
            .map_err(|e| bad(e.to_string()))?
            .to_string();
    }
    Ok(())
}

/// What happens (dry run) or happened (apply) to one mapped namespace.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NamespaceMigration {
    pub namespace: String,
    pub target: NamespaceTarget,
    /// False when the store has no live memory in it (unknown, or already
    /// migrated): nothing is written for it.
    pub found: bool,
    /// Live memories that move (or are dropped).
    pub memories: usize,
    /// Soft-deleted memories, left in the namespace.
    pub soft_deleted: usize,
    /// The target room does not exist yet and is (or will be) created.
    pub room_created: bool,
    /// Memories whose own, different author is kept.
    pub author_conflicts: usize,
    /// Memories whose own, different source is kept.
    pub source_conflicts: usize,
}

/// A namespace with live memories that the map does not name. Left alone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnmappedNamespace {
    pub namespace: String,
    pub memories: usize,
}

/// The report of a dry run or an applied migration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NamespaceMigrationReport {
    pub applied: bool,
    /// The export written before the first change; `None` for a dry run or
    /// when there was nothing to change.
    pub export_path: Option<String>,
    pub namespaces: Vec<NamespaceMigration>,
    pub unmapped: Vec<UnmappedNamespace>,
}

impl crate::Uteke {
    /// Dry run: report what [`Self::apply_namespace_migration`] would do.
    /// Writes nothing.
    pub fn plan_namespace_migration(
        &self,
        map: &NamespaceMigrationMap,
    ) -> Result<NamespaceMigrationReport, Error> {
        let mut namespaces = Vec::with_capacity(map.map.len());
        for (ns, target) in &map.map {
            namespaces.push(self.plan_one(ns, target)?);
        }
        let unmapped = self
            .store
            .list_namespaces_with_lifecycle_counts()?
            .into_iter()
            .filter(|(ns, active, _)| {
                *active > 0 && ns != DEFAULT_NAMESPACE && !map.map.contains_key(ns)
            })
            .map(|(namespace, memories, _)| UnmappedNamespace {
                namespace,
                memories,
            })
            .collect();
        Ok(NamespaceMigrationReport {
            applied: false,
            export_path: None,
            namespaces,
            unmapped,
        })
    }

    /// Apply the map. Writes a full export to `export_path` first (refuses
    /// when the file exists), then migrates one namespace per transaction.
    /// A map with a `drop` is refused unless `allow_drop`. When nothing would
    /// change, nothing is written, not even the export.
    pub fn apply_namespace_migration(
        &self,
        map: &NamespaceMigrationMap,
        export_path: &Path,
        allow_drop: bool,
    ) -> Result<NamespaceMigrationReport, Error> {
        let mut report = self.plan_namespace_migration(map)?;
        report.applied = true;
        let dropping: Vec<&str> = report
            .namespaces
            .iter()
            .filter(|e| e.target.drop && e.memories > 0)
            .map(|e| e.namespace.as_str())
            .collect();
        if !dropping.is_empty() && !allow_drop {
            return Err(Error::Validation(format!(
                "the map drops {}; pass --allow-drop to soft-delete them",
                dropping.join(", ")
            )));
        }
        if report.namespaces.iter().all(|e| e.memories == 0) {
            return Ok(report);
        }

        self.write_export(export_path)?;
        report.export_path = Some(export_path.to_string_lossy().into_owned());

        for entry in report.namespaces.iter().filter(|e| e.memories > 0) {
            self.migrate_one(&entry.namespace, &entry.target)?;
            tracing::info!(
                "namespace migrate: '{}' -> {:?} ({} memories)",
                entry.namespace,
                entry.target,
                entry.memories
            );
        }
        Ok(report)
    }

    fn plan_one(&self, ns: &str, target: &NamespaceTarget) -> Result<NamespaceMigration, Error> {
        let (active, soft_deleted, _) = self.store.namespace_lifecycle_counts(ns)?;
        let conn = &self.store.conn;
        let conflicts = |column: &str, value: &Option<String>| -> Result<usize, Error> {
            let Some(v) = value else { return Ok(0) };
            let sql = format!(
                "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND deprecated = 0 \
                 AND {column} IS NOT NULL AND {column} <> ?2"
            );
            conn.query_row(&sql, params![ns, v], |row| row.get::<_, i64>(0))
                .map(|n| n.max(0) as usize)
                .map_err(|e| Error::db("namespace migrate conflicts", e))
        };
        let room_created = match &target.room {
            Some(room) if active > 0 => self.store.get_room(room)?.is_none(),
            _ => false,
        };
        Ok(NamespaceMigration {
            namespace: ns.to_string(),
            target: target.clone(),
            found: active > 0,
            memories: active,
            soft_deleted,
            room_created,
            author_conflicts: conflicts("author", &target.author)?,
            source_conflicts: conflicts("source", &target.source)?,
        })
    }

    /// Write the full structural export to a new file.
    fn write_export(&self, path: &Path) -> Result<(), Error> {
        let export = self.export_full()?;
        // Owner-only from creation: the file holds every memory in the store (#1668).
        let mut file = crate::private_file::create_private(path, false).map_err(|e| {
            Error::Validation(format!(
                "cannot write the export {} (it must not exist yet): {e}",
                path.display()
            ))
        })?;
        file.write_all(export.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(Error::Io)
    }

    fn migrate_one(&self, ns: &str, target: &NamespaceTarget) -> Result<(), Error> {
        if target.drop {
            let reason = format!("namespace migrate: drop of '{ns}' (#1427)");
            let ids = self.store.namespace_ids(ns)?;
            self.store.deprecate_by_namespace(ns, &reason)?;
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during namespace migrate"))?;
            index_sync::remove_ids(
                &mut *index,
                ids.iter().map(String::as_str),
                &SyncPolicy::STANDARD,
            )
            .into_delete_result("namespace migrate (drop)")?;
            self.recall_cache.invalidate_namespace(ns);
            return Ok(());
        }

        let conn = &self.store.conn;
        let now = chrono::Utc::now().to_rfc3339();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin namespace migrate", e))?;
        let ids: Vec<(String, Option<String>)> = {
            let mut stmt = tx
                .prepare("SELECT id, tags FROM memories WHERE namespace = ?1 AND deprecated = 0")
                .map_err(|e| Error::db("namespace migrate rows", e))?;
            stmt.query_map(params![ns], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| Error::db("namespace migrate rows", e))?
                .collect::<Result<_, _>>()
                .map_err(|e| Error::db("namespace migrate rows", e))?
        };

        if let Some(room) = &target.room
            && self.store.get_room(room)?.is_none()
        {
            self.store.create_room(room, None, DEFAULT_NAMESPACE)?;
        }

        for (id, tags_json) in &ids {
            if let Some(tag) = &target.tag {
                add_tag(&tx, id, tags_json.as_deref(), tag)?;
            }
            for (column, value) in [("author", &target.author), ("source", &target.source)] {
                if let Some(v) = value {
                    tx.execute(
                        &format!(
                            "UPDATE memories SET {column} = ?1 WHERE id = ?2 AND {column} IS NULL"
                        ),
                        params![v, id],
                    )
                    .map_err(|e| Error::db("namespace migrate attribution", e))?;
                }
            }
            if let Some(room) = &target.room {
                let author: Option<String> = tx
                    .query_row(
                        "SELECT author FROM memories WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .map_err(|e| Error::db("namespace migrate room author", e))?;
                self.store.link_memory_to_room(
                    room,
                    id,
                    author.as_deref().unwrap_or("unknown"),
                    "participant",
                )?;
            }
            tx.execute(
                "UPDATE memories SET namespace = ?1, updated_at = ?2 WHERE id = ?3",
                params![DEFAULT_NAMESPACE, now, id],
            )
            .map_err(|e| Error::db("namespace migrate move", e))?;
        }
        tx.commit()
            .map_err(|e| Error::db("commit namespace migrate", e))?;
        self.recall_cache.invalidate_namespace(ns);
        self.recall_cache.invalidate_namespace(DEFAULT_NAMESPACE);
        Ok(())
    }
}

/// Add `tag` to the JSON `tags` column and the `memory_tags` index, once.
fn add_tag(
    conn: &rusqlite::Connection,
    id: &str,
    tags_json: Option<&str>,
    tag: &str,
) -> Result<(), Error> {
    let mut tags: Vec<String> = match tags_json {
        Some(s) if !s.trim().is_empty() => serde_json::from_str(s)
            .map_err(|e| Error::db_msg(format!("memory {id} has corrupted tags JSON: {e}")))?,
        _ => Vec::new(),
    };
    if !tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
        tags.push(tag.to_string());
        let json = serde_json::to_string(&tags).map_err(|e| Error::db("tags json", e))?;
        conn.execute(
            "UPDATE memories SET tags = ?1 WHERE id = ?2",
            params![json, id],
        )
        .map_err(|e| Error::db("namespace migrate tag", e))?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO memory_tags (memory_id, tag) VALUES (?1, ?2)",
        params![id, tag],
    )
    .map_err(|e| Error::db("namespace migrate tag index", e))?;
    Ok(())
}
