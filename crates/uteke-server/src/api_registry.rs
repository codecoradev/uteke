//! API route registry — single source of truth for endpoint metadata.
//!
//! Used by `crates/docgen` to auto-generate `docs/api-reference.md`, served
//! by `GET /routes`, and consulted by `handlers::route` (#1344) for both
//! reachability and the read-only-token gate: a (method, path) without a row
//! here is answered 404 before any handler runs, so a handler cannot exist
//! unregistered, and a new write route is gated by giving its row
//! `writes: true`.

#![allow(dead_code)]

#[cfg(feature = "docgen")]
use schemars::JsonSchema;
use serde::Serialize;

/// Contract tier for an endpoint (uteke-cloud #81 / contract doc `kontrak-core-cloud`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "docgen", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// CORE — semantic compatibility with uteke-cloud is guaranteed.
    /// Changes require a contract issue on uteke-cloud FIRST.
    Core,
    /// LAB — free to evolve; promoted to CORE only with a cloud
    /// implementation plus an explicit owner order.
    Lab,
}

/// Workspace version of this registry crate — echoed into
/// `docs/core-contract.json` by docgen.
pub const REGISTRY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Metadata for a single API endpoint.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "docgen", derive(JsonSchema))]
pub struct Endpoint {
    /// HTTP method (GET, POST, PUT, DELETE).
    pub method: &'static str,
    /// URL path (e.g., "/remember").
    pub path: &'static str,
    /// Contract tier — CORE (compat-guaranteed with uteke-cloud) or LAB.
    pub tier: Tier,
    /// Whether the endpoint mutates state. This single flag drives the
    /// read-only-token gate in `handlers::route` (#1344): a POST row with
    /// `writes = false` is reachable by a read-only token, every other
    /// non-GET request is refused with 403. It is routing metadata, not wire
    /// contract, so it stays out of `GET /routes` and the generated docs.
    #[serde(skip)]
    pub writes: bool,
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
/// `handlers::HANDLERS` (bin side) must list the same (method, path) rows; the
/// `handler_table_equals_the_registry` test fails when the two drift.
pub const ENDPOINTS: &[Endpoint] = &[
    // ── Health & Info ────────────────────────────────────────────────────
    Endpoint {
        method: "GET",
        path: "/health",
        tier: Tier::Core,
        writes: false,
        description: "Health check — tokenless requests get {status} only; a valid bearer token (or auth-disabled server) returns status, version, memory counts, and update info",
        request_type: None,
        response_type: Some("HealthResponse"),
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "GET",
        path: "/guide",
        tier: Tier::Lab,
        writes: false,
        description: "Returns the agent-facing memory tools guide for system prompt injection (#1010).",
        request_type: None,
        response_type: Some("GuideResponse"),
        excludes_deprecated: false,
        issues: &["#1010"],
    },
    Endpoint {
        method: "GET",
        path: "/routes",
        tier: Tier::Lab,
        writes: false,
        description: "Machine-readable API route introspection: returns this registry (method, path, tier, description, request/response types) as JSON — the same source of truth that generates docs/api-reference.md (#1289).",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1289"],
    },
    Endpoint {
        method: "GET",
        path: "/namespaces",
        tier: Tier::Core,
        writes: false,
        description: "List all namespaces in the memory store. `?with_counts=true` adds `count` (total) plus `active`/`deprecated` breakdown fields (#1181).",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["1181"],
    },
    Endpoint {
        method: "POST",
        path: "/namespaces/rename",
        tier: Tier::Core,
        writes: true,
        description: "Rename a namespace (`{from, to}`). When `to` already exists this is a merge; the old name vanishes (derived view). Returns `{from, to, moved, target_existed}` (#1181).",
        request_type: Some("NamespaceRenameRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1181"],
    },
    Endpoint {
        method: "POST",
        path: "/namespaces/delete",
        tier: Tier::Core,
        writes: true,
        description: "Delete a namespace with an explicit strategy for its memories (`{name, strategy, target?}`): `refuse` (default — 409 while any memory references the name), `merge` (move all memories to `target`), or `deprecate` (soft-delete — restorable, never hard-deleted) (#1181).",
        request_type: Some("NamespaceDeleteRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1181"],
    },
    Endpoint {
        method: "GET",
        path: "/stats",
        tier: Tier::Lab,
        writes: false,
        description: "Get memory statistics (count, etc.) for a namespace. Accepts `?namespace=X` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/stats",
        tier: Tier::Lab,
        writes: false,
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
        tier: Tier::Core,
        writes: true,
        description: "Store a new memory. Accepts content, tags, namespace, type, metadata. Explicit `type` is honored as-is (no silent normalization, #1302); unknown types are rejected with 400 listing the valid vocabulary. Omit `type` to auto-infer.",
        request_type: Some("RememberRequest"),
        response_type: Some("Memory"),
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/recall",
        tier: Tier::Core,
        writes: false,
        description: "Semantic search — recall memories by meaning. Returns ranked results. Set `explain: true` (#1160) to include per-result ranking signals (vector similarity/rank, RRF contributions, boosts); memory-only recall.",
        request_type: Some("RecallRequest"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/search",
        tier: Tier::Core,
        writes: false,
        description: "Keyword search — find memories by matching words in content/tags.",
        request_type: Some("SearchRequest"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/list",
        tier: Tier::Lab,
        writes: false,
        description: "List memories with optional filters (namespace, tags, sort, limit, offset). Set `include_meta: true` (#1188) for a pagination envelope `{memories, total, has_more, next_offset}` instead of the bare array (not supported with `at`).",
        request_type: Some("ListParams"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "DELETE",
        path: "/forget",
        tier: Tier::Lab,
        writes: true,
        description: "Deprecate a memory by ID. Returns 404 if ID doesn't exist.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "GET",
        path: "/memory",
        tier: Tier::Lab,
        writes: false,
        description: "Get a single memory by ID. Accepts `?id=...` query param.",
        request_type: None,
        response_type: Some("Memory"),
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "PUT",
        path: "/memory",
        tier: Tier::Core,
        writes: true,
        description: "Update an existing memory's content and/or metadata.",
        request_type: Some("MemoryUpdateRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/memory/pin",
        tier: Tier::Core,
        writes: true,
        description: "Pin a memory so it won't be removed by aging/cleanup operations.",
        request_type: Some("MemoryPinRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/memory/importance",
        tier: Tier::Core,
        writes: true,
        description: "Get or set the importance score of a memory.",
        request_type: Some("MemoryImportanceRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/memory/feedback",
        tier: Tier::Core,
        writes: true,
        description: "Submit positive/negative feedback on a memory for ranking signals.",
        request_type: Some("MemoryFeedbackRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/memory/doc-refs",
        tier: Tier::Core,
        writes: false,
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
        tier: Tier::Core,
        writes: false,
        description: "List all tags in a namespace. Accepts `?namespace=X` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/tags/rename",
        tier: Tier::Core,
        writes: true,
        description: "Rename a tag across all memories.",
        request_type: Some("TagRenameRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/tags/delete",
        tier: Tier::Lab,
        writes: true,
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
        tier: Tier::Core,
        writes: true,
        description: "Pin a memory by ID (legacy — prefer /memory/pin).",
        request_type: Some("PinRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/unpin",
        tier: Tier::Core,
        writes: true,
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
        tier: Tier::Core,
        writes: true,
        description: "Create a new memory room. Accepts `{\"name\": \"...\"}`.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/remember",
        tier: Tier::Core,
        writes: true,
        description: "Store a memory linked to a room. Accepts room_id, content, tags, type, author.",
        request_type: Some("RoomRememberRequest"),
        response_type: Some("Memory"),
        excludes_deprecated: false,
        issues: &["#789"],
    },
    Endpoint {
        method: "POST",
        path: "/room/recall",
        tier: Tier::Core,
        writes: false,
        description: "Semantic search within a room. Empty query returns all memories chronologically.",
        request_type: Some("RoomRecallRequest"),
        response_type: None,
        excludes_deprecated: true,
        issues: &["#785"],
    },
    Endpoint {
        method: "POST",
        path: "/room/summary",
        tier: Tier::Lab,
        writes: false,
        description: "Get room summary with memory clusters and statistics.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/summary-document",
        tier: Tier::Lab,
        writes: false,
        description: "Get room summary focused on document-type memories.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "GET",
        path: "/room/list",
        tier: Tier::Core,
        writes: false,
        description: "List all rooms.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/consolidate",
        tier: Tier::Core,
        writes: true,
        description: "Plan or execute segment-level LLM consolidation of room memories (#1088). Dry-run by default; `apply: true` executes with a hard budget cap. Write op — blocked for read-only tokens.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1088"],
    },
    Endpoint {
        method: "POST",
        path: "/room/stats",
        tier: Tier::Core,
        writes: false,
        description: "Get memory count for a room. Includes deprecated memories (known discrepancy vs /room/summary).",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#784"],
    },
    Endpoint {
        method: "GET",
        path: "/room/memories",
        tier: Tier::Core,
        writes: false,
        description: "List all memories in a room (chronological). Accepts `?room_id=...` query param; optional `?author=...` and `?namespace=...` filters (#1288). Without `namespace`, returns ALL namespaces that contributed to the room.",
        request_type: None,
        response_type: None,
        excludes_deprecated: true,
        issues: &[],
    },
    Endpoint {
        method: "DELETE",
        path: "/room/delete",
        tier: Tier::Core,
        writes: true,
        description: "Delete a room (unlink-only): room links are removed, memories and documents are preserved. Response: `{ deleted, unlinked_memories }`. Accepts `?room_id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/rename",
        tier: Tier::Core,
        writes: true,
        description: "Rename a room: the registry row and every member/document link move in one transaction; title, description, and namespace are preserved (#1202). Accepts `{ from, to }`. Write op — blocked for read-only tokens.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1202"],
    },
    Endpoint {
        method: "POST",
        path: "/room/update",
        tier: Tier::Core,
        writes: true,
        description: "Update a room's title and/or description; omitted fields stay unchanged (#1202). Accepts `{ room_id, title?, description? }`; 404 when the room is missing. Write op — blocked for read-only tokens.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1202"],
    },
    Endpoint {
        method: "POST",
        path: "/room/memory/move",
        tier: Tier::Core,
        writes: true,
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
        tier: Tier::Lab,
        writes: true,
        description: "Store a reference document in a room (large content >500 chars).",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/room/document/list",
        tier: Tier::Lab,
        writes: false,
        description: "List documents in a room.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "PUT",
        path: "/room/document/add",
        tier: Tier::Lab,
        writes: true,
        description: "Add a reference to an existing document in a room.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "DELETE",
        path: "/room/document/remove",
        tier: Tier::Lab,
        writes: true,
        description: "Remove a document reference from a room.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/room/list",
        tier: Tier::Lab,
        writes: false,
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
        tier: Tier::Lab,
        writes: true,
        description: "Create a new document with slug, title, content, tags.",
        request_type: Some("DocCreateRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/get",
        tier: Tier::Lab,
        writes: false,
        description: "Get a document by slug.",
        request_type: Some("DocGetRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/verify",
        tier: Tier::Lab,
        writes: true,
        description: "Check vector-index/SQLite consistency: row vs vector counts, mismatch flag (#1266).",
        request_type: None,
        response_type: Some("VerifyReport"),
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/repair",
        tier: Tier::Lab,
        writes: true,
        description: "Rebuild the vector index from stored embeddings to fix recall desync (#1266). SQLite data is untouched.",
        request_type: None,
        response_type: Some("RepairReport"),
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/list",
        tier: Tier::Lab,
        writes: false,
        description: "List documents with optional namespace/limit/roots_only/parent filters.",
        request_type: Some("DocListParams"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/search",
        tier: Tier::Lab,
        writes: false,
        description: "Search documents by query with optional mode/namespace/limit.",
        request_type: Some("DocSearchRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/update",
        tier: Tier::Lab,
        writes: true,
        description: "Update an existing document (content, title, tags, parent).",
        request_type: Some("DocUpdateRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/move",
        tier: Tier::Lab,
        writes: true,
        description: "Move a document to a different parent.",
        request_type: Some("DocMoveRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "DELETE",
        path: "/doc/delete",
        tier: Tier::Lab,
        writes: true,
        description: "Delete a document by slug or ID.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/doc/mem-refs",
        tier: Tier::Lab,
        writes: false,
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
        tier: Tier::Lab,
        writes: false,
        description: "Memory graph for visualisation: live memories as nodes, `memory_edges` (auto-links, supersession) plus explicit graph edges as edges, keyed by memory id. Accepts `?namespace=...`, `?node_id=<memory id>` (that memory's neighbourhood) and `?limit=` (edges, default 500, max 5000); response carries `truncated`. Derived `referenced_by` backlinks are omitted.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/graph/edge",
        tier: Tier::Lab,
        writes: true,
        description: "Add a directed edge between two memories. Accepts memory IDs (a linked graph node is ensured automatically, #1180) or existing graph node IDs. Returns `{ok, source_node, target_node}`.",
        request_type: Some("GraphEdgeRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1180"],
    },
    Endpoint {
        method: "DELETE",
        path: "/graph/edge",
        tier: Tier::Lab,
        writes: true,
        description: "Remove an edge between two nodes. Accepts memory IDs or graph node IDs via `?source=...&target=...` query params (#1180).",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["1180"],
    },
    Endpoint {
        method: "GET",
        path: "/edges",
        tier: Tier::Lab,
        writes: false,
        description: "List all `memory_edges` of one memory (outgoing and incoming, including derived backlinks). Not an alias of `/graph`. Accepts `?id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    // ── Timeline ─────────────────────────────────────────────────────────
    Endpoint {
        method: "GET",
        path: "/provenance",
        tier: Tier::Core,
        writes: false,
        description: "Full provenance report for a memory (#1172): author/source fields, trust tier, source hash at write vs live-recomputed content hash (tamper evidence), and the full timeline event chain with actor + evidence. Accepts `?id=...` query param.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["1172"],
    },
    Endpoint {
        method: "GET",
        path: "/contradictions",
        tier: Tier::Core,
        writes: false,
        description: "List superseded-but-not-restored memories (#1172) — the auditable contradiction resolution ledger (deprecated rows carrying a live superseded_by edge). Accepts `?namespace=...&limit=...`.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["1172"],
    },
    Endpoint {
        method: "POST",
        path: "/contradictions/undo",
        tier: Tier::Core,
        writes: true,
        description: "Undo a contradiction resolution (#1172): restore the retired memory to active, remove the supersession pair, and record supersession_undone provenance events. Body: `{id}`.",
        request_type: Some("ContradictionUndoRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1172"],
    },
    Endpoint {
        method: "GET",
        path: "/timeline",
        tier: Tier::Lab,
        writes: false,
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
        tier: Tier::Core,
        writes: true,
        description: "Import memories from a JSON array.",
        request_type: Some("ImportRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "GET",
        path: "/export",
        tier: Tier::Core,
        writes: false,
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
        tier: Tier::Lab,
        writes: true,
        description: "Remove orphaned memories (no room, no graph edges).",
        request_type: Some("PruneRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/consolidate",
        tier: Tier::Lab,
        writes: true,
        description: "Merge similar/duplicate memories automatically.",
        request_type: Some("ConsolidateRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/consolidate/pair",
        tier: Tier::Core,
        writes: true,
        description: "Consolidate a single caller-chosen duplicate pair: keep id_keep, deprecate (or hard-delete) id_remove.",
        request_type: Some("ConsolidatePairRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &["1076"],
    },
    Endpoint {
        method: "POST",
        path: "/aging",
        tier: Tier::Lab,
        writes: true,
        description: "Run aging cleanup — deprioritize or remove old/stale memories.",
        request_type: Some("AgingRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/importance",
        tier: Tier::Lab,
        writes: true,
        description: "Recompute importance scores for all memories.",
        request_type: Some("ImportanceRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/orphans",
        tier: Tier::Lab,
        writes: false,
        description: "List orphaned memories (not in any room, no edges).",
        request_type: Some("OrphansRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/extract",
        tier: Tier::Lab,
        writes: true,
        description: "Extract entities and relationships from memory content.",
        request_type: Some("ExtractRequest"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/rebuild-backlinks",
        tier: Tier::Lab,
        writes: true,
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
        tier: Tier::Lab,
        writes: true,
        description: "Get context window for a query (for LLM prompt enrichment).",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/dream",
        tier: Tier::Lab,
        writes: true,
        description: "Generate new memories/insights from existing memory corpus.",
        request_type: Some("serde_json::Value"),
        response_type: None,
        excludes_deprecated: false,
        issues: &[],
    },
    Endpoint {
        method: "POST",
        path: "/mcp",
        tier: Tier::Lab,
        // Per-tool gate inside uteke-mcp (READ_ONLY_TOOLS), #1439.
        writes: false,
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
        tier: Tier::Lab,
        writes: false,
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
        tier: Tier::Lab,
        writes: true,
        description: "Run lifecycle aging cycle: deprecate old memories, optionally prune expired ones.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#935"],
    },
    Endpoint {
        method: "POST",
        path: "/lifecycle/promote",
        tier: Tier::Lab,
        writes: true,
        description: "Restore a deprecated memory back to active status.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#935"],
    },
    Endpoint {
        method: "GET",
        path: "/lifecycle/status",
        tier: Tier::Lab,
        writes: false,
        description: "Get lifecycle status: active/deprecated counts and current configuration.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#935"],
    },
    Endpoint {
        method: "GET",
        path: "/lifecycle/deprecated",
        tier: Tier::Lab,
        writes: false,
        description: "List deprecated memories with TTL metadata.",
        request_type: None,
        response_type: None,
        excludes_deprecated: false,
        issues: &["#1007"],
    },
];

/// The registry row for a request, or `None` when the router has no such
/// route (the caller answers 404). `method` is the upper-case HTTP verb and
/// `path` excludes the query string.
pub fn find(method: &str, path: &str) -> Option<&'static Endpoint> {
    ENDPOINTS
        .iter()
        .find(|e| e.method == method && e.path == path)
}

/// Whether a read-only token may proceed with this request (#409, #524).
///
/// GET is always allowed. Any other verb is allowed only when `path` (the raw
/// request path, query string included, exactly as the gate always compared
/// it) is the path of a non-writing POST row. The verb is deliberately not
/// matched against that row: a `PUT /list` has always passed the gate and then
/// fallen through to the router's 404, and the gate must keep answering
/// identically.
pub fn read_only_allows(method: &str, path: &str) -> bool {
    method == "GET"
        || ENDPOINTS
            .iter()
            .any(|e| e.method == "POST" && !e.writes && e.path == path)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn rows_are_unique_and_well_formed() {
        let mut seen = BTreeSet::new();
        for e in ENDPOINTS {
            assert!(e.path.starts_with('/'), "{} must start with /", e.path);
            assert!(
                matches!(e.method, "GET" | "POST" | "PUT" | "DELETE"),
                "unexpected method {} on {}",
                e.method,
                e.path
            );
            assert!(
                seen.insert((e.method, e.path)),
                "duplicate row {} {}",
                e.method,
                e.path
            );
        }
        assert!(find("GET", "/health").is_some());
        assert!(find("POST", "/health").is_none());
    }

    /// Reviewed pin: the POST rows a read-only token may call. Flipping a
    /// `writes` flag on a mutating route to `false` (opening it to read-only
    /// tokens) must be a deliberate edit of this list too.
    #[test]
    fn read_only_post_set_is_the_reviewed_one() {
        let got: BTreeSet<&str> = ENDPOINTS
            .iter()
            .filter(|e| e.method == "POST" && !e.writes)
            .map(|e| e.path)
            .collect();
        let want: BTreeSet<&str> = [
            "/list",
            "/recall",
            "/search",
            "/stats",
            "/room/recall",
            "/room/summary",
            "/room/summary-document",
            "/room/stats",
            "/room/document/list",
            "/doc/get",
            "/doc/list",
            "/doc/search",
            "/doc/room/list",
            "/memory/doc-refs",
            "/doc/mem-refs",
            "/orphans",
            "/mcp",
        ]
        .into_iter()
        .collect();
        assert_eq!(got, want);
        // PUT and DELETE rows are never read-only.
        assert!(
            ENDPOINTS
                .iter()
                .filter(|e| e.method == "PUT" || e.method == "DELETE")
                .all(|e| e.writes)
        );
    }

    #[test]
    fn get_rows_never_write() {
        // GET is allowed for every token, so a writing GET row would be a
        // gate bypass by construction.
        for e in ENDPOINTS.iter().filter(|e| e.method == "GET") {
            assert!(!e.writes, "GET {} is flagged as writing", e.path);
        }
    }

    #[test]
    fn read_only_gate_follows_the_writes_flag() {
        for e in ENDPOINTS {
            let allowed = read_only_allows(e.method, e.path);
            let expected = e.method == "GET" || (e.method == "POST" && !e.writes);
            assert_eq!(allowed, expected, "{} {}", e.method, e.path);
            if e.writes {
                assert!(!allowed, "{} {} writes but is allowed", e.method, e.path);
            }
        }
        // A query string defeats the exact-path match, as it always has.
        assert!(!read_only_allows("POST", "/recall?x=1"));
        // Prefix tricks stay blocked.
        assert!(!read_only_allows("POST", "/recallfoo"));
    }

    // #1439: every MCP tool a read-only token may call maps to an HTTP row
    // that does not write. A tool added to READ_ONLY_TOOLS must be mapped
    // here, so the two read-only surfaces cannot drift apart unnoticed. GET rows
    // never write and are always open to read-only tokens, so for them this only
    // proves the route is registered; the POST rows carry the weight.
    #[test]
    fn mcp_read_only_tools_map_to_non_writing_rows() {
        const MAP: &[(&str, &str, &str)] = &[
            ("uteke_recall", "POST", "/recall"),
            ("uteke_search", "POST", "/search"),
            ("uteke_list", "POST", "/list"),
            ("uteke_get", "GET", "/memory"),
            ("uteke_provenance", "GET", "/provenance"),
            ("uteke_timeline", "GET", "/timeline"),
            ("uteke_contradictions", "GET", "/contradictions"),
            ("uteke_stats", "POST", "/stats"),
            ("uteke_doc_get", "POST", "/doc/get"),
            ("uteke_doc_list", "POST", "/doc/list"),
            ("uteke_doc_search", "POST", "/doc/search"),
            ("uteke_doc_list_rooms", "POST", "/doc/room/list"),
            ("uteke_graph", "GET", "/graph"),
            ("uteke_room_list", "GET", "/room/list"),
            ("uteke_room_recall", "POST", "/room/recall"),
            ("uteke_room_memories", "GET", "/room/memories"),
            ("uteke_room_stats", "POST", "/room/stats"),
            ("uteke_room_summary", "POST", "/room/summary"),
            (
                "uteke_room_summary_document",
                "POST",
                "/room/summary-document",
            ),
            // MCP alias of the summary document, not POST /room/document (writes).
            ("uteke_room_document", "POST", "/room/summary-document"),
            ("uteke_room_list_documents", "POST", "/room/document/list"),
            ("uteke_tags_list", "GET", "/tags"),
        ];
        for tool in uteke_mcp::READ_ONLY_TOOLS {
            let (_, method, path) = MAP
                .iter()
                .find(|(t, _, _)| t == tool)
                .unwrap_or_else(|| panic!("{tool} has no HTTP row mapping"));
            let row =
                find(method, path).unwrap_or_else(|| panic!("{method} {path} not registered"));
            assert!(!row.writes, "{tool} maps to writing row {method} {path}");
        }
        for (tool, _, _) in MAP {
            assert!(
                uteke_mcp::READ_ONLY_TOOLS.contains(tool),
                "{tool} is mapped but not in READ_ONLY_TOOLS"
            );
        }
    }
}
