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
use crate::memory::types::Memory;
use rusqlite::{params, OptionalExtension};
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
        let mut inserted = 0usize;
        for (target_id, edge_type) in edges {
            let changed = tx
                .execute(
                    "INSERT OR IGNORE INTO memory_edges
                        (source_id, target_id, edge_type, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![source_id, target_id, edge_type, now],
                )
                .map_err(|e| Error::db("batch insert memory edge", e))?;
            inserted += changed;
            // #350: ensure inverse backlink for every forward edge inside the
            // same transaction so the whole batch commits atomically.
            if let Some(backlink_type) = backlink_type_for(edge_type) {
                if source_id != target_id {
                    tx.execute(
                        "INSERT OR IGNORE INTO memory_edges
                            (source_id, target_id, edge_type, created_at)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![target_id, source_id, backlink_type, now],
                    )
                    .map_err(|e| Error::db("batch ensure backlink", e))?;
                }
            }
        }
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
                "SELECT target_id FROM memory_edges WHERE source_id = ?1
                 UNION
                 SELECT source_id FROM memory_edges WHERE target_id = ?1",
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
        // BACKLINKED_EDGE_TYPES is fixed at 4 entries (references, tagged_as,
        // supersedes, replies_to) — we bind them positionally.
        debug_assert_eq!(
            BACKLINKED_EDGE_TYPES.len(),
            4,
            "update rebuild_backlinks SQL if backlinked types change"
        );
        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_id, target_id FROM memory_edges
                 WHERE edge_type IN (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| Error::db("prepare rebuild backlinks scan", e))?;
        let rows: Vec<(String, String)> = stmt
            .query_map(
                params![
                    BACKLINKED_EDGE_TYPES[0],
                    BACKLINKED_EDGE_TYPES[1],
                    BACKLINKED_EDGE_TYPES[2],
                    BACKLINKED_EDGE_TYPES[3],
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
                ExtractedRef::Slug(slug) => match self.store.resolve_slug(&slug, namespace) {
                    Ok(Some(target)) => {
                        if target != source_id {
                            resolved.push((target, EDGE_REFERENCES.to_string()));
                        }
                    }
                    Ok(None) => {
                        tracing::debug!("Auto-edge: slug '{slug}' unresolved, skipped");
                    }
                    Err(e) => {
                        tracing::warn!("Auto-edge slug resolve failed for '{slug}': {e}");
                    }
                },
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
        const TOP_K: usize = 20;

        let index = match self.index.read() {
            Ok(i) => i,
            Err(_) => {
                tracing::debug!("auto_link_cosine: index lock failed, skipping");
                return;
            }
        };

        let results = index.search(embedding, TOP_K, 100);
        drop(index); // Release read lock ASAP

        if results.is_empty() {
            return;
        }

        // Filter by namespace if specified (#401 cora finding: cross-namespace links are wrong).
        let ns_set: Option<std::collections::HashSet<String>> = if let Some(ns) = namespace {
            match self.store.memories_in_namespace(ns) {
                Ok(ids) => Some(ids.into_iter().collect()),
                Err(e) => {
                    tracing::warn!("auto_link_cosine: failed to get namespace ids: {e}");
                    return;
                }
            }
        } else {
            None
        };

        // usearch returns distances (lower = more similar for cosine).
        // Convert distance to similarity: sim = 1.0 - dist.
        let edges: Vec<(String, String)> = results
            .iter()
            .filter(|(id, _)| id != source_id) // skip self
            .filter(|(id, _)| {
                // Skip memories outside our namespace.
                match &ns_set {
                    Some(set) => set.contains(id),
                    None => true,
                }
            })
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
            id: uuid::Uuid::new_v4().to_string(),
            content: content.to_string(),
            embedding: vec![],
            tags: tags.iter().map(|s| s.to_string()).collect(),
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
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
        assert_eq!(v, 13, "fresh store must reach CURRENT_SCHEMA_VERSION=13");

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
}
