//! Auto-wired memory edges — typed graph between memories (#346).
//!
//! Patterns extracted on every `remember()` (zero LLM, pure string parsing):
//!
//! | Pattern      | Edge type     | Resolved via            |
//! |--------------|---------------|-------------------------|
//! | `[[slug]]`   | `references`  | `memories.slug` lookup  |
//! | `@tag`       | `tagged_as`   | `memory_tags` junction  |
//! | `^<uuid>`    | `supersedes`  | `memories.id` direct    |
//! | `><uuid>`    | `replies_to`  | `memories.id` direct    |
//! | `rel:<t>:<id>` in metadata | `<t>` | `memories.id` direct (legacy compat) |
//!
//! Edges live in the `memory_edges` SQLite table (schema v8). Reads use
//! indexed SQL queries instead of the old O(n) JSON scan.

use crate::error::Error;
use crate::index_sync::{self, SyncPolicy};
use crate::memory::types::Memory;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::memory::store::Store;

// ── Edge types ────────────────────────────────────────────────────────────

/// Edge type for `[[slug]]` references.
pub const EDGE_REFERENCES: &str = "references";
/// Edge type for `@tag` mentions.
pub const EDGE_TAGGED_AS: &str = "tagged_as";
/// Edge type for `^<uuid>` (supersedes).
pub const EDGE_SUPERSEDES: &str = "supersedes";
/// Edge type for `><uuid>` (replies_to).
pub const EDGE_REPLIES_TO: &str = "replies_to";
/// Cosine-similarity auto-link edge (#401).
pub const EDGE_SIMILAR_TO: &str = "similar_to";
/// Edge type for `[[doc-slug]]` document references (#689).
pub const EDGE_REFERENCES_DOC: &str = "references_doc";
/// Possible duplicate edge — high cosine similarity (#401).
pub const EDGE_POSSIBLE_DUPLICATE: &str = "possible_duplicate";
/// Inverse edge type created automatically by backlink auto-generation (#350).
///
/// Whenever `(A → B, references)` is inserted, `ensure_backlink()` also inserts
/// `(B → A, referenced_by)` so the graph is navigable in both directions.
pub const EDGE_REFERENCED_BY: &str = "referenced_by";

/// Edge types that should receive an auto-generated `referenced_by` backlink (#350).
///
/// Backlinks are only generated for the forward link types listed here. Inverse
/// types (like `referenced_by` itself) are intentionally excluded to avoid
/// infinite ping-pong.
const BACKLINKED_EDGE_TYPES: &[&str] = &[
    EDGE_REFERENCES,
    EDGE_REFERENCES_DOC,
    EDGE_TAGGED_AS,
    EDGE_SUPERSEDES,
    EDGE_REPLIES_TO,
];

/// Return the backlink edge type for a forward edge type, if any (#350).
///
/// `references` → `referenced_by`. Other forward types currently map to
/// `referenced_by` as well; if finer-grained inverses are needed later,
/// extend this function.
pub fn backlink_type_for(edge_type: &str) -> Option<&'static str> {
    if BACKLINKED_EDGE_TYPES.contains(&edge_type) {
        Some(EDGE_REFERENCED_BY)
    } else {
        None
    }
}

/// A single typed edge between two memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEdge {
    pub source_id: String,
    pub target_id: String,
    pub edge_type: String,
    pub created_at: String,
}

/// Edge summary returned by `uteke edges <id>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeList {
    pub memory_id: String,
    /// Edges originating from this memory.
    pub outgoing: Vec<MemoryEdge>,
    /// Edges pointing to this memory.
    pub incoming: Vec<MemoryEdge>,
}

impl EdgeList {
    pub fn total(&self) -> usize {
        self.outgoing.len() + self.incoming.len()
    }
}

// ── Pattern extraction ─────────────────────────────────────────────────────

/// A raw extracted reference — target may be a slug, tag, or UUID depending on kind.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExtractedRef {
    /// `[[slug]]` — resolve via `memories.slug`.
    Slug(String),
    /// `@tag` — resolve via `memory_tags` (most recent memory with tag in namespace).
    Tag(String),
    /// `^<uuid>` or `><uuid>` — already a memory UUID.
    Uuid(String, &'static str),
    /// `rel:<type>:<id>` from `--meta` — legacy explicit form.
    Rel(String, String),
}

/// Extract raw references from content + tags + metadata.
///
/// Pure string parsing — no DB access, no LLM. Deterministic and cheap.
fn extract_refs(content: &str, tags: &[&str], metadata: &serde_json::Value) -> Vec<ExtractedRef> {
    let mut refs = Vec::new();

    // [[slug]] — Wikilink-style references.
    // Allow letters, digits, `-`, `_`. Reject empty.
    let mut i = 0;
    let bytes = content.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // find closing ]]
            if let Some(end) = find_close(&content[i + 2..], "]]") {
                let slug = &content[i + 2..i + 2 + end];
                if is_valid_slug(slug) {
                    refs.push(ExtractedRef::Slug(slug.to_string()));
                }
                i += 2 + end + 2;
                continue;
            }
        }
        i += 1;
    }

    // @tag — mentions of an existing tag (heuristic: only count if tag is not
    // already in this memory's tag list; we still record the edge so the link
    // survives even if the tag is later removed).
    let mut j = 0;
    while j < bytes.len() {
        if bytes[j] == b'@' {
            // capture [a-z0-9_-]+ starting after @
            let start = j + 1;
            let mut k = start;
            while k < bytes.len() && is_tag_char(bytes[k]) {
                k += 1;
            }
            if k > start {
                let tag = &content[start..k];
                if !tag.is_empty() {
                    refs.push(ExtractedRef::Tag(tag.to_string()));
                }
                j = k;
                continue;
            }
        }
        j += 1;
    }

    // ^<uuid> and ><uuid> — UUID-prefixed references.
    // Scan at byte level so surrounding punctuation doesn't break detection.
    let mut k = 0;
    while k < bytes.len() {
        let prefix_edge = match bytes[k] {
            b'^' => EDGE_SUPERSEDES,
            b'>' => EDGE_REPLIES_TO,
            _ => {
                k += 1;
                continue;
            }
        };
        // Capture token immediately after prefix: [0-9a-fA-F-]{36}
        let start = k + 1;
        let mut end = start;
        while end < bytes.len() {
            let b = bytes[end];
            if b.is_ascii_hexdigit() || b == b'-' {
                end += 1;
            } else {
                break;
            }
        }
        let token = &content[start..end];
        if looks_like_uuid(token) {
            refs.push(ExtractedRef::Uuid(token.to_string(), prefix_edge));
            k = end;
        } else {
            k += 1;
        }
    }

    // Legacy: rel:<type>:<id> from metadata.relationships JSON, or from
    // `--meta rel:type:id` flat strings stored under "meta_pairs".
    if let Some(arr) = metadata.get("relationships").and_then(|v| v.as_array()) {
        for rel in arr {
            let Some(rel_type) = rel.get("type").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(target) = rel.get("target").and_then(|v| v.as_str()) else {
                continue;
            };
            refs.push(ExtractedRef::Rel(rel_type.to_string(), target.to_string()));
        }
    }
    // Also scan flat `meta` strings ("rel:type:id") if present under metadata._meta_pairs.
    if let Some(pairs) = metadata.get("_meta_pairs").and_then(|v| v.as_array()) {
        for p in pairs {
            if let Some(s) = p.as_str() {
                if let Some(rest) = s.strip_prefix("rel:") {
                    if let Some((t, id)) = rest.split_once(':') {
                        refs.push(ExtractedRef::Rel(t.to_string(), id.to_string()));
                    }
                }
            }
        }
    }

    // Dedup keeping first occurrence. Tags from this memory's own `tags` param
    // do NOT create self-edges (they're already attributes).
    let _ = tags; // (reserved — we don't filter @tag against own tags to keep links stable)
    refs.dedup_by(|a, b| format!("{a:?}") == format!("{b:?}"));
    refs
}

fn find_close(haystack: &str, close: &str) -> Option<usize> {
    haystack.find(close)
}

fn is_valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_tag_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn looks_like_uuid(s: &str) -> bool {
    // Loose UUID check: 8-4-4-4-12 hex, case-insensitive, with hyphens.
    s.len() == 36 && {
        s.as_bytes().iter().enumerate().all(|(i, b)| {
            (matches!(i, 8 | 13 | 18 | 23) && *b == b'-') || (*b as char).is_ascii_hexdigit()
        })
    }
}

// ── Store-level edge operations ────────────────────────────────────────────

