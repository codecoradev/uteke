//! API route registry — single source of truth for endpoint metadata.
//!
//! Used by `crates/docgen` to auto-generate `docs/api-reference.md`.
//!
//! Every endpoint exposed by `uteke-serve` should be listed here.
//! If a handler exists but isn't registered here, the CI doc-generation
//! step will fail (count mismatch).

#![allow(dead_code)]

#[cfg(feature = "docgen")]
use schemars::JsonSchema;
use serde::Serialize;

/// Metadata for a single API endpoint.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "docgen", derive(JsonSchema))]
pub struct Endpoint {
    /// HTTP method (GET, POST, PUT, DELETE).
    pub method: &'static str,
    /// URL path (e.g., "/remember").
    pub path: &'static str,
    /// Short description of what the endpoint does.
    pub description: &'static str,
    /// Name of the request body type, if any (e.g., "RememberRequest").
    /// Null for GET endpoints or endpoints with no body.
    pub request_type: Option<&'static str>,
    /// Name of the primary response type, if structured (e.g., "Memory").
    pub response_type: Option<&'static str>,
    /// Whether the endpoint filters out deprecated memories.
    pub excludes_deprecated: bool,
    /// Related issue numbers for context.
    pub issues: &'static [&'static str],
}

/// Complete API route registry.
/// This must be kept in sync with `handlers.rs`.
pub const ENDPOINTS: &[Endpoint] = &[
    // ── Health & Info ────────────────────────────────────────────────────
    Endpoint {
        method: "GET",
        path: "/health",
        description: "Health check — returns server status and version",
        request_type: None,
        response_type: Some("HealthResponse"),
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "GET",
        path: "/guide",
        description: "Returns the agent-facing memory tools guide for system prompt injection (#1010).",
        request_type: None,
        response_type: Some("GuideResponse"),
        excludes_deprecated: false,
        issues: &["#1010"],
    },
    Endpoint {
        method: "GET",
        path: "/namespaces",
        description: "List all namespaces in the memory store. `?with_counts=true` adds `count` (total) plus `active`/`deprecated` breakdown fields (#1181).",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["1181"],
    },
    Endpoint {
        method: "POST",
        path: "/namespaces/rename",
        description: "Rename a namespace (`{from, to}`). When `to` already exists this is a merge; the old name vanishes (derived view). Returns `{from, to, moved, target_existed}` (#1181).",
        request_type: Some("NamespaceRenameRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1181"],
    },
    Endpoint {
        method: "POST",
        path: "/namespaces/delete",
        description: "Delete a namespace with an explicit strategy for its memories (`{name, strategy, target?}`): `refuse` (default — 409 while any memory references the name), `merge` (move all memories to `target`), or `deprecate` (soft-delete — restorable, never hard-deleted) (#1181).",
        request_type: Some("NamespaceDeleteRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1181"],
    },
    Endpoint {
        method: "GET",
        path: "/stats",
        description: "Get memory statistics (count, etc.) for a namespace. Accepts `?namespace=X` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/stats",
        description: "Get memory statistics via POST body. Accepts `{\"namespace\": \"...\"}`.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#786"],
    },
    // ── Core Memory Operations ───────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/remember",
        description: "Store a new memory. Accepts content, tags, namespace, type, metadata.",
        request_type: Some("RememberRequest"),
        response_type: Some("Memory"),
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/recall",
        description: "Semantic search — recall memories by meaning. Returns ranked results. Set `explain: true` (#1160) to include per-result ranking signals (vector similarity/rank, RRF contributions, boosts); memory-only recall.",
        request_type: Some("RecallRequest"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/search",
        description: "Keyword search — find memories by matching words in content/tags.",
        request_type: Some("SearchRequest"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/list",
        description: "List memories with optional filters (namespace, tags, sort, limit, offset). Set `include_meta: true` (#1188) for a pagination envelope `{memories, total, has_more, next_offset}` instead of the bare array (not supported with `at`).",
        request_type: Some("ListParams"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "DELETE",
        path: "/forget",
        description: "Deprecate a memory by ID. Returns 404 if ID doesn't exist.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "GET",
        path: "/memory",
        description: "Get a single memory by ID. Accepts `?id=...` query param.",
        request_type: None,
        response_type: Some("Memory"),
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "PUT",
        path: "/memory",
        description: "Update an existing memory's content and/or metadata.",
        request_type: Some("MemoryUpdateRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/memory/pin",
        description: "Pin a memory so it won't be removed by aging/cleanup operations.",
        request_type: Some("MemoryPinRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/memory/importance",
        description: "Get or set the importance score of a memory.",
        request_type: Some("MemoryImportanceRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/memory/feedback",
        description: "Submit positive/negative feedback on a memory for ranking signals.",
        request_type: Some("MemoryFeedbackRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/memory/doc-refs",
        description: "Get documents that reference a specific memory.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Tags ─────────────────────────────────────────────────────────────
    Endpoint {
        method: "GET",
        path: "/tags",
        description: "List all tags in a namespace. Accepts `?namespace=X` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/tags/rename",
        description: "Rename a tag across all memories.",
        request_type: Some("TagRenameRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/tags/delete",
        description: "Delete a tag from all memories.",
        request_type: Some("TagDeleteRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Pin/Unpin (legacy) ─────────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/pin",
        description: "Pin a memory by ID (legacy — prefer /memory/pin).",
        request_type: Some("PinRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/unpin",
        description: "Unpin a memory by ID (legacy — prefer /memory/pin with pin=false).",
        request_type: Some("PinRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Rooms ────────────────────────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/room/create",
        description: "Create a new memory room. Accepts `{\"name\": \"...\"}`.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/remember",
        description: "Store a memory linked to a room. Accepts room_id, content, tags, type, author.",
        request_type: Some("RoomRememberRequest"),
        response_type: Some("Memory"),
        excludes_deprecated: false,
        issues: &["#789"],
    },
    Endpoint {
        method: "POST",
        path: "/room/recall",
        description: "Semantic search within a room. Empty query returns all memories chronologically.",
        request_type: Some("RoomRecallRequest"),
        response_type: None,
        excludes_deprecated: true,
        issues: &["#785"],
    },
    Endpoint {
        method: "POST",
        path: "/room/summary",
        description: "Get room summary with memory clusters and statistics.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/summary-document",
        description: "Get room summary focused on document-type memories.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "GET",
        path: "/room/list",
        description: "List all rooms.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/consolidate",
        description: "Plan or execute segment-level LLM consolidation of room memories (#1088). Dry-run by default; `apply: true` executes with a hard budget cap. Write op — blocked for read-only tokens.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1088"],
    },
    Endpoint {
        method: "POST",
        path: "/room/stats",
        description: "Get memory count for a room. Includes deprecated memories (known discrepancy vs /room/summary).",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#784"],
    },
    Endpoint {
        method: "GET",
        path: "/room/memories",
        description: "List all memories in a room (chronological). Accepts `?room_id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "DELETE",
        path: "/room/delete",
        description: "Delete a room (unlink-only): room links are removed, memories and documents are preserved. Response: `{ deleted, unlinked_memories }`. Accepts `?room_id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/rename",
        description: "Rename a room: the registry row and every member/document link move in one transaction; title, description, and namespace are preserved (#1202). Accepts `{ from, to }`. Write op — blocked for read-only tokens.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1202"],
    },
    Endpoint {
        method: "POST",
        path: "/room/update",
        description: "Update a room's title and/or description; omitted fields stay unchanged (#1202). Accepts `{ room_id, title?, description? }`; 404 when the room is missing. Write op — blocked for read-only tokens.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1202"],
    },
    Endpoint {
        method: "POST",
        path: "/room/memory/move",
        description: "Move a memory from one room to another, preserving the link's author/role/joined_at provenance (#1202). Accepts `{ memory_id, from_room, to_room }`; 404 when the memory has no link in `from_room`. Write op — blocked for read-only tokens.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1202"],
    },
    // ── Room Documents ──────────────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/room/document",
        description: "Store a reference document in a room (large content >500 chars).",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/document/list",
        description: "List documents in a room.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "PUT",
        path: "/room/document/add",
        description: "Add a reference to an existing document in a room.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "DELETE",
        path: "/room/document/remove",
        description: "Remove a document reference from a room.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/room/list",
        description: "List rooms that reference a specific document.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Documents ───────────────────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/doc/create",
        description: "Create a new document with slug, title, content, tags.",
        request_type: Some("DocCreateRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/get",
        description: "Get a document by slug.",
        request_type: Some("DocGetRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/list",
        description: "List documents with optional namespace/limit/roots_only/parent filters.",
        request_type: Some("DocListParams"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/search",
        description: "Search documents by query with optional mode/namespace/limit.",
        request_type: Some("DocSearchRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/update",
        description: "Update an existing document (content, title, tags, parent).",
        request_type: Some("DocUpdateRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/move",
        description: "Move a document to a different parent.",
        request_type: Some("DocMoveRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "DELETE",
        path: "/doc/delete",
        description: "Delete a document by slug or ID.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/mem-refs",
        description: "Get memories that reference a specific document.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Graph ───────────────────────────────────────────────────────────
    Endpoint {
        method: "GET",
        path: "/graph",
        description: "Get graph edges for a memory. Accepts `?id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/graph/edge",
        description: "Add a directed edge between two memories. Accepts memory IDs (a linked graph node is ensured automatically, #1180) or existing graph node IDs. Returns `{ok, source_node, target_node}`.",
        request_type: Some("GraphEdgeRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1180"],
    },
    Endpoint {
        method: "DELETE",
        path: "/graph/edge",
        description: "Remove an edge between two nodes. Accepts memory IDs or graph node IDs via `?source=...&target=...` query params (#1180).",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["1180"],
    },
    Endpoint {
        method: "GET",
        path: "/edges",
        description: "List edges for a memory (alias for /graph). Accepts `?id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Timeline ─────────────────────────────────────────────────────────
    Endpoint {
        method: "GET",
        path: "/provenance",
        description: "Full provenance report for a memory (#1172): author/source fields, trust tier, source hash at write vs live-recomputed content hash (tamper evidence), and the full timeline event chain with actor + evidence. Accepts `?id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["1172"],
    },
    Endpoint {
        method: "GET",
        path: "/contradictions",
        description: "List superseded-but-not-restored memories (#1172) — the auditable contradiction resolution ledger (deprecated rows carrying a live superseded_by edge). Accepts `?namespace=...&limit=...`.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["1172"],
    },
    Endpoint {
        method: "POST",
        path: "/contradictions/undo",
        description: "Undo a contradiction resolution (#1172): restore the retired memory to active, remove the supersession pair, and record supersession_undone provenance events. Body: `{id}`.",
        request_type: Some("ContradictionUndoRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1172"],
    },
    Endpoint {
        method: "GET",
        path: "/timeline",
        description: "Get timeline of memory events for a memory. Accepts `?id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Import/Export ────────────────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/import",
        description: "Import memories from a JSON array.",
        request_type: Some("ImportRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "GET",
        path: "/export",
        description: "Export all memories as JSON. Accepts `?namespace=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Maintenance ──────────────────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/prune",
        description: "Remove orphaned memories (no room, no graph edges).",
        request_type: Some("PruneRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/consolidate",
        description: "Merge similar/duplicate memories automatically.",
        request_type: Some("ConsolidateRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/consolidate/pair",
        description: "Consolidate a single caller-chosen duplicate pair: keep id_keep, deprecate (or hard-delete) id_remove.",
        request_type: Some("ConsolidatePairRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1076"],
    },
    Endpoint {
        method: "POST",
        path: "/aging",
        description: "Run aging cleanup — deprioritize or remove old/stale memories.",
        request_type: Some("AgingRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/importance",
        description: "Recompute importance scores for all memories.",
        request_type: Some("ImportanceRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/orphans",
        description: "List orphaned memories (not in any room, no edges).",
        request_type: Some("OrphansRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/extract",
        description: "Extract entities and relationships from memory content.",
        request_type: Some("ExtractRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/rebuild-backlinks",
        description: "Rebuild backlink indices for memory graph.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Context / Dream / MCP ───────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/context",
        description: "Get context window for a query (for LLM prompt enrichment).",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/dream",
        description: "Generate new memories/insights from existing memory corpus.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/mcp",
        description: "MCP (Model Context Protocol) bridge endpoint for AI agent tool calls.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Recent ───────────────────────────────────────────────────────────
    Endpoint {
        method: "GET",
        path: "/recent",
        description: "Get recently added memories. Accepts `?limit=N&namespace=X` query params.",
        request_type: None,
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    // ── Lifecycle ───────────────────────────────────────────────────────
    Endpoint {
        method: "POST",
        path: "/lifecycle/cycle",
        description: "Run lifecycle aging cycle: deprecate old memories, optionally prune expired ones.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#935"],
    },
    Endpoint {
        method: "POST",
        path: "/lifecycle/promote",
        description: "Restore a deprecated memory back to active status.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#935"],
    },
    Endpoint {
        method: "GET",
        path: "/lifecycle/status",
        description: "Get lifecycle status: active/deprecated counts and current configuration.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#935"],
    },
    Endpoint {
        method: "GET",
        path: "/lifecycle/deprecated",
        description: "List deprecated memories with TTL metadata.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1007"],
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn registry_covers_handler_routes() {
        let handler_source = include_str!("handlers.rs");
        let handler_paths = handler_source
            .lines()
            .filter_map(|line| {
                let method_start = line.find("(Method::")?;
                let path_start = line[method_start..].find(", \"")? + method_start + 3;
                let path_end = line[path_start..].find('"')? + path_start;
                Some(&line[path_start..path_end])
            })
            .collect::<BTreeSet<_>>();
        let registered_paths = ENDPOINTS
            .iter()
            .map(|endpoint| endpoint.path)
            .collect::<BTreeSet<_>>();

        let missing = handler_paths
            .difference(&registered_paths)
            .copied()
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "Handler routes missing from API registry: {missing:?}"
        );

        assert!(!ENDPOINTS.is_empty(), "Endpoint registry must not be empty");
        let paths = registered_paths;
        assert!(paths.contains(&"/health"), "Missing /health");
        assert!(paths.contains(&"/remember"), "Missing /remember");
        assert!(paths.contains(&"/recall"), "Missing /recall");
        assert!(paths.contains(&"/room/remember"), "Missing /room/remember");
        assert!(paths.contains(&"/room/recall"), "Missing /room/recall");
    }
}
