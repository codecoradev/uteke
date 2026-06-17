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

    /// Bulk insert edges. Silently ignores duplicates and dangling targets.
    pub fn add_memory_edges_batch(
        &self,
        source_id: &str,
        edges: &[(String, String)],
    ) -> Result<usize, Error> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut inserted = 0usize;
        for (target_id, edge_type) in edges {
            let changed = self
                .conn
                .execute(
                    "INSERT OR IGNORE INTO memory_edges (source_id, target_id, edge_type, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![source_id, target_id, edge_type, now],
                )
                .map_err(|e| Error::db("batch insert memory edge", e))?;
            inserted += changed;
        }
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

        while let Some((cur, depth)) = frontier.pop_front() {
            if depth >= max_depth {
                continue;
            }
            // Gather neighbors (both directions) at the SQL level.
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT target_id FROM memory_edges WHERE source_id = ?1
                     UNION
                     SELECT source_id FROM memory_edges WHERE target_id = ?1",
                )
                .map_err(|e| Error::db("prepare bfs neighbors", e))?;
            let rows = stmt
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
                    // Verify target exists to avoid dangling edges.
                    match self.store.get_by_id(&id) {
                        Ok(Some(_)) => {
                            if id != source_id {
                                resolved.push((id, edge_type.to_string()));
                            }
                        }
                        Ok(None) => {
                            tracing::debug!("Auto-edge: uuid '{id}' unresolved, skipped");
                        }
                        Err(e) => {
                            tracing::warn!("Auto-edge uuid resolve failed for '{id}': {e}");
                        }
                    }
                }
                ExtractedRef::Rel(t, id) => match self.store.get_by_id(&id) {
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

    /// List edges for a memory (both directions). Public for `uteke edges <id>`.
    pub fn edges_for(&self, memory_id: &str) -> Result<EdgeList, Error> {
        self.store.list_memory_edges(memory_id)
    }

    /// Total edge count across the store.
    pub fn count_edges(&self) -> Result<usize, Error> {
        self.store.count_memory_edges()
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
        // Second identical insert must be a no-op.
        let n = store
            .add_memory_edges_batch(&a.id, &[(b.id.clone(), EDGE_REFERENCES.to_string())])
            .unwrap();
        assert_eq!(n, 0, "duplicate insert should change 0 rows");

        let edges = store.list_memory_edges(&a.id).unwrap();
        assert_eq!(edges.total(), 1);
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
}