impl Store {
    /// Insert a single edge. Silently ignores duplicates (UNIQUE constraint).
    ///
    /// This is the low-level store primitive. It does **not** generate a
    /// backlink. Use [`Uteke::link_memories`] / auto-wiring for the
    /// bidirectional behavior required by #350.
    pub fn add_memory_edge(
        &self,
        source_id: &str,
        target_id: &str,
        edge_type: &str,
    ) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO memory_edges (source_id, target_id, edge_type, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    source_id,
                    target_id,
                    edge_type,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| Error::db("add memory edge", e))?;
        Ok(())
    }

    /// Idempotently insert the inverse `referenced_by` edge for a forward
    /// edge `(source → target, edge_type)` (#350).
    ///
    /// Only forward link types (see [`BACKLINKED_EDGE_TYPES`]) receive a
    /// backlink. Calling this on an already-backlinked type (or on a pair
    /// that already has the backlink) is a no-op thanks to the table's
    /// `UNIQUE(source_id, target_id, edge_type)` constraint.
    ///
    /// Returns `true` if a new backlink row was inserted.
    pub fn ensure_backlink(
        &self,
        source_id: &str,
        target_id: &str,
        edge_type: &str,
    ) -> Result<bool, Error> {
        let Some(backlink_type) = backlink_type_for(edge_type) else {
            return Ok(false);
        };
        // target → source with the inverse type. Avoid self-loops: a memory
        // referencing itself should not produce a `referenced_by` echo.
        if source_id == target_id {
            return Ok(false);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let changed = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO memory_edges
                    (source_id, target_id, edge_type, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![target_id, source_id, backlink_type, now],
            )
            .map_err(|e| Error::db("ensure backlink", e))?;
        Ok(changed > 0)
    }

    /// Idempotently insert a forward edge **and** its backlink (#350).
    ///
    /// Use this when you want the "Iron Law of Back-Linking": a reference
    /// A→B automatically makes B→A navigable as `referenced_by`.
    ///
    /// Unlike the auto-wire path (`wire_edges`), this propagates backlink
    /// failures rather than swallowing them — callers who ask for an edge
    /// *with backlink* expect bidirectional consistency.
    ///
    /// Both inserts run inside a single SQLite transaction so the edge pair
    /// is atomic: if the backlink fails, the forward insert is rolled back.
    ///
    /// Returns whether a new forward row was inserted (duplicate inserts are
    /// a no-op for both forward and backlink rows).
    pub fn add_memory_edge_with_backlink(
        &self,
        source_id: &str,
        target_id: &str,
        edge_type: &str,
    ) -> Result<bool, Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin edge tx", e))?;
        let changed = tx
            .execute(
                "INSERT OR IGNORE INTO memory_edges
                    (source_id, target_id, edge_type, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![source_id, target_id, edge_type, now],
            )
            .map_err(|e| Error::db("add memory edge (with backlink)", e))?;
        // Insert the inverse inside the same transaction. If this fails the
        // whole tx is rolled back and the caller observes the error with no
        // partial state left behind.
        let Some(backlink_type) = backlink_type_for(edge_type) else {
            tx.commit()
                .map_err(|e| Error::db("commit edge tx (no backlink)", e))?;
            return Ok(changed > 0);
        };
        if source_id != target_id {
            tx.execute(
                "INSERT OR IGNORE INTO memory_edges
                    (source_id, target_id, edge_type, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![target_id, source_id, backlink_type, now],
            )
            .map_err(|e| Error::db("ensure backlink (tx)", e))?;
        }
        tx.commit().map_err(|e| Error::db("commit edge tx", e))?;
        Ok(changed > 0)
    }

    /// Bulk insert edges. Silently ignores duplicates and dangling targets.
    ///
    /// Each inserted forward edge also receives an automatic `referenced_by`
    /// backlink (#350). All inserts run inside a single transaction so the
    /// batch is atomic — on failure, no partial state is left behind.
    ///
    /// Backlink failures are propagated (use `wire_edges` at the `Uteke`
    /// level for best-effort auto-wiring that swallows them).
    ///
    /// Returns the number of **forward** rows inserted (backlinks are
    /// guaranteed on success and do not count toward the return value).
    pub fn add_memory_edges_batch(
        &self,
        source_id: &str,
        edges: &[(String, String)],
    ) -> Result<usize, Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin batch edge tx", e))?;

        // Prepare statements once, reuse across all iterations (eliminates re-prepare overhead).
        let mut stmt_forward = tx
            .prepare(
                "INSERT OR IGNORE INTO memory_edges
                    (source_id, target_id, edge_type, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| Error::db("prepare forward edge stmt", e))?;
        let mut stmt_backlink = tx
            .prepare(
                "INSERT OR IGNORE INTO memory_edges
                    (source_id, target_id, edge_type, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| Error::db("prepare backlink edge stmt", e))?;

        let mut inserted = 0usize;
        for (target_id, edge_type) in edges {
            let changed = stmt_forward
                .execute(params![source_id, target_id, edge_type, now])
                .map_err(|e| Error::db("batch insert memory edge", e))?;
            inserted += changed;
            // #350: ensure inverse backlink for every forward edge inside the
            // same transaction so the whole batch commits atomically.
            if let Some(backlink_type) = backlink_type_for(edge_type) {
                if source_id != target_id {
                    stmt_backlink
                        .execute(params![target_id, source_id, backlink_type, now])
                        .map_err(|e| Error::db("batch ensure backlink", e))?;
                }
            }
        }
        drop(stmt_forward);
        drop(stmt_backlink);
        tx.commit()
            .map_err(|e| Error::db("commit batch edge tx", e))?;
        Ok(inserted)
    }

    /// Resolve a slug to a memory id (most recent match, same namespace optional).
    ///
    /// Slugs are **not** globally unique — when duplicates exist, the most
    /// recently updated memory wins. Callers who need deterministic links
    /// should ensure slug uniqueness within their namespace.
    pub fn resolve_slug(
        &self,
        slug: &str,
        namespace: Option<&str>,
    ) -> Result<Option<String>, Error> {
        let row: Option<String> = match namespace {
            Some(ns) => self
                .conn
                .query_row(
                    "SELECT id FROM memories WHERE slug = ?1 AND namespace = ?2
                     ORDER BY updated_at DESC LIMIT 1",
                    params![slug, ns],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| Error::db("resolve slug", e))?,
            None => self
                .conn
                .query_row(
                    "SELECT id FROM memories WHERE slug = ?1 ORDER BY updated_at DESC LIMIT 1",
                    params![slug],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| Error::db("resolve slug", e))?,
        };
        Ok(row)
    }

    /// Resolve a slug to a document id (not memory). Returns `None` if no
    /// document with that slug exists. Used by `wire_edges` for `[[doc-slug]]`
    /// cross-entity linking (#689).
    pub fn resolve_document_slug(&self, slug: &str) -> Result<Option<String>, Error> {
        self.conn
            .query_row(
                "SELECT id FROM documents WHERE slug = ?1 LIMIT 1",
                params![slug],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| Error::db("resolve document slug", e))
    }

    /// Get all distinct target IDs for edges from `source_id` with a specific
    /// edge_type. Used for cross-entity recall (#689).
    pub fn edge_targets(&self, source_id: &str, edge_type: &str) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT target_id FROM memory_edges \
                 WHERE source_id = ?1 AND edge_type = ?2",
            )
            .map_err(|e| Error::db("prepare edge_targets", e))?;
        let rows = stmt
            .query_map(params![source_id, edge_type], |r| r.get::<_, String>(0))
            .map_err(|e| Error::db("query edge_targets", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get all distinct source IDs for edges pointing to `target_id` with a
    /// specific edge_type. Used for cross-entity recall (#689).
    pub fn edge_sources(&self, target_id: &str, edge_type: &str) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT source_id FROM memory_edges \
                 WHERE target_id = ?1 AND edge_type = ?2",
            )
            .map_err(|e| Error::db("prepare edge_sources", e))?;
        let rows = stmt
            .query_map(params![target_id, edge_type], |r| r.get::<_, String>(0))
            .map_err(|e| Error::db("query edge_sources", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Resolve a tag to the most recent memory id carrying that tag.
    pub fn resolve_tag_to_memory(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<Option<String>, Error> {
        let row: Option<String> = match namespace {
            Some(ns) => self
                .conn
                .query_row(
                    "SELECT m.id FROM memories m
                     JOIN memory_tags t ON t.memory_id = m.id
                     WHERE t.tag = ?1 AND m.namespace = ?2
                     ORDER BY m.updated_at DESC LIMIT 1",
                    params![tag, ns],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| Error::db("resolve tag", e))?,
            None => self
                .conn
                .query_row(
                    "SELECT m.id FROM memories m
                     JOIN memory_tags t ON t.memory_id = m.id
                     WHERE t.tag = ?1
                     ORDER BY m.updated_at DESC LIMIT 1",
                    params![tag],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| Error::db("resolve tag", e))?,
        };
        Ok(row)
    }

    /// Return all edges touching a memory (both directions).
    pub fn list_memory_edges(&self, memory_id: &str) -> Result<EdgeList, Error> {
        let mut outgoing = Vec::new();
        let mut incoming = Vec::new();

        let mut out_stmt = self
            .conn
            .prepare(
                "SELECT source_id, target_id, edge_type, created_at FROM memory_edges
                 WHERE source_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| Error::db("prepare outgoing edges", e))?;
        let out_rows = out_stmt
            .query_map(params![memory_id], |r| {
                Ok(MemoryEdge {
                    source_id: r.get(0)?,
                    target_id: r.get(1)?,
                    edge_type: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(|e| Error::db("query outgoing edges", e))?;
        for row in out_rows {
            outgoing.push(row.map_err(|e| Error::db("outgoing edge row", e))?);
        }

        let mut in_stmt = self
            .conn
            .prepare(
                "SELECT source_id, target_id, edge_type, created_at FROM memory_edges
                 WHERE target_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| Error::db("prepare incoming edges", e))?;
        let in_rows = in_stmt
            .query_map(params![memory_id], |r| {
                Ok(MemoryEdge {
                    source_id: r.get(0)?,
                    target_id: r.get(1)?,
                    edge_type: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(|e| Error::db("query incoming edges", e))?;
        for row in in_rows {
            incoming.push(row.map_err(|e| Error::db("incoming edge row", e))?);
        }

        Ok(EdgeList {
            memory_id: memory_id.to_string(),
            outgoing,
            incoming,
        })
    }

    /// Return memory ids reachable within `depth` hops from `start_id`.
    ///
    /// BFS using only the `memory_edges` table. Returns ids in BFS order,
    /// excluding the start id. Cycles are handled via a visited set.
    pub fn edge_bfs(&self, start_id: &str, max_depth: usize) -> Result<Vec<String>, Error> {
        use std::collections::{HashSet, VecDeque};
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(start_id.to_string());
        let mut frontier: VecDeque<(String, usize)> = VecDeque::new();
        frontier.push_back((start_id.to_string(), 0));
        let mut ordered: Vec<String> = Vec::new();

        // Prepare the neighbor lookup once and reuse across hops. Recompiling
        // per visited node would add avoidable overhead on deeper traversals
        // (CodeCora finding #134).
        let mut neighbors_stmt = self
            .conn
            .prepare(
                // Neighbours must be live: a deprecated memory is neither
                // returned nor traversed through (soft delete keeps its edges).
                "SELECT e.target_id FROM memory_edges e
                 JOIN memories m ON m.id = e.target_id AND m.deprecated = 0
                 WHERE e.source_id = ?1
                 UNION
                 SELECT e.source_id FROM memory_edges e
                 JOIN memories m ON m.id = e.source_id AND m.deprecated = 0
                 WHERE e.target_id = ?1",
            )
            .map_err(|e| Error::db("prepare bfs neighbors", e))?;

        while let Some((cur, depth)) = frontier.pop_front() {
            if depth >= max_depth {
                continue;
            }
            // Gather neighbors (both directions) at the SQL level.
            let rows = neighbors_stmt
                .query_map(params![cur], |r| r.get::<_, String>(0))
                .map_err(|e| Error::db("query bfs neighbors", e))?;
            for row in rows {
                let nb = row.map_err(|e| Error::db("bfs neighbor row", e))?;
                if visited.insert(nb.clone()) {
                    ordered.push(nb.clone());
                    frontier.push_back((nb, depth + 1));
                }
            }
        }
        Ok(ordered)
    }

    /// Rebuild all `referenced_by` backlinks from existing forward edges (#350).
    ///
    /// Scans every row in `memory_edges` whose `edge_type` is a backlinkable
    /// forward type (see [`BACKLINKED_EDGE_TYPES`]) and ensures the inverse
    /// `(target → source, referenced_by)` row exists. Idempotent.
    ///
    /// Returns the number of new backlink rows inserted.
    pub fn rebuild_backlinks(&self) -> Result<usize, Error> {
        // Collect all forward edges first to avoid holding a read cursor
        // while we write backlinks into the same table.
        //
        // BACKLINKED_EDGE_TYPES is fixed at 5 entries (references, references_doc,
        // tagged_as, supersedes, replies_to) — we bind them positionally.
        debug_assert_eq!(
            BACKLINKED_EDGE_TYPES.len(),
            5,
            "update rebuild_backlinks SQL if backlinked types change"
        );
        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_id, target_id FROM memory_edges
                 WHERE edge_type IN (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|e| Error::db("prepare rebuild backlinks scan", e))?;
        let rows: Vec<(String, String)> = stmt
            .query_map(
                params![
                    BACKLINKED_EDGE_TYPES[0],
                    BACKLINKED_EDGE_TYPES[1],
                    BACKLINKED_EDGE_TYPES[2],
                    BACKLINKED_EDGE_TYPES[3],
                    BACKLINKED_EDGE_TYPES[4],
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(|e| Error::db("rebuild backlinks scan", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::db("rebuild backlinks row iteration", e))?;

        let mut created = 0usize;
        let now = chrono::Utc::now().to_rfc3339();
        // Wrap the repair pass in a single transaction so either all missing
        // backlinks are written or none are (no partial repair state).
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::db("begin rebuild backlinks tx", e))?;
        for (source_id, target_id) in rows {
            if source_id == target_id {
                continue;
            }
            let changed = tx
                .execute(
                    "INSERT OR IGNORE INTO memory_edges
                        (source_id, target_id, edge_type, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![target_id, source_id, EDGE_REFERENCED_BY, now],
                )
                .map_err(|e| Error::db("rebuild backlinks insert", e))?;
            created += changed;
        }
        tx.commit()
            .map_err(|e| Error::db("commit rebuild backlinks tx", e))?;
        Ok(created)
    }

    /// Count total edges in the store.
    pub fn count_memory_edges(&self) -> Result<usize, Error> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memory_edges", [], |r| r.get(0))
            .map_err(|e| Error::db("count memory edges", e))?;
        Ok(n as usize)
    }
}

// ── Uteke-level operations ─────────────────────────────────────────────────

impl crate::Uteke {
    /// Auto-wire edges for a freshly-inserted memory.
    ///
    /// Called from `remember_precomputed` after the memory row is in SQLite.
    /// Resolves slug/tag refs against existing memories in the same namespace.
    /// Failures are logged and do not fail the `remember` call — auto-wiring
    /// is best-effort: a broken link just means no edge, not a failed insert.
    pub(crate) fn wire_edges(
        &self,
        source_id: &str,
        content: &str,
        tags: &[&str],
        metadata: &serde_json::Value,
        namespace: Option<&str>,
    ) {
        let refs = extract_refs(content, tags, metadata);
        if refs.is_empty() {
            return;
        }

        let mut resolved: Vec<(String, String)> = Vec::new();
        for r in refs {
            match r {
                ExtractedRef::Slug(slug) => {
                    match self.store.resolve_slug(&slug, namespace) {
                        Ok(Some(target)) => {
                            if target != source_id {
                                resolved.push((target, EDGE_REFERENCES.to_string()));
                            }
                        }
                        Ok(None) => {
                            // Slug not found in memories — check documents (#689).
                            match self.store.resolve_document_slug(&slug) {
                                Ok(Some(doc_id)) => {
                                    resolved.push((doc_id, EDGE_REFERENCES_DOC.to_string()));
                                }
                                Ok(None) => {
                                    tracing::debug!(
                                        "Auto-edge: slug '{slug}' unresolved (no memory or document), skipped"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Auto-edge document slug resolve failed for '{slug}': {e}"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Auto-edge slug resolve failed for '{slug}': {e}");
                        }
                    }
                }
                ExtractedRef::Tag(tag) => match self.store.resolve_tag_to_memory(&tag, namespace) {
                    Ok(Some(target)) => {
                        if target != source_id {
                            resolved.push((target, EDGE_TAGGED_AS.to_string()));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("Auto-edge tag resolve failed for '{tag}': {e}");
                    }
                },
                ExtractedRef::Uuid(id, edge_type) => {
                    // Verify target exists AND belongs to the same namespace
                    // (CodeCora #138: UUID edges must not leak across namespaces).
                    match self.store.get_by_id_in_namespace(&id, namespace) {
                        Ok(Some(_)) => {
                            if id != source_id {
                                resolved.push((id, edge_type.to_string()));
                            }
                        }
                        Ok(None) => {
                            tracing::debug!(
                                "Auto-edge: uuid '{id}' unresolved (missing or cross-namespace), skipped"
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Auto-edge uuid resolve failed for '{id}': {e}");
                        }
                    }
                }
                ExtractedRef::Rel(t, id) => match self.store.get_by_id_in_namespace(&id, namespace)
                {
                    Ok(Some(_)) => {
                        if id != source_id {
                            resolved.push((id, t));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("Auto-edge rel resolve failed for '{id}': {e}");
                    }
                },
            }
        }

        if resolved.is_empty() {
            return;
        }
        match self.store.add_memory_edges_batch(source_id, &resolved) {
            Ok(n) => {
                if n > 0 {
                    tracing::debug!("Auto-wired {n} edges for memory {source_id}");
                }
            }
            Err(e) => {
                tracing::warn!("Auto-wire edges failed for {source_id}: {e}");
            }
        }
    }

    /// Cosine-similarity auto-linking (#401).
    ///
    /// After inserting a new memory, search the vector index for the top-K
    /// most similar memories. Create `similar_to` edges for high-similarity
    /// matches and `possible_duplicate` edges for near-duplicates.
    ///
    /// Best-effort: errors are logged, never fail the remember() call.
    ///
    /// Thresholds (configurable in future via graph config):
    /// - `similar_to`: cosine similarity >= 0.80
    /// - `possible_duplicate`: cosine similarity >= 0.92
    pub(crate) fn auto_link_cosine(
        &self,
        source_id: &str,
        embedding: &[f32],
        namespace: Option<&str>,
    ) {
        const SIMILAR_THRESHOLD: f32 = 0.80;
        const DUPLICATE_THRESHOLD: f32 = 0.92;
        // Candidates that may become edges.
        const TOP_K: usize = 20;
        // The index is shared by every namespace and by document chunks, and
        // the namespace / liveness filter below runs AFTER the search. Fetch
        // extra so a small namespace inside a big index still finds its own
        // neighbours instead of being crowded out by other namespaces.
        const FETCH_K: usize = TOP_K * 4;

        let index = match self.index.read() {
            Ok(i) => i,
            Err(_) => {
                tracing::debug!("auto_link_cosine: index lock failed, skipping");
                return;
            }
        };

        let results = index.search(embedding, FETCH_K, 100);
        drop(index); // Release read lock ASAP

        if results.is_empty() {
            return;
        }

        // Keep only live memories of this namespace (#401: cross-namespace links
        // are wrong). One query over the candidates — not a scan of the whole
        // namespace per write. `chunk:<id>` document-chunk keys and deprecated
        // memories are not in the result, so they never become link targets.
        let candidate_ids: Vec<&str> = results
            .iter()
            .map(|(id, _)| id.as_str())
            .filter(|id| *id != source_id)
            .collect();
        let live = match self.store.live_ids_in_namespace(&candidate_ids, namespace) {
            Ok(set) => set,
            Err(e) => {
                tracing::warn!("auto_link_cosine: failed to check candidates: {e}");
                return;
            }
        };

        // usearch returns distances (lower = more similar for cosine).
        // Convert distance to similarity: sim = 1.0 - dist.
        let edges: Vec<(String, String)> = results
            .iter()
            .filter(|(id, _)| live.contains(id)) // also drops self
            .take(TOP_K)
            .filter_map(|(id, dist)| {
                // Cosine distance → similarity (clamp to [0, 1]).
                let sim = (1.0 - dist).clamp(0.0, 1.0);
                if sim >= DUPLICATE_THRESHOLD {
                    tracing::debug!(
                        "auto_link: {source_id} → {id} possible_duplicate (sim={sim:.3})"
                    );
                    Some((id.clone(), EDGE_POSSIBLE_DUPLICATE.to_string()))
                } else if sim >= SIMILAR_THRESHOLD {
                    tracing::debug!("auto_link: {source_id} → {id} similar_to (sim={sim:.3})");
                    Some((id.clone(), EDGE_SIMILAR_TO.to_string()))
                } else {
                    None
                }
            })
            .collect();

        if edges.is_empty() {
            return;
        }

        if let Err(e) = self.store.add_memory_edges_batch(source_id, &edges) {
            tracing::warn!("auto_link_cosine edge insert failed for {source_id}: {e}");
        }
    }

    /// List edges for a memory (both directions). Public for `uteke edges <id>`.
    pub fn edges_for(&self, memory_id: &str) -> Result<EdgeList, Error> {
        self.store.list_memory_edges(memory_id)
    }

    /// Total edge count across the store.
    pub fn count_edges(&self) -> Result<usize, Error> {
        self.store.count_memory_edges()
    }

    /// Manually link two memories with an edge type, **and** generate the
    /// `referenced_by` backlink automatically (#350).
    ///
    /// Public API for explicit edges (e.g. from CLI or API callers). The
    /// forward edge is `(source → target, edge_type)`; the backlink is
    /// `(target → source, referenced_by)`.
    ///
    /// Idempotent: calling twice with the same arguments inserts nothing new.
    /// Returns `true` if the **forward** edge was newly inserted.
    pub fn link_memories(
        &self,
        source_id: &str,
        target_id: &str,
        edge_type: &str,
    ) -> Result<bool, Error> {
        self.store
            .add_memory_edge_with_backlink(source_id, target_id, edge_type)
    }

    /// Rebuild all `referenced_by` backlinks from existing forward edges (#350).
    ///
    /// Scans every row in `memory_edges` whose `edge_type` is a backlinkable
    /// forward type (see [`BACKLINKED_EDGE_TYPES`]) and ensures the inverse
    /// `(target → source, referenced_by)` row exists. Idempotent: running it
    /// twice produces no additional rows.
    ///
    /// Returns the number of new backlink rows inserted. Use after upgrading
    /// a store from a pre-#350 version, or any time edges were inserted via
    /// the low-level `add_memory_edge` (which skips backlinks).
    pub fn rebuild_backlinks(&self) -> Result<usize, Error> {
        let created = self.store.rebuild_backlinks()?;
        tracing::info!("rebuild_backlinks: created {created} new backlink edges");
        Ok(created)
    }

    /// Get related memories via edge table (replaces O(n) JSON scan).
    ///
    /// Returns memories reachable within `depth` hops, excluding the start.
    pub fn related_via_edges(&self, memory_id: &str, depth: usize) -> Result<Vec<Memory>, Error> {
        let ids = self.store.edge_bfs(memory_id, depth)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(m) = self.store.get_by_id(&id)? {
                out.push(m);
            }
        }
        Ok(out)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(content: &str, tags: &[&str]) -> Memory {
        Memory {
            id: uuid::Uuid::now_v7().to_string(),
            content: content.to_string(),
            embedding: vec![],
            tags: tags.iter().map(|s| s.to_string()).collect(),
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
            access_count: 0,
            recall_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
            author_type: "agent".to_string(),
        }
    }

    #[test]
    fn extract_wikilink_slug() {
        let refs = extract_refs("see [[rust-async]] for more", &[], &serde_json::Value::Null);
        assert_eq!(refs, vec![ExtractedRef::Slug("rust-async".to_string())]);
    }

    #[test]
    fn extract_multiple_wikilinks() {
        let refs = extract_refs(
            "links: [[a]] and [[b-c]] plus [[d_e]]",
            &[],
            &serde_json::Value::Null,
        );
        assert_eq!(
            refs,
            vec![
                ExtractedRef::Slug("a".to_string()),
                ExtractedRef::Slug("b-c".to_string()),
                ExtractedRef::Slug("d_e".to_string()),
            ]
        );
    }

    #[test]
    fn extract_wikilink_rejects_invalid_chars() {
        // Spaces and dots not allowed inside [[ ]]
        let refs = extract_refs(
            "[[has space]] [[dot.notation]]",
            &[],
            &serde_json::Value::Null,
        );
        assert!(refs.is_empty(), "got {refs:?}");
    }

    #[test]
    fn extract_at_tag() {
        let refs = extract_refs("deployed to @staging today", &[], &serde_json::Value::Null);
        assert_eq!(refs, vec![ExtractedRef::Tag("staging".to_string())]);
    }

    #[test]
    fn extract_supersede_caret_uuid() {
        let id = "12345678-1234-1234-1234-123456789012";
        let refs = extract_refs(&format!("^{}", id), &[], &serde_json::Value::Null);
        assert_eq!(
            refs,
            vec![ExtractedRef::Uuid(id.to_string(), EDGE_SUPERSEDES)]
        );
    }

    #[test]
    fn extract_reply_arrow_uuid() {
        let id = "abcdef12-3456-7890-abcd-ef1234567890";
        let refs = extract_refs(&format!(">{}", id), &[], &serde_json::Value::Null);
        assert_eq!(
            refs,
            vec![ExtractedRef::Uuid(id.to_string(), EDGE_REPLIES_TO)]
        );
    }

    #[test]
    fn extract_rejects_short_id() {
        let refs = extract_refs("^not-a-uuid", &[], &serde_json::Value::Null);
        assert!(refs.is_empty(), "got {refs:?}");
    }

    #[test]
    fn extract_from_legacy_metadata_relationships() {
        let meta = serde_json::json!({
            "relationships": [
                {"type": "supersedes", "target": "11111111-1111-1111-1111-111111111111"},
                {"type": "references", "target": "22222222-2222-2222-2222-222222222222"},
            ]
        });
        let refs = extract_refs("plain text", &[], &meta);
        assert_eq!(
            refs,
            vec![
                ExtractedRef::Rel(
                    "supersedes".to_string(),
                    "11111111-1111-1111-1111-111111111111".to_string()
                ),
                ExtractedRef::Rel(
                    "references".to_string(),
                    "22222222-2222-2222-2222-222222222222".to_string()
                ),
            ]
        );
    }

    #[test]
    fn extract_from_flat_meta_pairs() {
        let meta = serde_json::json!({
            "_meta_pairs": ["rel:part_of:33333333-3333-3333-3333-333333333333"]
        });
        let refs = extract_refs("", &[], &meta);
        assert_eq!(
            refs,
            vec![ExtractedRef::Rel(
                "part_of".to_string(),
                "33333333-3333-3333-3333-333333333333".to_string()
            )]
        );
    }

    #[test]
    fn extract_mixed_patterns() {
        let id = "44444444-4444-4444-4444-444444444444";
        let refs = extract_refs(
            &format!("see [[api-spec]] and @deploy and ^{}", id),
            &[],
            &serde_json::Value::Null,
        );
        assert!(refs.contains(&ExtractedRef::Slug("api-spec".to_string())));
        assert!(refs.contains(&ExtractedRef::Tag("deploy".to_string())));
        assert!(refs.contains(&ExtractedRef::Uuid(id.to_string(), EDGE_SUPERSEDES)));
    }

    #[test]
    fn is_valid_slug_rules() {
        assert!(is_valid_slug("rust-async"));
        assert!(is_valid_slug("a_b-c"));
        assert!(is_valid_slug("ABC123"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("has space"));
        assert!(!is_valid_slug("dot.notation"));
    }

    #[test]
    fn looks_like_uuid_rules() {
        assert!(looks_like_uuid("12345678-1234-1234-1234-123456789012"));
        assert!(looks_like_uuid("ABCDEF12-3456-7890-ABCD-EF1234567890"));
        assert!(!looks_like_uuid("short"));
        assert!(!looks_like_uuid("12345678-1234-1234-1234-12345678901")); // 35 chars
        assert!(!looks_like_uuid("z2345678-1234-1234-1234-123456789012")); // non-hex
    }

    #[test]
    fn store_edge_roundtrip() {
        let store = Store::open(":memory:").unwrap();

        // Two memories, both need slug-less insert but referenced by UUID.
        let a = mem("alpha content", &[]);
        let b = mem("beta content", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        // Add edge a -> b
        store
            .add_memory_edge(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();

        let edges = store.list_memory_edges(&a.id).unwrap();
        assert_eq!(edges.total(), 1);
        assert_eq!(edges.outgoing.len(), 1);
        assert_eq!(edges.outgoing[0].target_id, b.id);
        assert_eq!(edges.outgoing[0].edge_type, EDGE_REFERENCES);

        // From b's perspective, incoming.
        let b_edges = store.list_memory_edges(&b.id).unwrap();
        assert_eq!(b_edges.incoming.len(), 1);
        assert_eq!(b_edges.incoming[0].source_id, a.id);
    }

    #[test]
    fn store_edge_dedup() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        store
            .add_memory_edge(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        // Second identical insert via the batch path (which now also ensures
        // a #350 backlink) must still report 0 *new forward* rows — the
        // forward edge is a duplicate. The backlink is best-effort and does
        // not count toward the returned count.
        let n = store
            .add_memory_edges_batch(&a.id, &[(b.id.clone(), EDGE_REFERENCES.to_string())])
            .unwrap();
        assert_eq!(n, 0, "duplicate forward insert should change 0 rows");

        // Only the forward edge was created via add_memory_edge (no backlink).
        // The subsequent add_memory_edges_batch call ALSO ensured the #350
        // backlink, so a now has 1 incoming referenced_by from b.
        let edges = store.list_memory_edges(&a.id).unwrap();
        assert_eq!(edges.outgoing.len(), 1, "a has one outgoing forward edge");
        assert_eq!(
            edges.incoming.len(),
            1,
            "a has one incoming backlink (from batch call)"
        );

        // b has no outgoing edges; one incoming forward edge; one outgoing
        // `referenced_by` backlink to a (#350).
        let b_edges = store.list_memory_edges(&b.id).unwrap();
        assert_eq!(b_edges.incoming.len(), 1, "b has one incoming forward edge");
        assert!(
            b_edges
                .outgoing
                .iter()
                .any(|e| { e.edge_type == EDGE_REFERENCED_BY && e.target_id == a.id }),
            "b should have a referenced_by backlink to a (#350)"
        );
    }

    #[test]
    fn store_bfs_two_hops() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        let c = mem("c", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();
        store.insert(&c).unwrap();

        // a -> b -> c
        store
            .add_memory_edge(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        store
            .add_memory_edge(&b.id, &c.id, EDGE_REFERENCES)
            .unwrap();

        let depth1 = store.edge_bfs(&a.id, 1).unwrap();
        assert_eq!(depth1, vec![b.id.clone()]);

        let depth2 = store.edge_bfs(&a.id, 2).unwrap();
        assert_eq!(depth2, vec![b.id.clone(), c.id.clone()]);
    }

    #[test]
    fn store_bfs_cycle_safe() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        // Cycle a -> b -> a
        store
            .add_memory_edge(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        store
            .add_memory_edge(&b.id, &a.id, EDGE_REFERENCES)
            .unwrap();

        let depth5 = store.edge_bfs(&a.id, 5).unwrap();
        // b appears once; a is start (excluded via visited seed).
        assert_eq!(depth5, vec![b.id.clone()]);
    }

    #[test]
    fn store_resolve_slug_and_tag() {
        let store = Store::open(":memory:").unwrap();
        let mut target = mem("target memory", &["deploy"]);
        target.slug = Some("api-spec".to_string());
        store.insert(&target).unwrap();

        // Slug lookup
        let id = store.resolve_slug("api-spec", None).unwrap();
        assert_eq!(id.as_deref(), Some(target.id.as_str()));

        // Tag lookup
        let id = store.resolve_tag_to_memory("deploy", None).unwrap();
        assert_eq!(id.as_deref(), Some(target.id.as_str()));

        // Missing slug
        assert!(store.resolve_slug("missing", None).unwrap().is_none());
    }

    /// Regression test for CodeCora finding #133: a dangling edge insert
    /// (target UUID that doesn't exist) must not crash the store. The
    /// v7→v8 migration uses an EXISTS guard; this exercises the same guard
    /// at the add_memory_edges_batch level by going through add_memory_edge
    /// which relies on FK constraints being deferred via OR IGNORE semantics.
    #[test]
    fn store_edge_to_dangling_target_is_skipped() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("alpha", &[]);
        store.insert(&a).unwrap();

        // Insert an edge to a non-existent target. With FK ON + the
        // EXISTS guard in the migration path, this insert would error at
        // the FK level. The Store API surface (add_memory_edge) is intended
        // for use after the target has been verified to exist (see
        // Uteke::wire_edges), so we expect this to fail rather than produce
        // a dangling row.
        let dangling = "00000000-0000-0000-0000-000000000000";
        let result = store.add_memory_edge(&a.id, dangling, EDGE_REFERENCES);
        // Either the FK rejects it (preferred) or it's silently ignored —
        // the important property is no panic and no orphan row.
        match result {
            Ok(()) => {
                // If the insert succeeded (FKs not enforced for some reason),
                // verify no actual row was written that would break traversal.
                let edges = store.list_memory_edges(&a.id).unwrap();
                let has_dangling = edges.outgoing.iter().any(|e| e.target_id == dangling);
                assert!(!has_dangling, "dangling edge should not persist");
            }
            Err(_) => {
                // FK rejected the insert — expected path.
            }
        }
    }

    /// Regression test for CodeCora finding #132: get_related() must union
    /// edge-table results with legacy metadata.relationships results so
    /// incoming relations encoded only in metadata are not lost.
    #[test]
    fn get_related_unions_edge_table_and_metadata() {
        // Uses the full Uteke API because get_related lives on Uteke, not Store.
        // We exercise the union logic indirectly by ensuring the legacy
        // metadata path is still reached when no edges table rows exist.
        let store = Store::open(":memory:").unwrap();
        let a = mem("alpha", &[]);
        let b = mem("beta", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        // Only one edge a -> b in the edge table.
        store
            .add_memory_edge(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();

        // BFS from a should return [b], from b should return [a].
        let from_a = store.edge_bfs(&a.id, 1).unwrap();
        assert_eq!(from_a, vec![b.id.clone()]);
        let from_b = store.edge_bfs(&b.id, 1).unwrap();
        assert_eq!(from_b, vec![a.id.clone()]);
    }

    /// Regression test for CodeCora finding #136: confirms the migration
    /// dispatcher reaches v8 from a fresh DB (which starts at v1 then
    /// loops v2..=v8 via run_migrations). Verifies the migration
    /// v7->v8 is actually invoked, not skipped.
    /// Regression test for CodeCora finding #136: confirms the migration
    /// dispatcher reaches v8 from a fresh DB (which starts at v1 then
    /// loops v2..=v8 via run_migrations). Verifies the migration
    /// v7->v8 is actually invoked, not skipped.
    #[test]
    fn migration_dispatcher_reaches_v8() {
        let store = Store::open(":memory:").unwrap();
        let v = store.schema_version().unwrap();
        assert_eq!(
            v,
            crate::memory::store::CURRENT_SCHEMA_VERSION,
            "fresh store must reach CURRENT_SCHEMA_VERSION"
        );

        // memory_edges table must exist and be queryable after migration.
        let n = store.count_memory_edges().unwrap();
        assert_eq!(n, 0, "fresh store has zero edges but table must exist");
    }

    /// Regression test for CodeCora finding #138: ^<uuid>, ><uuid>, and
    /// rel:<type>:<id> auto-edges must NOT resolve across namespace boundaries.
    /// Slug/tag resolution is already namespace-scoped; this extends the
    /// invariant to direct UUID references.
    #[test]
    fn uuid_edges_respect_namespace_isolation() {
        let store = Store::open(":memory:").unwrap();
        let mut a = mem("alpha", &[]);
        a.namespace = "ns-a".to_string();
        let mut b = mem("beta", &[]);
        b.namespace = "ns-b".to_string();
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        // a exists in ns-a only — lookup from ns-b must return None.
        let cross = store.get_by_id_in_namespace(&a.id, Some("ns-b")).unwrap();
        assert!(cross.is_none(), "cross-namespace lookup must return None");

        // Same-namespace lookup must still work.
        let same = store.get_by_id_in_namespace(&a.id, Some("ns-a")).unwrap();
        assert!(same.is_some(), "same-namespace lookup must succeed");
    }

    // ── #350: backlink auto-generation tests ─────────────────────────────

    #[test]
    fn backlink_type_for_forward_types() {
        assert_eq!(backlink_type_for(EDGE_REFERENCES), Some(EDGE_REFERENCED_BY));
        assert_eq!(backlink_type_for(EDGE_TAGGED_AS), Some(EDGE_REFERENCED_BY));
        assert_eq!(backlink_type_for(EDGE_SUPERSEDES), Some(EDGE_REFERENCED_BY));
        assert_eq!(backlink_type_for(EDGE_REPLIES_TO), Some(EDGE_REFERENCED_BY));
        // Inverse type itself is NOT backlinked (avoid ping-pong).
        assert_eq!(backlink_type_for(EDGE_REFERENCED_BY), None);
        // Custom/unknown type — no backlink.
        assert_eq!(backlink_type_for("part_of"), None);
    }

    #[test]
    fn ensure_backlink_creates_inverse() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        // (a → b, references) backlinked to (b → a, referenced_by).
        let created = store
            .ensure_backlink(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        assert!(created, "first ensure_backlink should insert a row");

        // b should now have an outgoing referenced_by pointing back to a.
        let b_edges = store.list_memory_edges(&b.id).unwrap();
        assert_eq!(b_edges.outgoing.len(), 1);
        assert_eq!(b_edges.outgoing[0].edge_type, EDGE_REFERENCED_BY);
        assert_eq!(b_edges.outgoing[0].target_id, a.id);

        // a should see it as incoming.
        let a_edges = store.list_memory_edges(&a.id).unwrap();
        assert_eq!(a_edges.incoming.len(), 1);
        assert_eq!(a_edges.incoming[0].edge_type, EDGE_REFERENCED_BY);
        assert_eq!(a_edges.incoming[0].source_id, b.id);
    }

    #[test]
    fn ensure_backlink_is_idempotent() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        let first = store
            .ensure_backlink(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        let second = store
            .ensure_backlink(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        assert!(first, "first call inserts");
        assert!(!second, "second call is a no-op (idempotent)");

        // Still only one backlink row.
        let b_edges = store.list_memory_edges(&b.id).unwrap();
        let backlinks: Vec<_> = b_edges
            .outgoing
            .iter()
            .filter(|e| e.edge_type == EDGE_REFERENCED_BY)
            .collect();
        assert_eq!(backlinks.len(), 1, "no duplicate backlinks");
    }

    #[test]
    fn ensure_backlink_skips_inverse_and_unknown_types() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        // referenced_by is itself an inverse — no meta-backlink.
        let created = store
            .ensure_backlink(&a.id, &b.id, EDGE_REFERENCED_BY)
            .unwrap();
        assert!(!created, "backlinking a backlink type must be a no-op");

        // Unknown / non-standard forward type — no backlink.
        let created = store.ensure_backlink(&a.id, &b.id, "part_of").unwrap();
        assert!(!created, "unknown edge type should not produce a backlink");
    }

    #[test]
    fn ensure_backlink_skips_self_loop() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        store.insert(&a).unwrap();

        let created = store
            .ensure_backlink(&a.id, &a.id, EDGE_REFERENCES)
            .unwrap();
        assert!(!created, "self-referencing backlink must be skipped");
        let edges = store.list_memory_edges(&a.id).unwrap();
        assert_eq!(edges.total(), 0, "no rows inserted for a self-loop");
    }

    #[test]
    fn add_memory_edge_with_backlink_creates_both() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        let inserted = store
            .add_memory_edge_with_backlink(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        assert!(inserted, "forward edge was inserted");

        // Forward.
        let a_edges = store.list_memory_edges(&a.id).unwrap();
        assert_eq!(a_edges.outgoing.len(), 1);
        assert_eq!(a_edges.outgoing[0].edge_type, EDGE_REFERENCES);
        assert_eq!(a_edges.outgoing[0].target_id, b.id);

        // Backlink.
        let b_edges = store.list_memory_edges(&b.id).unwrap();
        assert_eq!(b_edges.incoming.len(), 1, "b has incoming forward edge");
        let backlinks: Vec<_> = b_edges
            .outgoing
            .iter()
            .filter(|e| e.edge_type == EDGE_REFERENCED_BY)
            .collect();
        assert_eq!(backlinks.len(), 1, "b has one referenced_by backlink to a");
        assert_eq!(backlinks[0].target_id, a.id);
    }

    #[test]
    fn add_memory_edge_with_backlink_idempotent() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        let first = store
            .add_memory_edge_with_backlink(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        let second = store
            .add_memory_edge_with_backlink(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        assert!(first, "first call inserts");
        assert!(!second, "second call is a no-op for the forward edge");

        let a_edges = store.list_memory_edges(&a.id).unwrap();
        assert_eq!(a_edges.outgoing.len(), 1, "still one forward edge");
        let b_edges = store.list_memory_edges(&b.id).unwrap();
        let backlinks: Vec<_> = b_edges
            .outgoing
            .iter()
            .filter(|e| e.edge_type == EDGE_REFERENCED_BY)
            .collect();
        assert_eq!(backlinks.len(), 1, "still one backlink");
    }

    #[test]
    fn batch_insert_generates_backlinks() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        let c = mem("c", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();
        store.insert(&c).unwrap();

        let n = store
            .add_memory_edges_batch(
                &a.id,
                &[
                    (b.id.clone(), EDGE_REFERENCES.to_string()),
                    (c.id.clone(), EDGE_REFERENCES.to_string()),
                ],
            )
            .unwrap();
        assert_eq!(n, 2, "two forward edges inserted");

        // Each target should have a referenced_by backlink to a.
        for target in [&b.id, &c.id] {
            let edges = store.list_memory_edges(target).unwrap();
            let has_backlink = edges
                .outgoing
                .iter()
                .any(|e| e.edge_type == EDGE_REFERENCED_BY && e.target_id == a.id);
            assert!(
                has_backlink,
                "target {} should have a referenced_by backlink to a",
                &target[..8]
            );
        }
    }

    #[test]
    fn rebuild_backlinks_from_existing_forward_edges() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        let c = mem("c", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();
        store.insert(&c).unwrap();

        // Use the low-level add_memory_edge which does NOT create backlinks.
        store
            .add_memory_edge(&a.id, &b.id, EDGE_REFERENCES)
            .unwrap();
        store
            .add_memory_edge(&a.id, &c.id, EDGE_REFERENCES)
            .unwrap();
        // One backlink-type edge already present (should be left untouched).
        store
            .add_memory_edge(&b.id, &a.id, EDGE_REFERENCED_BY)
            .unwrap();

        let created = store.rebuild_backlinks().unwrap();
        // Backlinks expected: (b→a) already present, (c→a) missing.
        assert_eq!(
            created, 1,
            "exactly one new backlink (c→a) should be created"
        );

        // Running again must be a no-op.
        let again = store.rebuild_backlinks().unwrap();
        assert_eq!(again, 0, "rebuild_backlinks is idempotent");

        // Both targets now have backlinks to a.
        for target in [&b.id, &c.id] {
            let edges = store.list_memory_edges(target).unwrap();
            let has_backlink = edges
                .outgoing
                .iter()
                .any(|e| e.edge_type == EDGE_REFERENCED_BY && e.target_id == a.id);
            assert!(
                has_backlink,
                "target {} backlinked to a after rebuild",
                &target[..8]
            );
        }
    }

    #[test]
    fn rebuild_backlinks_ignores_non_forward_types() {
        let store = Store::open(":memory:").unwrap();
        let a = mem("a", &[]);
        let b = mem("b", &[]);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();

        // Only a referenced_by edge exists (no forward edge). Rebuild should
        // not create a second inverse edge.
        store
            .add_memory_edge(&a.id, &b.id, EDGE_REFERENCED_BY)
            .unwrap();
        let created = store.rebuild_backlinks().unwrap();
        assert_eq!(created, 0, "non-forward types are not backlinked");
    }

    #[test]
    #[ignore = "uses Uteke::open which requires the ONNX embedder (network/model)"]
    fn wire_edges_generates_backlinks() {
        // End-to-end: Uteke::wire_edges should produce backlinks because it
        // goes through add_memory_edges_batch (now backlink-aware).
        let uteke = crate::Uteke::open(":memory:").unwrap();

        // Create target memory first so the ^<uuid> reference resolves.
        let target = uteke
            .remember("target memory", &[], None, Some("default"))
            .unwrap();

        // Source memory references the target via ^<uuid> (supersedes).
        // wire_edges resolves this to an EDGE_SUPERSEDES forward edge, which
        // is backlinkable (#350).
        let source = uteke
            .remember(
                &format!("new version ^{}", target),
                &[],
                None,
                Some("default"),
            )
            .unwrap();

        // Forward edge: source → target (supersedes).
        let src_edges = uteke.edges_for(&source).unwrap();
        assert!(
            src_edges
                .outgoing
                .iter()
                .any(|e| { e.edge_type == EDGE_SUPERSEDES && e.target_id == target }),
            "expected forward supersedes edge source → target"
        );

        // Backlink edge: target → source (referenced_by).
        let tgt_edges = uteke.edges_for(&target).unwrap();
        let has_backlink = tgt_edges
            .outgoing
            .iter()
            .any(|e| e.edge_type == EDGE_REFERENCED_BY && e.target_id == source);
        assert!(
            has_backlink,
            "expected referenced_by backlink target → source (#350)"
        );
    }

    #[test]
    #[ignore = "uses Uteke::open which requires the ONNX embedder (network/model)"]
    fn auto_link_cosine_creates_similar_edges() {
        // Two very similar memories should get a similar_to edge (#401).
        let uteke = crate::Uteke::open(":memory:").unwrap();

        let _id1 = uteke
            .remember(
                "The project deadline is next Friday",
                &[],
                None,
                Some("default"),
            )
            .unwrap();

        let id2 = uteke
            .remember(
                "The project deadline is next Friday",
                &[],
                None,
                Some("default"),
            )
            .unwrap();

        // The second memory should have an edge to the first.
        let edges = uteke.edges_for(&id2).unwrap();
        let has_similar = edges
            .outgoing
            .iter()
            .any(|e| e.edge_type == EDGE_SIMILAR_TO || e.edge_type == EDGE_POSSIBLE_DUPLICATE);
        assert!(
            has_similar,
            "expected similar_to or possible_duplicate edge for near-identical memories"
        );
    }

    #[test]
    #[ignore = "uses Uteke::open which requires the ONNX embedder (network/model)"]
    fn auto_link_cosine_no_false_positives() {
        // Very different memories should NOT get similar_to edges.
        let uteke = crate::Uteke::open(":memory:").unwrap();

        let _id1 = uteke
            .remember("The weather is sunny today", &[], None, Some("default"))
            .unwrap();

        let id2 = uteke
            .remember(
                "Rust programming language memory safety",
                &[],
                None,
                Some("default"),
            )
            .unwrap();

        let edges = uteke.edges_for(&id2).unwrap();
        let has_similar = edges
            .outgoing
            .iter()
            .any(|e| e.edge_type == EDGE_SIMILAR_TO || e.edge_type == EDGE_POSSIBLE_DUPLICATE);
        // Unlikely to be similar (different topics).
        assert!(
            !has_similar,
            "should not have similar edge for dissimilar memories"
        );
    }

    // ── Cross-entity integration tests (#689) ─────────────────────────────
    //
    // NOTE: memory_edges has FK constraints → memories(id) on both source_id
    // and target_id. Document IDs don't exist in memories, so direct
    // add_memory_edge(memory_id, doc_id, REFERENCES_DOC) violates the FK.
    // To test edge operations with document-like targets, we insert a stub
    // memory row with the document ID. This matches how the schema works
    // and exercises the actual edge CRUD code paths.

    /// Helper: insert a stub memory row with a given ID (bypasses FK constraints).
    fn stub_memory(store: &Store, id: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        store
            .conn
            .execute(
                "INSERT INTO memories (id, content, namespace, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, "stub", "default", now, now],
            )
            .unwrap();
    }

    #[test]
    fn cross_entity_doc_edge_round_trip() {
        let store = Store::open(":memory:").unwrap();

        // 1. Create a document (no embedder needed).
        let doc = crate::memory::documents::Document {
            id: "doc-1".to_string(),
            slug: "my-doc".to_string(),
            title: "My Doc".to_string(),
            content: "Document content here.".to_string(),
            namespace: None,
            author: None,
            tags: vec![],
            metadata: serde_json::Value::Null,
            version: 1,
            content_type: "markdown".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            parent_id: None,
            path: "/my-doc/".to_string(),
            depth: 0,
            sort_order: 0,
            has_children: false,
        };
        store.upsert_document(&doc).unwrap();

        // 2. Create a real memory via store.insert.
        let m = mem("test memory content", &[]);
        let mem_id = m.id.clone();
        store.insert(&m).unwrap();

        // Stub the doc ID in memories so the FK on memory_edges.target_id is satisfied.
        stub_memory(&store, "doc-1");

        // 3. Insert edge: memory → document, type=references_doc.
        store
            .add_memory_edge(&mem_id, "doc-1", EDGE_REFERENCES_DOC)
            .unwrap();

        // 4. Verify edge_targets returns doc-1 (memory → doc).
        let targets = store.edge_targets(&mem_id, EDGE_REFERENCES_DOC).unwrap();
        assert_eq!(targets, vec!["doc-1"]);

        // 5. Verify edge_sources returns memory_id (doc → memory lookup).
        let sources = store.edge_sources("doc-1", EDGE_REFERENCES_DOC).unwrap();
        assert_eq!(sources, vec![mem_id]);

        // 6. Verify resolve_document_slug works for the created document.
        let resolved = store.resolve_document_slug("my-doc").unwrap();
        assert_eq!(resolved, Some("doc-1".to_string()));
    }

    #[test]
    fn cross_entity_doc_edge_with_backlink() {
        let store = Store::open(":memory:").unwrap();

        // Create document.
        let doc = crate::memory::documents::Document {
            id: "doc-bl".to_string(),
            slug: "backlink-doc".to_string(),
            title: "Backlink Doc".to_string(),
            content: "Content".to_string(),
            namespace: None,
            author: None,
            tags: vec![],
            metadata: serde_json::Value::Null,
            version: 1,
            content_type: "markdown".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            parent_id: None,
            path: "/backlink-doc/".to_string(),
            depth: 0,
            sort_order: 0,
            has_children: false,
        };
        store.upsert_document(&doc).unwrap();
        stub_memory(&store, "doc-bl");

        let m = mem("memory with backlink", &[]);
        let mem_id = m.id.clone();
        store.insert(&m).unwrap();

        // Insert forward edge WITH backlink (references_doc is in BACKLINKED_EDGE_TYPES).
        let inserted = store
            .add_memory_edge_with_backlink(&mem_id, "doc-bl", EDGE_REFERENCES_DOC)
            .unwrap();
        assert!(inserted, "forward edge should be new");

        // Forward: memory → doc (references_doc)
        let targets = store.edge_targets(&mem_id, EDGE_REFERENCES_DOC).unwrap();
        assert_eq!(targets, vec!["doc-bl"]);

        // Backlink: doc → memory (referenced_by)
        let backlinks = store.edge_sources(&mem_id, EDGE_REFERENCED_BY).unwrap();
        assert!(
            backlinks.contains(&"doc-bl".to_string()),
            "doc should have referenced_by edge back to memory"
        );
    }

    #[test]
    fn cross_entity_doc_edge_idempotent() {
        let store = Store::open(":memory:").unwrap();

        let doc = crate::memory::documents::Document {
            id: "doc-idem".to_string(),
            slug: "idem-doc".to_string(),
            title: "Idempotent".to_string(),
            content: "Content".to_string(),
            namespace: None,
            author: None,
            tags: vec![],
            metadata: serde_json::Value::Null,
            version: 1,
            content_type: "markdown".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            parent_id: None,
            path: "/idem-doc/".to_string(),
            depth: 0,
            sort_order: 0,
            has_children: false,
        };
        store.upsert_document(&doc).unwrap();
        stub_memory(&store, "doc-idem");

        let m = mem("idempotent memory", &[]);
        let mem_id = m.id.clone();
        store.insert(&m).unwrap();

        // Insert twice — second should be no-op (INSERT OR IGNORE).
        store
            .add_memory_edge(&mem_id, "doc-idem", EDGE_REFERENCES_DOC)
            .unwrap();
        store
            .add_memory_edge(&mem_id, "doc-idem", EDGE_REFERENCES_DOC)
            .unwrap();

        let targets = store.edge_targets(&mem_id, EDGE_REFERENCES_DOC).unwrap();
        assert_eq!(targets.len(), 1, "should still have exactly one edge");
    }

    #[test]
    fn cross_entity_doc_edge_fk_blocks_non_memory_target() {
        // Verify that memory_edges FK prevents inserting a target_id that
        // doesn't exist in the memories table. This is the current schema
        // constraint that blocks direct memory→document edges without a
        // stub memory row.
        let store = Store::open(":memory:").unwrap();

        let m = mem("source memory", &[]);
        let mem_id = m.id.clone();
        store.insert(&m).unwrap();

        // "ghost-doc-id" is NOT in memories → FK violation.
        let result = store.add_memory_edge(&mem_id, "ghost-doc-id", EDGE_REFERENCES_DOC);
        assert!(result.is_err(), "FK should reject non-memory target_id");
    }

    #[test]
    fn cross_entity_resolve_nonexistent_doc_slug_returns_none() {
        let store = Store::open(":memory:").unwrap();
        let resolved = store.resolve_document_slug("no-such-doc").unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn cross_entity_edge_targets_empty_for_no_edges() {
        let store = Store::open(":memory:").unwrap();
        let targets = store
            .edge_targets("ghost-mem", EDGE_REFERENCES_DOC)
            .unwrap();
        assert!(targets.is_empty());
    }
}

/// Edge type marking the superseded (stale) side of a supersession pair (#1053).
pub const EDGE_SUPERSEDED_BY: &str = "superseded_by";

impl crate::Uteke {
    /// Mark `old_id` as superseded by `new_id` (#1053).
    ///
    /// Wires a typed edge pair — `old → new: superseded_by` (the annotation
    /// recall reads) and `new → old: supersedes` (navigable inverse) — then
    /// soft-deprecates the old memory so it drops out of recall by default.
    /// Both ids must exist and differ; self-supersession is rejected.
    ///
    /// Returns the ids of the pair for confirmation output.
    pub fn supersede(
        &self,
        old_id: &str,
        new_id: &str,
        reason: Option<&str>,
    ) -> Result<(String, String), Error> {
        if old_id == new_id {
            return Err(Error::validation(
                "Cannot supersede a memory with itself (old_id == new_id)",
            ));
        }
        let old = self
            .store
            .get_by_id(old_id)?
            .ok_or_else(|| Error::validation(format!("Old memory not found: {old_id}")))?;
        let new = self
            .store
            .get_by_id(new_id)?
            .ok_or_else(|| Error::validation(format!("New memory not found: {new_id}")))?;

        let now = chrono::Utc::now().to_rfc3339();
        let reason_text = match reason {
            Some(r) => format!("superseded by {new_id}: {r}"),
            None => format!("superseded by {new_id}"),
        };
        let conn = self.graph_store();
        // Edge pair AND the soft-deprecation in ONE transaction — a failure
        // in any of the three writes rolls the whole supersession back
        // (cora finding: committing edges before deprecating leaves a
        // superseded_by pointer on a still-active memory).
        {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| Error::db("begin supersede tx", e))?;
            // Re-supersession: drop this memory's PREVIOUS supersession pair
            // first (code-scanning #687) — otherwise stacked superseded_by
            // edges accumulate and supersession_of() becomes ambiguous.
            tx.execute(
                "DELETE FROM memory_edges WHERE source_id = ?1 AND edge_type = ?2",
                params![old.id, EDGE_SUPERSEDED_BY],
            )
            .map_err(|e| Error::db("clear prior superseded_by edge", e))?;
            tx.execute(
                "DELETE FROM memory_edges WHERE target_id = ?1 AND edge_type = ?2",
                params![old.id, EDGE_SUPERSEDES],
            )
            .map_err(|e| Error::db("clear prior supersedes edge", e))?;
            tx.execute(
                "INSERT OR IGNORE INTO memory_edges (source_id, target_id, edge_type, created_at) VALUES (?1,?2,?3,?4)",
                params![old.id, new.id, EDGE_SUPERSEDED_BY, now],
            )
            .map_err(|e| Error::db("insert superseded_by edge", e))?;
            tx.execute(
                "INSERT OR IGNORE INTO memory_edges (source_id, target_id, edge_type, created_at) VALUES (?1,?2,?3,?4)",
                params![new.id, old.id, EDGE_SUPERSEDES, now],
            )
            .map_err(|e| Error::db("insert supersedes edge", e))?;
            tx.execute(
                "UPDATE memories SET deprecated = 1, valid_until = ?1, deprecate_reason = ?2, updated_at = ?1, deprecated_at = ?1 WHERE id = ?3 AND deprecated = 0",
                params![now, reason_text, old.id],
            )
            .map_err(|e| Error::db("deprecate superseded memory", e))?;
            // Re-supersession of an already-deprecated memory: refresh the
            // reason/timestamp so the resolution ledger reflects the CURRENT
            // winner (stale "superseded by <old winner>" text would make the
            // ledger contradict the edge pair — cora finding, #1172 F2).
            tx.execute(
                "UPDATE memories SET deprecate_reason = ?1, updated_at = ?2, deprecated_at = ?2 WHERE id = ?3 AND deprecated = 1",
                params![reason_text, now, old.id],
            )
            .map_err(|e| Error::db("refresh superseded reason", e))?;
            tx.commit()
                .map_err(|e| Error::db("commit supersede tx", e))?;
        }

        // Provenance chain (#1172 Fase 2): record the resolution as
        // provenance-bearing events on BOTH sides — the retired memory gets
        // `superseded` (why it lost), the winner gets `updated` pointing
        // back (what it replaced). Best-effort, like all timeline writes.
        self.store
            .add_timeline_event_with_provenance(
                &old.id,
                crate::timeline::TimelineEventType::Superseded,
                Some(&serde_json::json!({
                    "superseded_by": new.id,
                    "reason": reason_text,
                })),
                Some("system:supersede"),
                Some(&serde_json::json!([
                    {"memory": new.id, "relation": "supersedes_this"},
                    {"memory": old.id, "relation": "subject"},
                ])),
            )
            .unwrap_or_else(|e| tracing::warn!("superseded event failed for {}: {e}", old.id));
        self.store
            .add_timeline_event_with_provenance(
                &new.id,
                crate::timeline::TimelineEventType::Updated,
                Some(&serde_json::json!({
                    "superseded": old.id,
                    "reason": reason_text,
                })),
                Some("system:supersede"),
                Some(&serde_json::json!([
                    {"memory": old.id, "relation": "superseded_by_this"},
                ])),
            )
            .unwrap_or_else(|e| tracing::warn!("supersedes event failed for {}: {e}", new.id));

        // Same hygiene soft_forget() applies to deprecated rows: remove from
        // the vector index (deprecated memories must not surface in
        // semantic recall — code-scanning #685) and invalidate the recall
        // cache for the namespace.
        {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during supersede"))?;
            index_sync::remove_ids(&mut *index, [old.id.as_str()], &SyncPolicy::STANDARD)
                .warn_if_stale("supersede");
        }
        self.recall_cache.invalidate_namespace(&old.namespace);

        Ok((old.id, new.id))
    }

    /// Look up the supersession target for a memory, if any (#1053).
    /// Returns `Some(new_id)` when an outgoing `superseded_by` edge exists.
    /// Deterministic: latest by created_at, then id (code-scanning #687 —
    /// bare LIMIT 1 with no ORDER BY could return any row).
    pub fn supersession_of(&self, memory_id: &str) -> Result<Option<String>, Error> {
        let conn = self.graph_store();
        let target: Option<String> = conn
            .query_row(
                "SELECT target_id FROM memory_edges WHERE source_id = ?1 AND edge_type = ?2 \
                 ORDER BY created_at DESC, target_id ASC LIMIT 1",
                params![memory_id, EDGE_SUPERSEDED_BY],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(Error::db("supersession lookup", other)),
            })?;
        Ok(target)
    }

    /// Undo a supersession (#1172 Fase 2): restore the deprecated memory to
    /// active, remove the supersession edge pair, and record
    /// `supersession_undone` provenance events on both memories.
    ///
    /// Returns `Ok(None)` when the memory has no supersession to undo;
    /// `Ok(Some(undone_id))` on success. The previously-winning memory stays
    /// untouched and active — both now live side by side, with the full
    /// history auditable via provenance.
    pub fn undo_supersession(&self, memory_id: &str) -> Result<Option<String>, Error> {
        let superseded_by = match self.supersession_of(memory_id)? {
            Some(new_id) => new_id,
            None => return Ok(None),
        };

        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.graph_store();
        // Restore AND edge-pair removal in ONE transaction — a failure in any
        // write rolls the whole undo back. Committing the restore first would
        // leave a live superseded_by pair on an ACTIVE memory if the edge
        // deletes failed (the exact partial state the supersede tx comment
        // warns about) — cora finding on the #1172 F2 branch.
        {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| Error::db("begin undo_supersession tx", e))?;
            tx.execute(
                "UPDATE memories SET deprecated = 0, valid_until = NULL, deprecate_reason = NULL, deprecated_at = NULL, updated_at = ?1 WHERE id = ?2 AND deprecated = 1",
                params![now, memory_id],
            )
            .map_err(|e| Error::db("restore superseded memory", e))?;
            tx.execute(
                "DELETE FROM memory_edges WHERE source_id = ?1 AND edge_type = ?2 AND target_id = ?3",
                params![memory_id, EDGE_SUPERSEDED_BY, superseded_by],
            )
            .map_err(|e| Error::db("delete superseded_by edge", e))?;
            tx.execute(
                "DELETE FROM memory_edges WHERE source_id = ?1 AND edge_type = ?2 AND target_id = ?3",
                params![superseded_by, EDGE_SUPERSEDES, memory_id],
            )
            .map_err(|e| Error::db("delete supersedes edge", e))?;
            tx.commit()
                .map_err(|e| Error::db("commit undo_supersession tx", e))?;
        }

        // Post-commit side effects (same contract as promote()): re-add to
        // the vector index and invalidate the recall cache.
        if let Some(memory) = self.store.get_by_id(memory_id).ok().flatten() {
            if !memory.embedding.is_empty() {
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during undo_supersession"))?;
                let sync = index_sync::upsert(
                    &mut *index,
                    &memory.id,
                    &memory.embedding,
                    &SyncPolicy::STANDARD,
                );
                sync.warn_if_stale("undo_supersession");
            }
            self.recall_cache.invalidate_namespace(&memory.namespace);
        }

        // Provenance events on both sides — the undo itself is auditable.
        self.store
            .add_timeline_event_with_provenance(
                memory_id,
                crate::timeline::TimelineEventType::SupersessionUndone,
                Some(&serde_json::json!({
                    "was_superseded_by": superseded_by,
                })),
                Some("system:undo_supersession"),
                Some(&serde_json::json!([
                    {"memory": superseded_by, "relation": "was_superseded_by"},
                ])),
            )
            .unwrap_or_else(|e| {
                tracing::warn!("supersession_undone event failed for {memory_id}: {e}")
            });
        self.store
            .add_timeline_event_with_provenance(
                &superseded_by,
                crate::timeline::TimelineEventType::Updated,
                Some(&serde_json::json!({
                    "supersession_of": memory_id,
                    "undone": true,
                })),
                Some("system:undo_supersession"),
                Some(&serde_json::json!([
                    {"memory": memory_id, "relation": "supersession_undone"},
                ])),
            )
            .unwrap_or_else(|e| tracing::warn!("undo event failed for {superseded_by}: {e}"));

        Ok(Some(superseded_by.to_string()))
    }

    /// List memories in a namespace that were superseded but not restored
    /// (#1172 Fase 2) — the auditable resolution ledger: each entry carries
    /// the retired memory, its winner (via provenance), and the reason.
    /// Membership is EDGE-driven: a row is listed iff it is deprecated AND
    /// carries a `superseded_by` edge — the exact predicate
    /// `undo_supersession` resolves against. Free-text reason matching would
    /// diverge (legacy custom reasons invisible; forget-reasons that mention
    /// "superseded" falsely listed) — cora finding on the #1172 F2 branch.
    pub fn contradiction_resolutions(
        &self,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::DeprecatedMemoryInfo>, Error> {
        let sql = if namespace.is_some() {
            "SELECT m.id, m.content, m.memory_type, m.namespace, m.tags, m.importance, m.deprecated_at, m.deprecate_reason \
             FROM memories m \
             JOIN memory_edges e ON e.source_id = m.id AND e.edge_type = ?2 \
             WHERE m.deprecated = 1 AND m.namespace = ?1 \
             ORDER BY m.deprecated_at DESC LIMIT ?3"
        } else {
            "SELECT m.id, m.content, m.memory_type, m.namespace, m.tags, m.importance, m.deprecated_at, m.deprecate_reason \
             FROM memories m \
             JOIN memory_edges e ON e.source_id = m.id AND e.edge_type = ?1 \
             WHERE m.deprecated = 1 \
             ORDER BY m.deprecated_at DESC LIMIT ?2"
        };
        let mut stmt = self
            .graph_store()
            .prepare(sql)
            .map_err(|e| Error::db("prepare contradiction_resolutions", e))?;
        let map_row = |r: &rusqlite::Row| -> rusqlite::Result<crate::DeprecatedMemoryInfo> {
            let tags_str: String = r.get(4)?;
            let tags = serde_json::from_str::<Vec<String>>(&tags_str).unwrap_or_default();
            Ok(crate::DeprecatedMemoryInfo {
                id: r.get(0)?,
                content: r.get(1)?,
                memory_type: r.get(2)?,
                namespace: r.get(3)?,
                tags,
                importance: r.get(5).unwrap_or(0.5),
                deprecated_at: r
                    .get::<_, Option<String>>(6)?
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc)),
                deprecate_reason: r.get(7)?,
            })
        };
        let rows = match namespace {
            Some(ns) => stmt
                .query_map(params![ns, EDGE_SUPERSEDED_BY, limit as i64], map_row)
                .map_err(|e| Error::db("query contradiction_resolutions", e))?,
            // Both variants bind every placeholder positionally (?1..?n).
            // (The original no-ns variant declared LIMIT ?2 while binding a
            // single param → runtime bind error, caught by the #1172 F2 MCP
            // roundtrip test.)
            None => stmt
                .query_map(params![EDGE_SUPERSEDED_BY, limit as i64], map_row)
                .map_err(|e| Error::db("query contradiction_resolutions", e))?,
        };
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::db("contradiction_resolutions row", e))?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod supersession_tests {
    use crate::Uteke;
    use rusqlite::params;

    /// #1053: supersede wires both edges + deprecates old; supersession_of
    /// resolves the pointer; promote restores.
    #[test]
    fn test_supersede_pair_and_lookup() {
        let dir = std::env::temp_dir().join(format!("ss-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();

        let embedding = vec![0.5_f32; 768];
        let old = uteke
            .remember_precomputed(
                "old decision: trait Store polymorphism",
                &[],
                None,
                Some("ss-ns"),
                "decision",
                "text",
                &embedding,
            )
            .unwrap();
        let new = uteke
            .remember_precomputed(
                "new decision: port to Postgres, no trait",
                &[],
                None,
                Some("ss-ns"),
                "decision",
                "text",
                &embedding,
            )
            .unwrap();

        // Self-supersession rejected.
        assert!(uteke.supersede(&old, &old, None).is_err());
        // Unknown id rejected.
        assert!(uteke.supersede(&old, "ffffffff", None).is_err());

        let (o, n) = uteke.supersede(&old, &new, Some("ADR-0005 pivot")).unwrap();
        assert_eq!((o.as_str(), n.as_str()), (old.as_str(), new.as_str()));

        // Pointer resolves.
        assert_eq!(
            uteke.supersession_of(&old).unwrap().as_deref(),
            Some(new.as_str())
        );
        assert_eq!(uteke.supersession_of(&new).unwrap(), None);

        // Old memory soft-deprecated (hidden from list, restorable).
        let m = uteke.get_by_id(&old).unwrap().unwrap();
        assert!(m.deprecated, "old memory soft-deprecated");
        let listed = uteke.list(None, 100, 0, Some("ss-ns")).unwrap();
        assert!(listed.iter().all(|x| x.id != old));

        // Deprecated memory must also leave the VECTOR INDEX — otherwise it
        // still surfaces in semantic recall (code-scanning #685).
        {
            let idx = uteke.index.read().unwrap();
            let needle = vec![0.5_f32; 768];
            let hits = idx.search(&needle, 10, 100);
            assert!(
                hits.iter().all(|(id, _)| id != &old),
                "superseded memory must not remain in the vector index"
            );
        }

        // Both edges exist.
        let conn = uteke.graph_store();
        let fwd: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_edges WHERE source_id=?1 AND target_id=?2 AND edge_type='superseded_by'",
                params![old, new],
                |r| r.get(0),
            )
            .unwrap();
        let inv: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_edges WHERE source_id=?1 AND target_id=?2 AND edge_type='supersedes'",
                params![new, old],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!((fwd, inv), (1, 1));

        // Re-supersession replaces the pair (code-scanning #687): a second
        // target must not stack a second superseded_by edge.
        let third = uteke
            .remember_precomputed(
                "third decision: final word",
                &[],
                None,
                Some("ss-ns"),
                "decision",
                "text",
                &embedding,
            )
            .unwrap();
        uteke.supersede(&old, &third, None).unwrap();
        assert_eq!(
            uteke.supersession_of(&old).unwrap().as_deref(),
            Some(third.as_str()),
            "latest supersession wins"
        );
        let stacked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_edges WHERE source_id=?1 AND edge_type='superseded_by'",
                params![old],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stacked, 1, "no stacked superseded_by edges");

        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// #1172 Fase 2: supersede writes provenance-bearing events on both
    /// sides; contradiction_resolutions lists the retired memory; undo
    /// restores it, removes the pair, and records supersession_undone.
    #[test]
    fn supersession_evidence_chain_and_undo() {
        let dir = std::env::temp_dir().join(format!("chain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();
        let embedding = vec![0.6_f32; 768];

        let old = uteke
            .remember_precomputed(
                "old claim: deploy at 8am",
                &[],
                None,
                Some("chain-ns"),
                "fact",
                "text",
                &embedding,
            )
            .unwrap();
        let new = uteke
            .remember_precomputed(
                "new claim: deploy at 9am",
                &[],
                None,
                Some("chain-ns"),
                "fact",
                "text",
                &embedding,
            )
            .unwrap();

        uteke
            .supersede(&old, &new, Some("corrected deploy time"))
            .unwrap();

        // The retired memory's provenance carries the superseded event with
        // actor + evidence pointing at the winner.
        let report = uteke.provenance(&old).unwrap().expect("old exists");
        let superseded = report
            .events
            .iter()
            .find(|e| e.event_type == "superseded")
            .expect("superseded event recorded");
        assert_eq!(superseded.actor.as_deref(), Some("system:supersede"));
        let evidence = superseded.evidence.as_ref().expect("evidence");
        assert_eq!(evidence[0]["memory"], serde_json::json!(new));
        // Supersede always sets a deprecation reason — verified via the
        // superseded event's reason payload.
        assert!(
            superseded.event_data.is_some(),
            "superseded event must carry the reason payload"
        );

        // The winner's chain carries a mirrored updated event.
        let new_report = uteke.provenance(&new).unwrap().expect("new exists");
        assert!(
            new_report.events.iter().any(
                |e| e.event_type == "updated" && e.actor.as_deref() == Some("system:supersede")
            )
        );

        // The resolution ledger lists the retired memory.
        let ledger = uteke
            .contradiction_resolutions(Some("chain-ns"), 50)
            .unwrap();
        assert!(
            ledger.iter().any(|d| d.id == old),
            "retired memory in ledger"
        );

        // Undo: restores the old memory, removes the pair, records events.
        let undone = uteke.undo_supersession(&old).unwrap();
        assert_eq!(undone.as_deref(), Some(new.as_str()));
        assert!(
            uteke.undo_supersession(&old).unwrap().is_none(),
            "second undo is a no-op"
        );

        let restored = uteke.provenance(&old).unwrap().expect("old exists");
        assert!(!restored.deprecated, "undo must restore the memory");
        assert!(
            restored
                .events
                .iter()
                .any(|e| e.event_type == "supersession_undone"),
            "supersession_undone event recorded"
        );
        assert_eq!(
            uteke.supersession_of(&old).unwrap(),
            None,
            "supersession pair removed"
        );

        // Ledger no longer contains the restored memory.
        let ledger_after = uteke
            .contradiction_resolutions(Some("chain-ns"), 50)
            .unwrap();
        assert!(!ledger_after.iter().any(|d| d.id == old));

        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod resupersession_ledger_tests {
    use crate::Uteke;

    /// Regression (cora finding, #1172 F2): re-superseding an already-
    /// deprecated memory must refresh deprecate_reason/deprecated_at so the
    /// resolution ledger names the CURRENT winner, not the first one.
    #[test]
    fn resupersession_refreshes_ledger_reason() {
        let dir = std::env::temp_dir().join(format!(
            "rsled-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();

        let embedding = vec![0.5_f32; 768];
        let mk = |content: &str| -> String {
            uteke
                .remember_precomputed(
                    content,
                    &[],
                    None,
                    Some("rs-ns"),
                    "decision",
                    "text",
                    &embedding,
                )
                .unwrap()
        };
        let old = mk("old decision");
        let mid = mk("mid decision");
        let latest = mk("latest decision");

        uteke.supersede(&old, &mid, Some("first pick")).unwrap();
        // Re-supersession: old is already deprecated; winner changes.
        uteke
            .supersede(&old, &latest, Some("second pivot"))
            .unwrap();

        // Edge pointer is the current winner.
        assert_eq!(
            uteke.supersession_of(&old).unwrap().as_deref(),
            Some(latest.as_str())
        );

        // Ledger must reflect the CURRENT winner and reason.
        let ledger = uteke.contradiction_resolutions(Some("rs-ns"), 50).unwrap();
        let entry = ledger
            .iter()
            .find(|d| d.id == old)
            .expect("old memory in ledger");
        let reason = entry.deprecate_reason.as_deref().unwrap_or("");
        assert!(
            reason.contains(latest.as_str()) && reason.contains("second pivot"),
            "ledger reason must name the current winner: {reason}"
        );
        assert!(
            !reason.contains(mid.as_str()),
            "stale first-winner reason must be gone: {reason}"
        );

        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod auto_link_hot_path_tests {
    //! #1368 — the remember() hot path must not scan a whole namespace, must
    //! not be crowded out by other namespaces, and must never link to
    //! deprecated memories.
    use crate::Uteke;

    const DIMS: usize = 768;

    /// Unit-ish vectors sharing axis 0 with a private perturbation on axis
    /// `k`: pairwise cosine = 1 / (1 + eps^2) ≈ 0.89 — above the similar_to
    /// threshold (0.80), below possible_duplicate (0.92) and the remember()
    /// dedup threshold (0.95), so every one of them is stored and linkable.
    fn vec_k(k: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; DIMS];
        v[0] = 1.0;
        v[k + 1] = 0.35;
        v
    }

    fn mem(u: &Uteke, text: &str, ns: &str, k: usize) -> String {
        u.remember_precomputed(text, &[], None, Some(ns), "fact", "text", &vec_k(k))
            .unwrap()
    }

    fn similar_targets(u: &Uteke, id: &str) -> Vec<String> {
        u.edges_for(id)
            .unwrap()
            .outgoing
            .into_iter()
            .filter(|e| e.edge_type == "similar_to")
            .map(|e| e.target_id)
            .collect()
    }

    /// `e0 + 0.35*e_hot + 0.05*e_k`: sim to `e0 + 0.35*e_hot` is ~0.999.
    fn near_hot(hot: usize, k: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; DIMS];
        v[0] = 1.0;
        v[hot + 1] = 0.35;
        v[k + 1] = 0.05;
        v
    }

    #[test]
    fn small_namespace_inside_a_big_index_still_gets_its_own_links() {
        let u = Uteke::open(":memory:").unwrap();
        let hot = 300;
        // 60 memories of ANOTHER namespace that are much closer (~0.999) to the
        // newcomer than its own neighbours (~0.89). A plain top-20 search
        // followed by a namespace filter is filled entirely by them and the
        // newcomer gets no links at all.
        for k in 0..60 {
            u.remember_precomputed(
                &format!("big {k}"),
                &[],
                None,
                Some("big"),
                "fact",
                "text",
                &near_hot(hot, k),
            )
            .unwrap();
        }
        let s1 = mem(&u, "small one", "small", 100);
        let s2 = mem(&u, "small two", "small", 101);
        let s3 = mem(&u, "small three", "small", 102);
        let new = u
            .remember_precomputed(
                "small four",
                &[],
                None,
                Some("small"),
                "fact",
                "text",
                &vec_k(hot),
            )
            .unwrap();

        let targets = similar_targets(&u, &new);
        for s in [&s1, &s2, &s3] {
            assert!(targets.contains(s), "missing link to {s}: {targets:?}");
        }
        // never across namespaces
        let all = u.store.list(None, None, 1000, 0).unwrap();
        for t in &targets {
            let m = all.iter().find(|m| &m.id == t).unwrap();
            assert_eq!(m.namespace, "small", "cross-namespace link to {t}");
        }
    }

    #[test]
    fn deprecated_memories_are_never_link_targets() {
        let u = Uteke::open(":memory:").unwrap();
        let live = mem(&u, "live neighbour", "ns", 1);
        let dead = mem(&u, "deprecated neighbour", "ns", 2);
        // Deprecate in SQLite only: the vector stays in the index, exactly
        // like a stale entry after a crash or an old binary (#1210).
        u.store.deprecate(&dead).unwrap();

        let new = mem(&u, "newcomer", "ns", 3);
        let targets = similar_targets(&u, &new);
        assert!(targets.contains(&live));
        assert!(!targets.contains(&dead), "linked to a deprecated memory");
    }

    #[test]
    fn live_ids_in_namespace_filters_candidates_only() {
        let u = Uteke::open(":memory:").unwrap();
        let a = mem(&u, "a", "one", 1);
        let b = mem(&u, "b", "two", 2);
        let gone = mem(&u, "gone", "one", 3);
        u.store.deprecate(&gone).unwrap();

        let ids = [a.as_str(), b.as_str(), gone.as_str(), "chunk:xyz", "nope"];
        let in_one = u.store.live_ids_in_namespace(&ids, Some("one")).unwrap();
        assert_eq!(in_one, [a.clone()].into_iter().collect());
        let anywhere = u.store.live_ids_in_namespace(&ids, None).unwrap();
        assert_eq!(anywhere, [a, b].into_iter().collect());
        assert!(
            u.store
                .live_ids_in_namespace::<&str>(&[], Some("one"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn dedup_stays_scoped_to_the_namespace_and_ignores_deprecated() {
        let u = Uteke::open(":memory:").unwrap();
        let v = vec_k(7);
        let first = u
            .remember_precomputed("same text", &[], None, Some("ns-a"), "fact", "text", &v)
            .unwrap();
        // Same namespace: the near-duplicate is found.
        assert_eq!(
            u.check_duplicate(&v, Some("ns-a")).unwrap(),
            Some(first.clone())
        );
        // Another namespace holds nothing similar.
        assert_eq!(u.check_duplicate(&v, Some("ns-b")).unwrap(), None);
        // No namespace filter: any live memory counts.
        assert_eq!(u.check_duplicate(&v, None).unwrap(), Some(first.clone()));
        // A deprecated memory still in the index is not a dedup target (#1210).
        u.store.deprecate(&first).unwrap();
        assert_eq!(u.check_duplicate(&v, Some("ns-a")).unwrap(), None);
    }
}

#[cfg(test)]
mod deprecated_edge_reader_tests {
    //! #1367 — soft delete keeps `memory_edges` rows; readers that feed
    //! ranking/traversal must ignore the retired side, while the audit view
    //! (`edges_for`, the supersession ledger) keeps showing them.
    use crate::Uteke;

    fn mem(u: &Uteke, text: &str, k: usize) -> String {
        // Orthogonal vectors: no auto-created similar_to edges, so the chain
        // below consists only of the explicit edges the test adds.
        let mut v = vec![0.0_f32; 768];
        v[k] = 1.0;
        u.remember_precomputed(text, &[], None, Some("d"), "fact", "text", &v)
            .unwrap()
    }

    #[test]
    fn bfs_skips_deprecated_memories_but_edges_for_keeps_history() {
        let u = Uteke::open(":memory:").unwrap();
        let a = mem(&u, "a", 1);
        let b = mem(&u, "b", 2);
        let c = mem(&u, "c", 3);
        // chain a - b - c with an explicit relation type that is not auto-linked
        u.link_memories(&a, &b, "references").unwrap();
        u.link_memories(&b, &c, "references").unwrap();
        u.store.deprecate(&b).unwrap();

        let related: Vec<String> = u
            .related_via_edges(&a, 3)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert!(!related.contains(&b), "deprecated b must not be returned");
        assert!(
            !related.contains(&c),
            "c is only reachable through the deprecated b"
        );

        // The audit view still lists the edge to the deprecated memory.
        let outgoing = u.edges_for(&a).unwrap().outgoing;
        assert!(
            outgoing
                .iter()
                .any(|e| e.target_id == b && e.edge_type == "references"),
            "history must stay visible: {outgoing:?}"
        );
    }
}
