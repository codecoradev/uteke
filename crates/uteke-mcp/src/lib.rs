//! uteke-mcp library — shared MCP protocol handler.
//!
//! Used by both the stdio binary (`uteke-mcp`) and the HTTP endpoint
//! on `uteke-server` (`POST /mcp`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uteke_core::Uteke;

// ── JSON-RPC types ──────────────────────────────────────────────────────────
//
// Per JSON-RPC 2.0 spec:
//   - "result" and "error" are mutually exclusive; omit the absent one.
//   - Notifications (id is None/absent) MUST NOT receive a response.

#[derive(Serialize)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    Success {
        jsonrpc: &'static str,
        id: Value,
        result: Value,
    },
    Error {
        jsonrpc: &'static str,
        id: Value,
        error: JsonRpcError,
    },
}

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

// ── MCP Protocol types ──────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "role")]
enum McpContent {
    #[serde(rename = "text")]
    Text { r#type: String, text: String },
}

#[derive(Serialize)]
struct ToolResult {
    content: Vec<McpContent>,
    #[serde(rename = "isError")]
    is_error: bool,
}

/// Who is calling (#1439). The HTTP bridge passes the token role; stdio is
/// always `Full` (the process belongs to the local caller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Every tool.
    Full,
    /// Only the tools in [`READ_ONLY_TOOLS`].
    ReadOnly,
}

/// Tools a read-only caller may use: the MCP counterparts of the GET rows and
/// the non-writing POST rows in uteke-server's `api_registry`. A new tool is
/// write-gated until it is added here on purpose.
pub const READ_ONLY_TOOLS: &[&str] = &[
    "uteke_recall",
    "uteke_search",
    "uteke_list",
    "uteke_get",
    "uteke_provenance",
    "uteke_timeline",
    "uteke_contradictions",
    "uteke_stats",
    "uteke_doc_get",
    "uteke_doc_list",
    "uteke_doc_search",
    "uteke_doc_list_rooms",
    "uteke_graph",
    "uteke_room_list",
    "uteke_room_recall",
    "uteke_room_memories",
    "uteke_room_stats",
    "uteke_room_summary",
    "uteke_room_summary_document",
    // MCP alias of the read-only summary document above; NOT the HTTP
    // `POST /room/document` route, which writes.
    "uteke_room_document",
    "uteke_room_list_documents",
    "uteke_tags_list",
];

/// Handle a single MCP JSON-RPC request with full access (#381).
///
/// Used by the stdio binary, whose caller owns the process. Equivalent to
/// `handle_jsonrpc_as(uteke, raw, Access::Full)`.
pub fn handle_jsonrpc(uteke: &Uteke, raw: &str) -> Option<String> {
    handle_jsonrpc_as(uteke, raw, Access::Full)
}

/// Handle a single MCP JSON-RPC request on behalf of a caller with the given
/// `access` (#381, #1439).
///
/// This is the shared handler behind both the stdio binary and the HTTP
/// endpoint. With `Access::ReadOnly`, `tools/call` refuses any tool not in
/// [`READ_ONLY_TOOLS`] before it runs. Returns `Some(JsonRpcResponse)` for
/// regular requests and `None` for notifications (which must not receive a
/// response per JSON-RPC 2.0 §4.1).
pub fn handle_jsonrpc_as(uteke: &Uteke, raw: &str, access: Access) -> Option<String> {
    let req: JsonRpcRequest = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            let resp = JsonRpcResponse::Error {
                jsonrpc: "2.0",
                id: Value::Null,
                error: JsonRpcError {
                    code: -32700,
                    message: format!("Parse error: {e}"),
                },
            };
            return Some(serde_json::to_string(&resp).unwrap_or_default());
        }
    };

    let is_notification = req.id.is_none();
    let id = req.id.unwrap_or(Value::Null);

    match handle_request(uteke, &req.method, req.params, access) {
        Ok(result) => {
            if is_notification {
                // Notifications must not receive any response per JSON-RPC 2.0 §4.1.
                None
            } else {
                Some(
                    serde_json::to_string(&JsonRpcResponse::Success {
                        jsonrpc: "2.0",
                        id,
                        result,
                    })
                    .unwrap_or_default(),
                )
            }
        }
        Err(msg) => {
            if is_notification {
                None
            } else {
                Some(
                    serde_json::to_string(&JsonRpcResponse::Error {
                        jsonrpc: "2.0",
                        id,
                        error: JsonRpcError {
                            code: -32603,
                            message: msg,
                        },
                    })
                    .unwrap_or_default(),
                )
            }
        }
    }
}

// ── Handler ─────────────────────────────────────────────────────────────────

fn handle_request(
    uteke: &Uteke,
    method: &str,
    params: Option<Value>,
    access: Access,
) -> Result<Value, String> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "uteke",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": initialize_instructions()
        })),

        "notifications/initialized" => Ok(Value::Null),

        "tools/list" => {
            let mut tools = vec![
                tool_remember(),
                tool_recall(),
                tool_search(),
                tool_list(),
                tool_get(),
                tool_provenance(),
                tool_timeline(),
                tool_contradictions(),
                tool_contradictions_undo(),
                tool_supersede(),
                tool_update(),
                tool_forget(),
                tool_stats(),
                tool_context(),
                tool_dream(),
                tool_verify(),
                tool_repair(),
                tool_doc_create(),
                tool_doc_update(),
                tool_doc_get(),
                tool_doc_list(),
                tool_doc_search(),
                tool_doc_delete(),
                tool_doc_move(),
                tool_graph(),
                tool_graph_add_edge(),
                tool_graph_remove_edge(),
                tool_room_create(),
                tool_room_list(),
                tool_room_delete(),
                tool_room_rename(),
                tool_room_update(),
                tool_room_memory_move(),
                tool_room_recall(),
                tool_room_memories(),
                tool_room_stats(),
                tool_room_summary(),
                tool_room_document(),
                tool_room_add_document(),
                tool_room_remove_document(),
                tool_room_list_documents(),
                tool_doc_list_rooms(),
                tool_tags_list(),
                tool_tags_rename(),
                tool_tags_delete(),
                tool_namespace_rename(),
                tool_namespace_delete(),
                tool_pin(),
                tool_unpin(),
            ];
            // A read-only caller sees only the tools it can call (#1446);
            // tools/call keeps its own gate below.
            if access == Access::ReadOnly {
                tools.retain(|t| {
                    t["name"]
                        .as_str()
                        .is_some_and(|n| READ_ONLY_TOOLS.contains(&n))
                });
            }
            Ok(serde_json::json!({ "tools": tools }))
        }

        "tools/call" => {
            let params = params.ok_or("Missing params for tools/call")?;
            let tool_name = params["name"].as_str().ok_or("Missing tool name")?;
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));

            if access == Access::ReadOnly && !READ_ONLY_TOOLS.contains(&tool_name) {
                // A tool error, not a JSON-RPC error: the call was understood
                // and refused (uteke-cloud answers route refusals the same way).
                let refused = ToolResult {
                    content: vec![McpContent::Text {
                        r#type: "text".to_string(),
                        text: format!("Read-only token cannot use {tool_name}"),
                    }],
                    is_error: true,
                };
                return serde_json::to_value(refused).map_err(|e| e.to_string());
            }

            let result = match tool_name {
                "uteke_remember" => exec_remember(uteke, &arguments)?,
                "uteke_recall" => exec_recall(uteke, &arguments)?,
                "uteke_search" => exec_search(uteke, &arguments)?,
                "uteke_list" => exec_list(uteke, &arguments)?,
                "uteke_get" => exec_get(uteke, &arguments)?,
                "uteke_provenance" => exec_provenance(uteke, &arguments)?,
                "uteke_timeline" => exec_timeline(uteke, &arguments)?,
                "uteke_contradictions" => exec_contradictions(uteke, &arguments)?,
                "uteke_contradictions_undo" => exec_contradictions_undo(uteke, &arguments)?,
                "uteke_supersede" => exec_supersede(uteke, &arguments)?,
                "uteke_update" => exec_update(uteke, &arguments)?,
                "uteke_forget" => exec_forget(uteke, &arguments)?,
                "uteke_stats" => exec_stats(uteke, &arguments)?,
                "uteke_context" => exec_context(uteke, &arguments)?,
                "uteke_dream" => exec_dream(uteke, &arguments)?,
                "uteke_verify" => exec_verify(uteke)?,
                "uteke_repair" => exec_repair(uteke)?,
                "uteke_doc_create" => exec_doc_create(uteke, &arguments)?,
                "uteke_doc_update" => exec_doc_update(uteke, &arguments)?,
                "uteke_doc_get" => exec_doc_get(uteke, &arguments)?,
                "uteke_doc_list" => exec_doc_list(uteke, &arguments)?,
                "uteke_doc_search" => exec_doc_search(uteke, &arguments)?,
                "uteke_doc_delete" => exec_doc_delete(uteke, &arguments)?,
                "uteke_doc_move" => exec_doc_move(uteke, &arguments)?,
                "uteke_graph" => exec_graph(uteke, &arguments)?,
                "uteke_graph_add_edge" => exec_graph_add_edge(uteke, &arguments)?,
                "uteke_graph_remove_edge" => exec_graph_remove_edge(uteke, &arguments)?,
                "uteke_room_create" => exec_room_create(uteke, &arguments)?,
                "uteke_room_list" => exec_room_list(uteke, &arguments)?,
                "uteke_room_delete" => exec_room_delete(uteke, &arguments)?,
                "uteke_room_rename" => exec_room_rename(uteke, &arguments)?,
                "uteke_room_update" => exec_room_update(uteke, &arguments)?,
                "uteke_room_memory_move" => exec_room_memory_move(uteke, &arguments)?,
                "uteke_room_recall" => exec_room_recall(uteke, &arguments)?,
                "uteke_room_memories" => exec_room_memories(uteke, &arguments)?,
                "uteke_room_stats" => exec_room_stats(uteke, &arguments)?,
                "uteke_room_summary" => exec_room_summary(uteke, &arguments)?,
                "uteke_room_summary_document" | "uteke_room_document" => {
                    exec_room_document(uteke, &arguments)?
                }
                "uteke_room_add_document" => exec_room_add_document(uteke, &arguments)?,
                "uteke_room_remove_document" => exec_room_remove_document(uteke, &arguments)?,
                "uteke_room_list_documents" => exec_room_list_documents(uteke, &arguments)?,
                "uteke_doc_list_rooms" => exec_doc_list_rooms(uteke, &arguments)?,
                "uteke_tags_list" => exec_tags_list(uteke, &arguments)?,
                "uteke_tags_rename" => exec_tags_rename(uteke, &arguments)?,
                "uteke_tags_delete" => exec_tags_delete(uteke, &arguments)?,
                "uteke_namespace_rename" => exec_namespace_rename(uteke, &arguments)?,
                "uteke_namespace_delete" => exec_namespace_delete(uteke, &arguments)?,
                "uteke_pin" => exec_pin(uteke, &arguments)?,
                "uteke_unpin" => exec_unpin(uteke, &arguments)?,
                _ => return Err(format!("Unknown tool: {tool_name}")),
            };

            Ok(serde_json::to_value(result).map_err(|e| e.to_string())?)
        }

        "ping" => Ok(serde_json::json!({})),

        _ => Err(format!("Unknown method: {method}")),
    }
}

// ── Tool Definitions ────────────────────────────────────────────────────────

fn tool_remember() -> Value {
    serde_json::json!({
        "name": "uteke_remember",
        "description": "Store a new memory in uteke. The content will be embedded and indexed for semantic search.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "The text content to remember" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for categorization (optional)" },
                "namespace": { "type": "string", "description": "DEPRECATED: namespaces will be removed in a future 0.x release; prefer rooms, tags and author. Namespace for isolation (default: 'default'); a non-default value makes the result carry a deprecation warning." },
                "type": { "type": "string", "description": "Memory type: fact, procedure, preference, decision, context, note, insight, reference, event (default: inferred from the content, else fact)" },
                "room": { "type": "string", "description": "Room ID for collaborative memory (optional)" },
                "author": { "type": "string", "description": "Who wrote this memory: the agent, user or tool name (1 to 128 characters). Recorded on the memory with or without a room; in a room it is also the room author (default there: anonymous)." }
            },
            "required": ["content"]
        }
    })
}

fn tool_recall() -> Value {
    serde_json::json!({
        "name": "uteke_recall",
        "description": "Unified semantic search over memories and documents. Returns the most relevant results ranked by embedding similarity. Use --type 'all' (default) to search both, 'memory' for memories only, or 'doc' for documents only.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query" },
                "limit": { "type": "integer", "description": "Max results (default 10, at most 100; a larger value is capped at 100, a non-integer is an error)", "default": 10, "maximum": 100 },
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace to search (default: 'default')" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Filter by tags (optional)" },
                "min_score": { "type": "number", "description": "Minimum similarity score 0..1 (default: 0.0)" },
                "type": { "type": "string", "enum": ["all", "memory", "doc"], "description": "Search type: 'all' (default, unified), 'memory', or 'doc'" },
                "strategy": { "type": "string", "enum": ["fusion", "hybrid", "vector", "fts5", "graph"], "description": "Recall strategy: 'fusion' (default since 0.16.0, weighted RRF of vector×1.7 + hybrid×1, #1123), 'hybrid' (vector+FTS5 via RRF), 'vector' (similarity only), 'fts5' (keyword only), or 'graph' (hybrid + graph-signal reranking)", "default": "fusion" },
                "explain": { "type": "boolean", "description": "Return per-result ranking signals (#1160): vector similarity/rank, RRF contributions, jaccard/salience/recency/graph boosts. Memory-only: do not pass type with explain." },
                "entity": { "type": "string", "description": "Only memories whose metadata entity equals this value" },
                "category": { "type": "string", "description": "Only memories whose metadata category equals this value" },
                "strict": { "type": "boolean", "description": "Use the strict similarity floor ([recall] min_score_strict, default 0.5) when min_score is not given" },
                "enrich": { "type": "boolean", "description": "Attach linked documents and memories to each result" },
                "at": { "type": "string", "description": "Point in time (RFC3339): recall the memories that existed then. Not combinable with explain or pack." },
                "after": { "type": "string", "description": "Only memories created at or after this time (RFC3339). Not combinable with explain or pack." },
                "before": { "type": "string", "description": "Only memories created at or before this time (RFC3339). Not combinable with explain or pack." },
                "pack": { "type": "boolean", "description": "Return a budgeted context pack (#1281): {selected, skipped, budget_used, budget_chars} instead of a bare list. Deterministic, LLM-free, rank-order preserving. Pair with budget_chars and exclude_ids.", "default": false },
                "budget_chars": { "type": "integer", "description": "Character budget for pack mode (default 4000).", "default": 4000 },
                "exclude_ids": { "type": "array", "items": { "type": "string" }, "description": "Memory IDs already injected this turn; excluded from the pack and reported as skipped[reason=excluded]." },
                "full_ids": { "type": "boolean", "description": "Print full UUIDs instead of the 8-char prefix. Use when prefixes are ambiguous (ids are UUIDv7: memories written within ~65s share a prefix, #1357).", "default": false }
            },
            "required": ["query"]
        }
    })
}

fn tool_list() -> Value {
    serde_json::json!({
        "name": "uteke_list",
        "description": "List memories, optionally filtered by tag.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tag": { "type": "string", "description": "Filter by tag (optional)" },
                "limit": { "type": "integer", "description": "Max results (default 10, at most 1000; a larger value is capped at 1000, a non-integer is an error)", "default": 10, "maximum": 1000 },
                "offset": { "type": "integer", "description": "Pagination offset (default 0)", "default": 0 },
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace (optional)" },
                "at": { "type": "string", "description": "Point in time (RFC3339): list the memories that existed then" },
                "include_meta": { "type": "boolean", "description": "Append the total, has_more and next_offset of the page (not with at)", "default": false },
                "full_ids": { "type": "boolean", "description": "Print full UUIDs instead of the 8-char prefix. Use when prefixes are ambiguous (ids are UUIDv7: memories written within ~65s share a prefix, #1357).", "default": false }
            }
        }
    })
}

fn tool_get() -> Value {
    serde_json::json!({
        "name": "uteke_get",
        "description": "Fetch a single memory's FULL record by id — content, tags, metadata, timestamps, importance, no truncation. Use after recall/search when you need the complete entry.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Full UUID or unambiguous prefix (8-char id from recall/list works)" }
            },
            "required": ["id"]
        }
    })
}

fn tool_provenance() -> Value {
    serde_json::json!({
        "name": "uteke_provenance",
        "description": "Full provenance report for a memory (#1172): author/source fields, trust tier, source hash at write vs live content hash (tamper evidence), and the full timeline event chain with actor + evidence. Use when auditing why a memory exists, who wrote it, and whether it changed after write.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Full UUID or unambiguous prefix" }
            },
            "required": ["id"]
        }
    })
}

/// Full provenance report for a memory (#1172 Fase 1).
fn exec_provenance(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let id = resolve_id(uteke, id_arg)?;

    let report = uteke
        .provenance(&id)
        .map_err(|e| format!("Failed: {e}"))?
        .ok_or_else(|| format!("Memory not found: {id}"))?;

    let text = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn tool_timeline() -> Value {
    serde_json::json!({
        "name": "uteke_timeline",
        "description": "Timeline event chain for one memory (#347): created/updated/deprecated/forgot/superseded events with actor + evidence. Audit a memory's full lifecycle — who wrote it, when it changed, whether and by whom it was deprecated or deleted. Companion to uteke_provenance (which adds author/source/trust fields).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Full UUID or unambiguous prefix" },
                "limit": { "type": "number", "description": "Max events to return (default 100)" }
            },
            "required": ["id"]
        }
    })
}

/// Timeline event chain for one memory (#347, owner decision 2026-09-18:
/// audit-grade history). Events survive memory deletion, so this also works
/// as a tombstone lookup for forgotten memories.
fn exec_timeline(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let id = resolve_id(uteke, id_arg)?;
    let limit = args["limit"].as_u64().unwrap_or(100).min(1000) as usize;

    let events = uteke
        .timeline(&id, limit)
        .map_err(|e| format!("Failed: {e}"))?;

    let text = serde_json::to_string_pretty(&events).unwrap_or_else(|_| "[]".to_string());
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn tool_contradictions() -> Value {
    serde_json::json!({
        "name": "uteke_contradictions",
        "description": "List the contradiction resolution ledger (#1172): memories that were superseded and soft-deprecated by conflict resolution, with winner, reason, and timestamp. Audit what the pipeline decided without digging through timeline events.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Filter by namespace" },
                "limit": { "type": "integer", "description": "Max entries (default 50)" }
            },
            "required": []
        }
    })
}

fn tool_contradictions_undo() -> Value {
    serde_json::json!({
        "name": "uteke_contradictions_undo",
        "description": "Undo a contradiction resolution (#1172): restore a superseded memory to active, remove the supersession edge pair, and record the undo in the audit trail. Use when a conflict resolution was wrong.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Full UUID or unambiguous prefix of the RETIRED memory to restore" }
            },
            "required": ["id"]
        }
    })
}

/// Contradiction resolution ledger (#1172 Fase 2).
fn exec_contradictions(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(50) as usize;

    let resolutions = uteke
        .contradiction_resolutions(namespace, limit)
        .map_err(|e| format!("Failed: {e}"))?;

    let text = serde_json::to_string_pretty(&resolutions).unwrap_or_else(|_| "[]".to_string());
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

/// Undo a contradiction resolution (#1172 Fase 2).
fn exec_contradictions_undo(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let id = resolve_id(uteke, id_arg)?;

    match uteke
        .undo_supersession(&id)
        .map_err(|e| format!("Failed: {e}"))?
    {
        Some(winner) => {
            let text = format!(
                "\u{2713} Restored memory {id}\n  was superseded by {winner}\n  supersession edges removed \u{2014} the pair is no longer flagged"
            );
            Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text,
                }],
                is_error: false,
            })
        }
        None => Err(format!("No supersession found for memory: {id}")),
    }
}

fn tool_supersede() -> Value {
    serde_json::json!({
        "name": "uteke_supersede",
        "description": "Mark a memory as superseded by a newer one (e.g. a decision pivot). Wires superseded_by/supersedes edges and soft-deprecates the old memory — recall flags the pair so agents don't act on stale info.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "old_id": { "type": "string", "description": "Full UUID or unambiguous prefix of the STALE memory" },
                "new_id": { "type": "string", "description": "Full UUID or unambiguous prefix of the CURRENT memory" },
                "reason": { "type": "string", "description": "Why it was superseded (stored on the deprecation, e.g. 'ADR-0005 pivot')" }
            },
            "required": ["old_id", "new_id"]
        }
    })
}

fn tool_update() -> Value {
    serde_json::json!({
        "name": "uteke_update",
        "description": "Partially update a memory — only provided fields change (same semantics as HTTP PUT /memory). Changing content regenerates its embedding.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Full UUID or unambiguous prefix" },
                "content": { "type": "string", "description": "New content (triggers re-embed)" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Replacement tag set" },
                "metadata": { "type": "object", "description": "Replacement metadata JSON" },
                "importance": { "type": "number", "description": "0.0–1.0" },
                "pinned": { "type": "boolean", "description": "Pin (never decays) or unpin" },
                "memory_type": { "type": "string", "description": "fact | procedure | preference | decision | context | note | insight | reference | event" },
                "namespace": { "type": "string", "description": "DEPRECATED: namespaces will be removed in a future 0.x release; prefer rooms, tags and author. Move the memory to this namespace (#1181 — plain move, no re-embed); the result carries a deprecation warning." }
            },
            "required": ["id"]
        }
    })
}

fn tool_forget() -> Value {
    serde_json::json!({
        "name": "uteke_forget",
        "description": "Delete a memory by its ID.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Full UUID or unambiguous prefix (8-char id from recall/list works)" }
            },
            "required": ["id"]
        }
    })
}

fn tool_stats() -> Value {
    serde_json::json!({
        "name": "uteke_stats",
        "description": "Get memory statistics (total count, tags, tiers).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace (optional)" }
            }
        }
    })
}

fn tool_search() -> Value {
    serde_json::json!({
        "name": "uteke_search",
        "description": "Keyword (FTS5) text search over stored memories. Faster than semantic recall for exact matches.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keywords to search for" },
                "limit": { "type": "integer", "description": "Max results (default 10)", "default": 10 },
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace (optional)" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Filter by tags (optional)" }
            },
            "required": ["query"]
        }
    })
}

fn tool_doc_create() -> Value {
    serde_json::json!({
        "name": "uteke_doc_create",
        "description": "Create or update a document in the wiki/knowledge base. Markdown content is auto-chunked and embedded for section-level semantic search.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "slug": { "type": "string", "description": "URL-friendly identifier (globally unique)" },
                "title": { "type": "string", "description": "Document title (auto-derived from first heading if omitted)" },
                "content": { "type": "string", "description": "Full markdown content" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags (optional)" },
                "parent": { "type": "string", "description": "Parent document slug for hierarchy (optional)" }
            },
            "required": ["slug", "content"]
        }
    })
}

fn tool_doc_update() -> Value {
    serde_json::json!({
        "name": "uteke_doc_update",
        "description": "Partially update a document. Changed content triggers automatic chunk rebuild. Title/tags/metadata-only updates skip chunk rebuild.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Document UUID or slug" },
                "title": { "type": "string", "description": "New title (optional)" },
                "content": { "type": "string", "description": "New markdown content (optional, triggers chunk rebuild)" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Replace tags (optional)" },
                "metadata": { "type": "object", "description": "Replace metadata (optional)" },
            },
            "required": ["id"]
        }
    })
}

fn tool_doc_get() -> Value {
    serde_json::json!({
        "name": "uteke_doc_get",
        "description": "Get a document by slug or ID. Returns full markdown content.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id_or_slug": { "type": "string", "description": "Document slug or UUID" },
            },
            "required": ["id_or_slug"]
        }
    })
}

fn tool_doc_list() -> Value {
    serde_json::json!({
        "name": "uteke_doc_list",
        "description": "List documents in the wiki/knowledge base.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "Max results (default 20)", "default": 20 },
            }
        }
    })
}

fn tool_doc_search() -> Value {
    serde_json::json!({
        "name": "uteke_doc_search",
        "description": "Search documents by meaning or keywords. Supports semantic, FTS5, and hybrid (default) search modes.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "mode": { "type": "string", "description": "Search mode: semantic, fts, or hybrid (default: hybrid)" },
                "limit": { "type": "integer", "description": "Max results (default 5)", "default": 5 },
            },
            "required": ["query"]
        }
    })
}

fn tool_doc_delete() -> Value {
    serde_json::json!({
        "name": "uteke_doc_delete",
        "description": "Delete a document by its UUID. Cascades to all chunks.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Document UUID" }
            },
            "required": ["id"]
        }
    })
}

fn tool_doc_move() -> Value {
    serde_json::json!({
        "name": "uteke_doc_move",
        "description": "Move a document to a new parent or root. Updates parent_id in the documents table.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Document UUID or slug to move" },
                "parent": { "type": "string", "description": "New parent document slug or UUID. Omit to move to root." }
            },
            "required": ["id"]
        }
    })
}

fn tool_graph() -> Value {
    serde_json::json!({
        "name": "uteke_graph",
        "description": "Get the memory graph: nodes (live memories), edges (auto-links, supersession, explicit edges), stats, and the most recent edges.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Filter by namespace (optional)" },
                "limit": { "type": "integer", "description": "Max edges (default 500, max 5000)" }
            }
        }
    })
}

fn tool_context() -> Value {
    serde_json::json!({
        "name": "uteke_context",
        "description": "Get a smart project context summary. Returns memory counts by type, top tags, and recent activity — ready to inject into agent prompts. Not raw recall, but a structured overview of what the agent knows.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace to summarize (default: 'default')" }
            }
        }
    })
}

fn tool_dream() -> Value {
    serde_json::json!({
        "name": "uteke_dream",
        "description": "Run the dream cycle maintenance pipeline: lint → backlinks → dedup → orphans → compact → verify. DESTRUCTIVE when applied — defaults to dry_run (preview only). To apply changes you MUST pass dry_run=false, and ideally scope to a single namespace. Always dry-run first to preview projected changes.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace to process. STRONGLY RECOMMENDED — omitting processes ALL namespaces" },
                "dry_run": { "type": "boolean", "description": "Preview changes without applying (default: TRUE — no mutations). Pass false explicitly to apply" },
                "phases": { "type": "array", "items": { "type": "string" }, "description": "Specific phases: lint, backlinks, dedup, orphans, compact, verify (default: all)" },
                "confirm_large": { "type": "boolean", "description": "Required when an APPLYING run projects more than 100 changes (default: false — run refuses instead)" }
            }
        }
    })
}

fn tool_verify() -> Value {
    serde_json::json!({
        "name": "uteke_verify",
        "description": "Check vector-index/SQLite consistency without mutating anything (#1266). Reports row/vector counts and whether they match. Run after serve upgrades or whenever recall behaves as if recent memories lack vectors.",
        "inputSchema": { "type": "object", "properties": {} }
    })
}

fn tool_repair() -> Value {
    serde_json::json!({
        "name": "uteke_repair",
        "description": "Rebuild the vector index from stored embeddings (#1266). Fixes recall desync where rows exist in SQLite/FTS5 but not in the usearch index. DESTRUCTIVE to the index only — SQLite data is untouched; the index is rebuilt from it. Verify first with uteke_verify.",
        "inputSchema": { "type": "object", "properties": {} }
    })
}

fn tool_room_recall() -> Value {
    serde_json::json!({
        "name": "uteke_room_recall",
        "description": "Semantic recall within a room context. Requires an EXISTING room_id (create via uteke_room_create first) — unknown room ids error at call time. Searches across all namespaces in the room using hybrid RRF ranking. Optional min_score, tags (any-of), pack/budget_chars/exclude_ids (budgeted context pack) mirror uteke_recall; every result carries its memory id.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" },
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default 5)", "default": 5 },
                "min_score": { "type": "number", "description": "Minimum score; weaker results are dropped (default 0.0)" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Keep only memories that carry at least one of these tags (optional)" },
                "pack": { "type": "boolean", "description": "Return a budgeted context pack (#1281): {selected, skipped, budget_used, budget_chars} instead of a bare list. Deterministic, LLM-free, rank-order preserving. Pair with budget_chars and exclude_ids.", "default": false },
                "budget_chars": { "type": "integer", "description": "Character budget for pack mode (default 4000).", "default": 4000 },
                "exclude_ids": { "type": "array", "items": { "type": "string" }, "description": "Memory IDs already injected this turn; excluded from the pack and reported as skipped[reason=excluded]." },
                "full_ids": { "type": "boolean", "description": "Print full UUIDs instead of the 8-char prefix in the id line (ids are UUIDv7, prefixes collide, #1357).", "default": false }
            },
            "required": ["room_id", "query"]
        }
    })
}

fn tool_room_memories() -> Value {
    serde_json::json!({
        "name": "uteke_room_memories",
        "description": "List all memories in a room chronologically (by joined_at). Cross-namespace: returns memories from all namespaces. Use this for full timeline listing without semantic ranking.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" },
                "author": { "type": "string", "description": "Optional author filter" },
                "limit": { "type": "integer", "description": "Max results (default 100)", "default": 100 },
                "full_ids": { "type": "boolean", "description": "Print full UUIDs instead of the 8-char prefix. Use when prefixes are ambiguous (ids are UUIDv7: memories written within ~65s share a prefix, #1357).", "default": false }
            },
            "required": ["room_id"]
        }
    })
}

fn tool_room_create() -> Value {
    serde_json::json!({
        "name": "uteke_room_create",
        "description": "Create a new room for collaborative memory. A room groups memories by topic with participant tracking.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Unique room identifier" },
                "title": { "type": "string", "description": "Room title (optional)" },
                "namespace": { "type": "string", "description": "DEPRECATED: namespaces will be removed in a future 0.x release; prefer rooms, tags and author. Namespace for the room (default: 'default')." }
            },
            "required": ["room_id"]
        }
    })
}

/// Short, workspace-independent guidance sent with `initialize` (#1395).
/// The full tool guide stays behind `uteke guide`.
fn initialize_instructions() -> String {
    format!(
        "{} Run `uteke guide` for the full memory tools guide.",
        uteke_core::guide::CONVENTIONS_FIRST
    )
}

fn tool_room_list() -> Value {
    serde_json::json!({
        "name": "uteke_room_list",
        "description": "List all rooms, optionally filtered by namespace and/or by a case-insensitive substring of the room id or title.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Filter by namespace (omit for all)" },
                "name": { "type": "string", "description": "Keep only rooms whose id or title contains this text, case-insensitive (omit for all)" }
            }
        }
    })
}

fn tool_room_delete() -> Value {
    serde_json::json!({
        "name": "uteke_room_delete",
        "description": "Unregister a room: its links are removed and the memories are retained (they are not deleted). To delete a memory, use uteke_forget with its id.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier to delete" }
            },
            "required": ["room_id"]
        }
    })
}

fn tool_room_rename() -> Value {
    serde_json::json!({
        "name": "uteke_room_rename",
        "description": "Rename a room. The registry row and every member/document link move in one transaction; title, description, and namespace are preserved (#1202).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Current room identifier" },
                "to": { "type": "string", "description": "New room identifier (must not exist)" }
            },
            "required": ["from", "to"]
        }
    })
}

fn tool_room_update() -> Value {
    serde_json::json!({
        "name": "uteke_room_update",
        "description": "Update a room's title and/or description. Omitted fields stay unchanged (#1202).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" },
                "title": { "type": "string", "description": "New title (omit to keep current)" },
                "description": { "type": "string", "description": "New description (omit to keep current)" }
            },
            "required": ["room_id"]
        }
    })
}

fn tool_room_memory_move() -> Value {
    serde_json::json!({
        "name": "uteke_room_memory_move",
        "description": "Move a memory from one room to another, preserving the link's author/role/joined_at provenance (#1202).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "memory_id": { "type": "string", "description": "Memory identifier" },
                "from_room": { "type": "string", "description": "Source room (memory must be a member)" },
                "to_room": { "type": "string", "description": "Target room (must exist)" }
            },
            "required": ["memory_id", "from_room", "to_room"]
        }
    })
}

fn tool_room_stats() -> Value {
    serde_json::json!({
        "name": "uteke_room_stats",
        "description": "Show room statistics including memory count, participant list, and activity timestamps.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" }
            },
            "required": ["room_id"]
        }
    })
}

fn tool_room_summary() -> Value {
    serde_json::json!({
        "name": "uteke_room_summary",
        "description": "Generate a topic clustering summary for a room. Returns topic clusters, participants, time range, top tags, recent decisions, and pinned highlights.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" }
            },
            "required": ["room_id"]
        }
    })
}

fn tool_room_document() -> Value {
    serde_json::json!({
        "name": "uteke_room_summary_document",
        "description": "Generate a structured document from room memories, grouped by memory type (decisions, facts, procedures, preferences, etc.). Useful for producing meeting minutes or decision records.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" }
            },
            "required": ["room_id"]
        }
    })
}

fn tool_room_add_document() -> Value {
    serde_json::json!({
        "name": "uteke_room_add_document",
        "description": "Link a document to a room. The document must already exist (use uteke_doc_upsert first).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" },
                "doc_slug": { "type": "string", "description": "Document slug to link" }
            },
            "required": ["room_id", "doc_slug"]
        }
    })
}

fn tool_room_remove_document() -> Value {
    serde_json::json!({
        "name": "uteke_room_remove_document",
        "description": "Unlink a document from a room. Does not delete the document itself.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" },
                "doc_slug": { "type": "string", "description": "Document slug to unlink" }
            },
            "required": ["room_id", "doc_slug"]
        }
    })
}

fn tool_room_list_documents() -> Value {
    serde_json::json!({
        "name": "uteke_room_list_documents",
        "description": "List all document slugs linked to a room.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" }
            },
            "required": ["room_id"]
        }
    })
}

fn tool_doc_list_rooms() -> Value {
    serde_json::json!({
        "name": "uteke_doc_list_rooms",
        "description": "List all rooms that reference a specific document.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "doc_slug": { "type": "string", "description": "Document slug to look up" }
            },
            "required": ["doc_slug"]
        }
    })
}

fn tool_tags_list() -> Value {
    serde_json::json!({
        "name": "uteke_tags_list",
        "description": "List all tags with usage counts. Optionally filter by namespace and sort by count (default) or alphabetically.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace to filter tags (default: all namespaces)" },
                "sort": { "type": "string", "enum": ["count", "alpha"], "description": "Sort order: 'count' by usage count descending (default), 'alpha' alphabetically" }
            }
        }
    })
}

fn tool_tags_rename() -> Value {
    serde_json::json!({
        "name": "uteke_tags_rename",
        "description": "Rename a tag across all memories. Updates both the tag index and memory records atomically.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "old_tag": { "type": "string", "description": "Current tag name to rename" },
                "new_tag": { "type": "string", "description": "New tag name" },
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace scope (default: all namespaces)" }
            },
            "required": ["old_tag", "new_tag"]
        }
    })
}

fn tool_tags_delete() -> Value {
    serde_json::json!({
        "name": "uteke_tags_delete",
        "description": "Delete a tag from all memories. Removes the tag from every memory that uses it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tag": { "type": "string", "description": "Tag name to delete" },
                "namespace": { "type": "string", "description": "DEPRECATED (prefer rooms, tags and author): Namespace scope (default: all namespaces)" }
            },
            "required": ["tag"]
        }
    })
}

fn tool_namespace_rename() -> Value {
    serde_json::json!({
        "name": "uteke_namespace_rename",
        "description": "DEPRECATED (namespaces will be removed in a future 0.x release; prefer rooms, tags and author). Rename a namespace, merging into the target when it already exists. Namespaces are a derived view — the old name vanishes when its last memory moved (#1181).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Current namespace name" },
                "to": { "type": "string", "description": "New namespace name (existing name = merge)" }
            },
            "required": ["from", "to"]
        }
    })
}

fn tool_namespace_delete() -> Value {
    serde_json::json!({
        "name": "uteke_namespace_delete",
        "description": "DEPRECATED (namespaces will be removed in a future 0.x release; prefer rooms, tags and author). Delete a namespace with an explicit strategy for its memories: refuse (default — error while any memory uses the name), merge (move all memories to `target`), or deprecate (soft-delete — restorable via promote, never hard-deleted) (#1181).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Namespace to delete" },
                "strategy": { "type": "string", "enum": ["refuse", "merge", "deprecate"], "description": "Default: refuse" },
                "target": { "type": "string", "description": "Target namespace when strategy = merge" }
            },
            "required": ["name"]
        }
    })
}

fn tool_pin() -> Value {
    serde_json::json!({
        "name": "uteke_pin",
        "description": "Pin a memory so it never decays. Pinned memories are immune to aging and pruning during maintenance cycles.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Full UUID or unambiguous prefix (8-char id from recall/list works)" }
            },
            "required": ["id"]
        }
    })
}

fn tool_unpin() -> Value {
    serde_json::json!({
        "name": "uteke_unpin",
        "description": "Unpin a memory, allowing it to decay normally.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Full UUID or unambiguous prefix (8-char id from recall/list works)" }
            },
            "required": ["id"]
        }
    })
}

fn tool_graph_add_edge() -> Value {
    serde_json::json!({
        "name": "uteke_graph_add_edge",
        "description": "Add an edge between two memories in the knowledge graph. Both memories must exist. Self-loops are rejected.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "source": { "type": "string", "description": "Source memory ID" },
                "target": { "type": "string", "description": "Target memory ID" },
                "edge_type": { "type": "string", "description": "Edge relation type (default: 'related')" },
                "weight": { "type": "number", "description": "Edge weight (default: 1.0)" }
            },
            "required": ["source", "target"]
        }
    })
}

fn tool_graph_remove_edge() -> Value {
    serde_json::json!({
        "name": "uteke_graph_remove_edge",
        "description": "Remove an edge between two memories in the knowledge graph.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "source": { "type": "string", "description": "Source memory ID" },
                "target": { "type": "string", "description": "Target memory ID" }
            },
            "required": ["source", "target"]
        }
    })
}

// ── Tool Executors ──────────────────────────────────────────────────────────

/// Decode the arguments of `uteke_remember` into the typed request shared
/// with the HTTP server and the CLI (#1343), under the same rules as
/// `POST /remember` (#1439): content and tags validated at decode, an omitted
/// `type` inferred from the content, an unknown `type` refused, a `type` copied
/// into the metadata, a wrong-shaped `tags` an error and an unknown argument
/// refused. `room` is the one MCP-only argument (it routes the write to a room).
/// The tool exposes no entity/category/metadata/source/author_type fields, so
/// those stay unset.
pub fn decode_remember_args(args: &Value) -> Result<uteke_core::RememberRequest, String> {
    reject_unknown_args(&tool_remember(), args, &[])?;
    let content = args["content"].as_str().ok_or("Missing 'content'")?;
    let tags = match &args["tags"] {
        Value::Null => Vec::new(),
        Value::Array(a) => a
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or("Invalid 'tags': expected an array of strings")?,
        _ => return Err("Invalid 'tags': expected an array of strings".into()),
    };
    let input = uteke_core::RememberInput {
        content: content.to_string(),
        tags,
        namespace: args["namespace"].as_str().map(str::to_string),
        memory_type: match &args["type"] {
            Value::Null => None,
            Value::String(t) => Some(t.clone()),
            _ => return Err("Invalid 'type': expected a string".into()),
        },
        room: args["room"].as_str().map(str::to_string),
        author: args["author"].as_str().map(str::to_string),
        ..uteke_core::RememberInput::default()
    };
    uteke_core::RememberRequest::decode(input).map_err(|e| e.to_string())
}

/// Deprecation signal for a namespaced write (#1427, phase 1): a trailing
/// warning line for the tool text, empty for the default namespace so those
/// results stay byte-identical. Also logs once per namespace (server side).
fn namespace_deprecation_suffix(namespace: Option<&str>) -> String {
    uteke_core::namespace_deprecation::namespace_write_warning(namespace)
        .map(|w| format!("\n⚠ {w}"))
        .unwrap_or_default()
}

/// The error text of a failed write. Strict privacy mode (#1464) is
/// recognisable: it starts with `secret_detected`, the same code the HTTP API
/// uses.
fn write_error(e: uteke_core::Error) -> String {
    match e {
        uteke_core::Error::SecretDetected { .. } => format!("secret_detected: {e}"),
        other => format!("Failed: {other}"),
    }
}

fn exec_remember(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let req = decode_remember_args(args)?;
    let content = req.content.as_str();
    let tags = req.tag_refs();
    let namespace = req.namespace.as_deref();
    let room = req.room.as_deref();
    let author = req.author.as_deref().unwrap_or("anonymous");
    let metadata = req.metadata.clone();

    let outcome = if let Some(room_id) = room {
        // A room write takes a concrete type: infer it as every write does.
        let memory_type = req
            .memory_type
            .as_deref()
            .unwrap_or_else(|| uteke_core::MemoryType::infer_stored_name(content));
        uteke
            .remember_in_room(
                content,
                &tags,
                metadata,
                namespace,
                memory_type,
                room_id,
                author,
            )
            .map_err(write_error)?
    } else if let Some(t) = req.memory_type.as_deref() {
        uteke
            .remember_typed_detailed(content, &tags, metadata, namespace, t)
            .map_err(write_error)?
    } else {
        uteke
            .remember_detailed(content, &tags, metadata, namespace)
            .map_err(write_error)?
    };
    // The named writer is the memory's own author, with or without a room
    // (#1427 phase 2); already validated and trimmed when the arguments were
    // decoded. The "anonymous" fallback of the room link is not an author.
    if let Some(a) = req.author.as_deref() {
        uteke
            .set_author(&outcome.id, Some(a))
            .map_err(|e| format!("Failed to set author: {e}"))?;
    }

    let mut text = format!("✓ Stored memory with ID: {}", outcome.id);
    if let Some(ref warn) = outcome.warning {
        text.push_str(&format!(
            "\n⚠ Embedding NOT written: {warn}\nMemory is keyword-searchable only; run `uteke repair` to backfill."
        ));
    }
    if let Some(summary) = outcome.redaction_summary() {
        text.push_str(&format!(
            "\n⚠ Secrets masked in the stored text (#1464): {summary}"
        ));
    }
    text.push_str(&namespace_deprecation_suffix(namespace));

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

/// Decode the arguments of `uteke_recall` into the typed request shared with
/// the HTTP server and the CLI (#1343), under the MCP policy: no limit cap,
/// blank queries accepted, `tags: []` kept as a filter. The tool exposes no
/// entity/category/enrich/at/after/before/strict, so those stay unset.
pub fn decode_recall_args(args: &Value) -> Result<uteke_core::RecallRequest, String> {
    let query = args["query"].as_str().ok_or("Missing 'query'")?;
    // An invalid `limit` is an error, as on `POST /recall`, not a silent default (#1439).
    let limit = match &args["limit"] {
        Value::Null => None,
        v => Some(
            v.as_u64()
                .ok_or("Invalid 'limit': expected a non-negative integer")? as usize,
        ),
    };
    let input = uteke_core::RecallInput {
        query: query.to_string(),
        limit,
        tags: args["tags"].as_array().map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        }),
        namespace: args["namespace"].as_str().map(str::to_string),
        min_score: args["min_score"].as_f64().map(|m| m as f32),
        search_type: args["type"].as_str().map(str::to_string),
        entity: args["entity"].as_str().map(str::to_string),
        category: args["category"].as_str().map(str::to_string),
        strict: args["strict"].as_bool().unwrap_or(false),
        enrich: args["enrich"].as_bool().unwrap_or(false),
        at: args["at"].as_str().map(str::to_string),
        after: args["after"].as_str().map(str::to_string),
        before: args["before"].as_str().map(str::to_string),
        pack: args["pack"].as_bool().unwrap_or(false),
        budget_chars: args["budget_chars"].as_u64().map(|b| b as usize),
        exclude_ids: args["exclude_ids"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        strategy: args["strategy"].as_str().map(str::to_string),
        explain: args["explain"].as_bool().unwrap_or(false),
    };
    uteke_core::RecallRequest::decode(input).map_err(|e| e.to_string())
}

fn exec_recall(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    reject_unknown_args(&tool_recall(), args, &["full_ids"])?;
    let mut req = decode_recall_args(args)?;
    // The documented MCP default is memories and documents; the shared
    // execution treats an absent type as memory-only, so name it here. Explain
    // is memory-only and keeps its type absent.
    if !req.explain {
        req = req.with_default_search_type("all");
    }
    let full_ids = args["full_ids"].as_bool().unwrap_or(false);

    // The similarity floor and the strategy fall back to `[recall]` of
    // uteke.toml (defaults equal the built-ins), as `POST /recall` does.
    let recall = uteke.recall_settings();
    let defaults = uteke_core::recall_exec::RecallDefaults {
        min_score: Some(recall.min_score),
        min_score_strict: Some(recall.min_score_strict),
        default_strategy: Some(recall.default_strategy.as_str()),
    };
    // The same decision tree as `POST /recall` (#1439): a guard failure or an
    // unknown strategy/type is the bare message; anything else is `Failed: ...`.
    let output = uteke.run_recall(&req, defaults).map_err(|e| match e {
        // The wire name of `search_type` is `type` on this tool.
        uteke_core::Error::Validation(msg) => msg.replace("omit search_type", "omit type"),
        other => format!("Failed: {other}"),
    })?;

    let text_result = |text: String| ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    };
    use uteke_core::recall_exec::RecallOutput;
    match output {
        RecallOutput::Explained(explained) => {
            if explained.is_empty() {
                return Ok(text_result("No results found.".to_string()));
            }
            Ok(text_result(
                serde_json::to_string_pretty(&explained).unwrap_or_else(|_| "[]".to_string()),
            ))
        }
        RecallOutput::Packed(pack) => {
            if pack.selected.is_empty() {
                return Ok(text_result("No results fit the budget.".to_string()));
            }
            Ok(text_result(
                serde_json::to_string_pretty(&pack).unwrap_or_else(|_| "{}".to_string()),
            ))
        }
        RecallOutput::Unified(results) => {
            if results.is_empty() {
                return Ok(text_result("No results found.".to_string()));
            }
            let mut lines = Vec::new();
            for (i, r) in results.iter().enumerate() {
                let type_label = match r.result_type {
                    uteke_core::SearchResultType::Memory => "[mem]",
                    uteke_core::SearchResultType::Document => "[doc]",
                };
                let detail = match &r.result_type {
                    uteke_core::SearchResultType::Memory => r
                        .memory_id
                        .as_ref()
                        .map(|id| format!(" (id: {})", display_id(id, full_ids)))
                        .unwrap_or_default(),
                    uteke_core::SearchResultType::Document => r
                        .doc_slug
                        .as_ref()
                        .map(|slug| format!(" (slug: {})", slug))
                        .unwrap_or_default(),
                };
                lines.push(format!(
                    "{}{}. [{:.2}] {}",
                    i + 1,
                    type_label,
                    r.score,
                    r.content
                ));
                if !detail.is_empty() {
                    lines.push(format!("       {}", detail));
                }
                // #1053: flag memories superseded by a newer decision so agents
                // don't act on stale info.
                if let uteke_core::SearchResultType::Memory = r.result_type {
                    if let Some(mid) = &r.memory_id {
                        if let Ok(Some(newer)) = uteke.supersession_of(mid) {
                            let short: String = newer.chars().take(8).collect();
                            lines.push(format!(
                                "       ⚠ superseded by {short} — verify before acting"
                            ));
                        }
                    }
                }
            }
            Ok(text_result(lines.join("\n")))
        }
        // Memory-only results (a point in time was given): the same shape as
        // the unified lines, all memories.
        RecallOutput::Memories { results, .. } => {
            if results.is_empty() {
                return Ok(text_result("No results found.".to_string()));
            }
            let mut lines = Vec::new();
            for (i, r) in results.iter().enumerate() {
                lines.push(format!(
                    "{}[mem]. [{:.2}] {}",
                    i + 1,
                    r.score,
                    r.memory.content
                ));
                lines.push(format!(
                    "       (id: {})",
                    display_id(&r.memory.id, full_ids)
                ));
            }
            Ok(text_result(lines.join("\n")))
        }
    }
}

/// Refuse an argument the tool does not declare (#1439), with the list of the
/// ones it does. The list comes from the tool's own `inputSchema`, so the two
/// cannot drift; `extra` are accepted names the schema does not list.
fn reject_unknown_args(tool: &Value, args: &Value, extra: &[&str]) -> Result<(), String> {
    let (Some(given), Some(declared)) = (
        args.as_object(),
        tool["inputSchema"]["properties"].as_object(),
    ) else {
        return Ok(());
    };
    for key in given.keys() {
        if !declared.contains_key(key) && !extra.contains(&key.as_str()) {
            let mut supported: Vec<&str> = declared
                .keys()
                .map(String::as_str)
                .chain(extra.iter().copied())
                .collect();
            supported.sort_unstable();
            return Err(format!(
                "Unknown argument '{key}'. Supported: {}",
                supported.join(", ")
            ));
        }
    }
    Ok(())
}

/// Decode the arguments of `uteke_list` into the typed request shared with
/// the HTTP server and the CLI (#1343), under the MCP policy: default limit
/// 20, no cap. Fields of the wrong type fall back to their defaults. The tool
/// exposes no `at`/`include_meta`, so those stay unset.
pub fn decode_list_args(args: &Value) -> Result<uteke_core::ListRequest, String> {
    // A field of the wrong type is an error, as on `POST /list`, not a silent
    // default (#1439).
    let uint = |key: &str| -> Result<Option<usize>, String> {
        match &args[key] {
            Value::Null => Ok(None),
            v => v
                .as_u64()
                .map(|n| Some(n as usize))
                .ok_or_else(|| format!("Invalid '{key}': expected a non-negative integer")),
        }
    };
    let text = |key: &str| -> Result<Option<String>, String> {
        match &args[key] {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s.clone())),
            _ => Err(format!("Invalid '{key}': expected a string")),
        }
    };
    let include_meta = match &args["include_meta"] {
        Value::Null => false,
        Value::Bool(b) => *b,
        _ => return Err("Invalid 'include_meta': expected a boolean".to_string()),
    };
    let input = uteke_core::ListInput {
        tag: text("tag")?,
        limit: uint("limit")?,
        offset: uint("offset")?,
        namespace: text("namespace")?,
        at: text("at")?,
        include_meta,
    };
    uteke_core::ListRequest::decode(input).map_err(|e| e.to_string())
}

fn exec_list(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    reject_unknown_args(&tool_list(), args, &["full_ids"])?;
    let req = decode_list_args(args)?;
    let full_ids = args["full_ids"].as_bool().unwrap_or(false);

    // The same page / point-in-time / envelope choice as `POST /list` (#1439).
    use uteke_core::list_exec::ListOutput;
    let (memories, footer) = match uteke.run_list(&req).map_err(|e| format!("Failed: {e}"))? {
        ListOutput::Memories(m) => (m, None),
        ListOutput::Page {
            memories,
            total,
            meta,
        } => {
            let next = meta
                .next_offset
                .map_or("none".to_string(), |n| n.to_string());
            (
                memories,
                Some(format!(
                    "-- total {total}, has_more {}, next_offset {next}",
                    meta.has_more
                )),
            )
        }
    };

    if memories.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No memories found.".to_string(),
            }],
            is_error: false,
        });
    }

    let mut lines: Vec<String> = memories
        .iter()
        .map(|m| {
            let id = display_id(&m.id, full_ids);
            format!("[{id}] {} ({})", m.content, m.tags.join(", "))
        })
        .collect();
    lines.extend(footer);

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

/// Render a memory id for tool output: the 8-char prefix, or the full UUID
/// when `full_ids` is set. IDs are UUIDv7, so memories written within the same
/// ~65s share their first 8 chars and the prefix alone can be ambiguous (#1357).
fn display_id(id: &str, full: bool) -> &str {
    if full { id } else { id.get(..8).unwrap_or(id) }
}

/// Resolve an id argument to a full UUID (#1048).
///
/// Accepts the full UUID or any unambiguous prefix (e.g. the 8-char ids
/// printed by recall/list). Errors loudly on ambiguous prefixes instead of
/// silently no-oping. Exact UUIDs skip the prefix scan.
fn resolve_id<'a>(uteke: &'a Uteke, id: &'a str) -> Result<String, String> {
    // The shared resolver under the MCP policy (#1343): hex-and-dash ids
    // only (#1328), full UUIDs normalised, no existence check.
    uteke_core::resolve_memory_id(uteke, id).map_err(|e| e.message().to_string())
}

/// #1053: mark old_id superseded by new_id — wires the edge pair
/// (superseded_by / supersedes) and soft-deprecates the old memory.
fn exec_supersede(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let old_arg = args["old_id"].as_str().ok_or("Missing 'old_id'")?;
    let new_arg = args["new_id"].as_str().ok_or("Missing 'new_id'")?;
    let reason = args["reason"].as_str();
    let old_id = resolve_id(uteke, old_arg)?;
    let new_id = resolve_id(uteke, new_arg)?;

    let (o, n) = uteke
        .supersede(&old_id, &new_id, reason)
        .map_err(|e| format!("Failed: {e}"))?;

    let mut text = format!("✓ Superseded {o} → {n}");
    text.push_str("\n  old memory soft-deprecated (restore: uteke lifecycle promote)");
    text.push_str("\n  recall results now flag the pair until the old row is pruned");
    if let Some(r) = reason {
        text.push_str(&format!("\n  reason: {r}"));
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

/// #1049: read a single memory by id (or unambiguous prefix) — full record,
/// no truncation. recall/search return ranked excerpts; list truncates.
fn exec_get(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let id = resolve_id(uteke, id_arg)?;

    let m = uteke
        .get_by_id(&id)
        .map_err(|e| format!("Failed: {e}"))?
        .ok_or_else(|| format!("Memory not found: {id}"))?;

    let body = serde_json::json!({
        "id": m.id,
        "content": m.content,
        "tags": m.tags,
        "metadata": m.metadata,
        "namespace": m.namespace,
        "memory_type": m.memory_type,
        "importance": m.importance,
        "pinned": m.pinned,
        "deprecated": m.deprecated,
        "created_at": m.created_at,
        "updated_at": m.updated_at,
        "last_accessed": m.last_accessed,
        "access_count": m.access_count,
        "source": m.source,
    });

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: serde_json::to_string_pretty(&body)
                .map_err(|e| format!("Serialization failed: {e}"))?,
        }],
        is_error: false,
    })
}

/// #1049: partial update — same semantics as HTTP PUT /memory (#659):
/// only provided fields change; content changes regenerate the embedding.
fn exec_update(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let id = resolve_id(uteke, id_arg)?;

    let content = args["content"].as_str();
    let importance = args["importance"].as_f64();
    let pinned = args["pinned"].as_bool();
    let memory_type = args["memory_type"].as_str();
    let tags: Option<Vec<String>> = args["tags"].as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    });
    let tags_ref = tags.as_deref();
    let metadata = args.get("metadata").filter(|m| !m.is_null());

    let outcome = uteke
        .update_memory_detailed(
            &id,
            content,
            tags_ref,
            metadata,
            importance,
            pinned,
            memory_type,
        )
        .map_err(write_error)?;

    if !outcome.updated {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Memory not found: {id}"),
            }],
            is_error: true,
        });
    }

    // Namespace move (#1181) — plain column update, no re-embed needed.
    if let Some(ns) = args["namespace"].as_str() {
        let moved = uteke
            .move_memory(&id, ns)
            .map_err(|e| format!("Failed: {e}"))?;
        if !moved {
            return Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text: format!("Memory not found: {id}"),
                }],
                is_error: true,
            });
        }
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!(
                "✓ Updated memory {id}{}{}",
                outcome
                    .redaction_summary()
                    .map(|s| format!("\n⚠ Secrets masked in the stored text (#1464): {s}"))
                    .unwrap_or_default(),
                namespace_deprecation_suffix(args["namespace"].as_str())
            ),
        }],
        is_error: false,
    })
}

fn exec_forget(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    // The tool takes an id only: a `tag` or `namespace` is refused rather than
    // ignored, so an extra key can never widen the target. The id is resolved
    // by the typed request shared with HTTP and the CLI (#1343, #1439).
    reject_unknown_args(&tool_forget(), args, &[])?;
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let request = uteke_core::ForgetRequest::decode(uteke_core::ForgetInput {
        id: Some(id_arg.to_string()),
        ..Default::default()
    })
    .map_err(|e| e.message().to_string())?;
    let id = request
        .resolve(uteke)
        .map_err(|e| e.message().to_string())?
        .ok_or("Missing 'id'")?;

    uteke.forget(&id).map_err(|e| format!("Failed: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("✓ Forgotten: {id}"),
        }],
        is_error: false,
    })
}

fn exec_stats(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();

    let stats = uteke.stats(namespace).map_err(|e| format!("Failed: {e}"))?;

    // #1052: agents need triage fields on one call — tiers, pinned/deprecated
    // split, and (when unscoped) the per-namespace breakdown.
    let pinned = uteke.count_pinned(namespace).unwrap_or(0);
    let deprecated = uteke.count_deprecated(namespace).unwrap_or(0);

    let mut lines = vec![
        format!(
            "Total: {} | Tags: {} | DB: {} bytes",
            stats.total_memories, stats.unique_tags, stats.db_size_bytes
        ),
        format!(
            "Tiers: hot {} | warm {} | cold {}",
            stats.hot, stats.warm, stats.cold
        ),
        format!(
            "Pinned: {} | Deprecated (soft-deleted): {}",
            pinned, deprecated
        ),
        format!(
            "Recall cache: {} hits / {} misses",
            stats.cache_hits, stats.cache_misses
        ),
    ];

    if namespace.is_none() {
        if let Ok(ns_counts) = uteke.namespace_counts() {
            if ns_counts.len() > 1 {
                lines.push("Namespaces:".to_string());
                for (ns, count) in ns_counts {
                    lines.push(format!("  {ns}: {count}"));
                }
            }
        }
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_search(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let query = args["query"].as_str().ok_or("Missing 'query'")?;
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let namespace = args["namespace"].as_str();

    let tags_filter: Option<Vec<&str>> = args["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>());
    let tags_ref = tags_filter.as_deref();

    let results = uteke
        .search(query, limit, tags_ref, namespace)
        .map_err(|e| format!("Failed: {e}"))?;

    if results.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No memories found.".to_string(),
            }],
            is_error: false,
        });
    }

    let lines: Vec<String> = results
        .iter()
        .map(|sr| format!("[{:.2}] {}", sr.score, sr.memory.content))
        .collect();

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_doc_create(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let slug = args["slug"].as_str().ok_or("Missing 'slug'")?;
    let content = args["content"].as_str().ok_or("Missing 'content'")?;
    let title = args["title"].as_str().unwrap_or("");
    let parent = args["parent"].as_str();
    let tags: Vec<&str> = args["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let id = uteke
        .doc_upsert_with_parent(slug, title, content, &tags, None, parent)
        .map_err(|e| format!("Failed: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("✓ Document '{slug}' stored (id: {id})"),
        }],
        is_error: false,
    })
}

fn exec_doc_update(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id = args["id"].as_str().ok_or("Missing 'id'")?;
    let title = args["title"].as_str();
    let content = args["content"].as_str();
    let tags: Option<Vec<String>> = args["tags"].as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    });
    let metadata = args.get("metadata").filter(|v| !v.is_null()).cloned();

    match uteke.doc_update(id, title, content, tags.as_deref(), metadata.as_ref()) {
        Ok(Some(doc)) => {
            let chunks_hint = if content.is_some() {
                " (chunks rebuilt)"
            } else {
                ""
            };
            Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text: format!(
                        "✓ Document '{}' updated to v{}{chunks_hint}",
                        doc.slug, doc.version
                    ),
                }],
                is_error: false,
            })
        }
        Ok(None) => Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Document '{id}' not found"),
            }],
            is_error: false,
        }),
        Err(e) => Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Error: {e}"),
            }],
            is_error: true,
        }),
    }
}

fn exec_doc_get(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_or_slug = args["id_or_slug"].as_str().ok_or("Missing 'id_or_slug'")?;

    let doc = uteke
        .doc_get(id_or_slug)
        .map_err(|e| format!("Failed: {e}"))?;

    match doc {
        Some(d) => Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("# {}\n\n{}", d.title, d.content),
            }],
            is_error: false,
        }),
        None => Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Document '{id_or_slug}' not found"),
            }],
            is_error: false,
        }),
    }
}

fn exec_doc_list(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let limit = args["limit"].as_u64().unwrap_or(20) as usize;
    let namespace = args["namespace"].as_str();

    let docs = uteke
        .doc_list(namespace, limit)
        .map_err(|e| format!("Failed: {e}"))?;

    if docs.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No documents found.".to_string(),
            }],
            is_error: false,
        });
    }

    let lines: Vec<String> = docs
        .iter()
        .map(|d| format!("{} — {} (v{})", d.slug, d.title, d.version))
        .collect();

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_doc_search(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let query = args["query"].as_str().ok_or("Missing 'query'")?;
    let mode = args["mode"].as_str().unwrap_or("hybrid");
    let limit = args["limit"].as_u64().unwrap_or(5) as usize;

    let results = uteke
        .doc_search(query, limit, mode)
        .map_err(|e| format!("Failed: {e}"))?;

    if results.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No documents found.".to_string(),
            }],
            is_error: false,
        });
    }

    let lines: Vec<String> = results
        .iter()
        .map(|d| {
            // #1052: score + chunk snippet make ranking visible and results
            // actionable (was: bare "slug — title").
            let mut line = format!(
                "[{:.2}] {} — {}",
                d.score, d.document.slug, d.document.title
            );
            if !d.chunk_heading.is_empty() {
                line.push_str(&format!(" § {}", d.chunk_heading));
            }
            if !d.chunk_snippet.is_empty() {
                let snip: String = d.chunk_snippet.chars().take(120).collect();
                line.push_str(&format!(" \"{snip}…\""));
            }
            line
        })
        .collect();

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_doc_delete(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id = args["id"].as_str().ok_or("Missing 'id'")?;

    let (deleted, chunks) = uteke.doc_delete(id).map_err(|e| format!("Failed: {e}"))?;

    if deleted {
        Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("✓ Deleted document: {id} ({chunks} chunks removed)"),
            }],
            is_error: false,
        })
    } else {
        Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Document not found: {id}"),
            }],
            is_error: false,
        })
    }
}

fn exec_doc_move(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id = args["id"].as_str().ok_or("Missing 'id'")?;
    let parent = args["parent"].as_str();

    let moved = uteke
        .doc_move(id, parent, None)
        .map_err(|e| format!("Failed: {e}"))?;

    let msg = match parent {
        Some(p) => format!("Moved document: {id} -> parent: {p} ({moved} row(s) updated)"),
        None => format!("Moved document: {id} -> root ({moved} row(s) updated)"),
    };

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: msg,
        }],
        is_error: false,
    })
}

fn exec_graph(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();
    let limit = args["limit"]
        .as_u64()
        .map(|v| v as usize)
        .unwrap_or(uteke_core::graph_view::GRAPH_VIEW_DEFAULT_LIMIT);

    let view = uteke
        .graph_view(namespace, None, limit)
        .map_err(|e| format!("Failed: {e}"))?;

    let mut text = format!(
        "Graph: {} nodes, {} edges, {} relation types{}",
        view.stats.node_count,
        view.stats.edge_count,
        view.stats.relation_types.len(),
        if view.truncated {
            " (truncated; raise 'limit')"
        } else {
            ""
        }
    );
    // Show the most recent edges so the agent can actually use the graph.
    let label: std::collections::HashMap<&str, &str> = view
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    for e in view.edges.iter().take(15) {
        text.push_str(&format!(
            "\n- {} -[{}]-> {}",
            label
                .get(e.source_id.as_str())
                .unwrap_or(&e.source_id.as_str()),
            e.relation,
            label
                .get(e.target_id.as_str())
                .unwrap_or(&e.target_id.as_str())
        ));
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_graph_add_edge(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let source_arg = args["source"].as_str().ok_or("Missing 'source'")?;
    let source = resolve_id(uteke, source_arg)?;
    let target_arg = args["target"].as_str().ok_or("Missing 'target'")?;
    let target = resolve_id(uteke, target_arg)?;
    let edge_type = args["edge_type"].as_str().unwrap_or("related");
    let weight = args["weight"].as_f64().unwrap_or(1.0);

    if source == target {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "Error: self-loop edges are not allowed (source == target)".to_string(),
            }],
            is_error: true,
        });
    }

    // Validate both memories exist
    match uteke.get_by_id(&source) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text: format!("Error: source memory not found: {source}"),
                }],
                is_error: true,
            });
        }
        Err(e) => return Err(format!("Failed: {e}")),
    }
    match uteke.get_by_id(&target) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text: format!("Error: target memory not found: {target}"),
                }],
                is_error: true,
            });
        }
        Err(e) => return Err(format!("Failed: {e}")),
    }

    let gs = uteke.graph();
    gs.add_edge_for_memories(&source, &target, edge_type, weight)
        .map_err(|e| format!("Failed: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("✓ Added edge: {source} -[{edge_type}]-> {target}"),
        }],
        is_error: false,
    })
}

fn exec_graph_remove_edge(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let source_arg = args["source"].as_str().ok_or("Missing 'source'")?;
    let source = resolve_id(uteke, source_arg)?;
    let target_arg = args["target"].as_str().ok_or("Missing 'target'")?;
    let target = resolve_id(uteke, target_arg)?;

    let gs = uteke.graph();
    let removed = gs
        .remove_edge_between(&source, &target)
        .map_err(|e| format!("Failed: {e}"))?;

    if removed {
        Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("✓ Removed edge: {source} -> {target}"),
            }],
            is_error: false,
        })
    } else {
        Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Edge not found: {source} -> {target}"),
            }],
            is_error: true,
        })
    }
}

fn exec_context(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();

    let context = uteke
        .build_context(namespace)
        .map_err(|e| format!("Failed: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: context,
        }],
        is_error: false,
    })
}

fn exec_verify(uteke: &Uteke) -> Result<ToolResult, String> {
    let report = uteke.verify().map_err(|e| format!("Failed: {e}"))?;
    let text = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_repair(uteke: &Uteke) -> Result<ToolResult, String> {
    let report = uteke.repair().map_err(|e| format!("Failed: {e}"))?;
    let text = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_dream(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();
    // #1050: dry_run defaults to TRUE — a no-args call must never mutate.
    // Applying requires an explicit dry_run=false.
    let dry_run = args["dry_run"].as_bool().unwrap_or(true);
    let confirm_large = args["confirm_large"].as_bool().unwrap_or(false);
    const LARGE_BATCH_THRESHOLD: usize = 100;

    // Parse phases if specified.
    let phases: Vec<uteke_core::DreamPhase> = args["phases"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .filter_map(|s| match s {
                    "lint" => Some(uteke_core::DreamPhase::Lint),
                    "backlinks" => Some(uteke_core::DreamPhase::Backlinks),
                    "dedup" => Some(uteke_core::DreamPhase::Dedup),
                    "orphans" => Some(uteke_core::DreamPhase::Orphans),
                    "compact" => Some(uteke_core::DreamPhase::Compact),
                    "verify" => Some(uteke_core::DreamPhase::Verify),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    // Applying runs are guarded (#1050):
    // 1. Unscoped (namespace=None) applying runs must announce the scope —
    //    we require a namespace for applying runs unless confirm_large is
    //    also set, making "accidental whole-store maintenance" a two-flag
    //    decision instead of a default.
    // 2. Large batches (>100 projected changes) need confirm_large=true,
    //    else the run refuses and reports what it WOULD have done.
    if !dry_run && namespace.is_none() && !confirm_large {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "Refused: applying run without a namespace scope. Pass namespace=<ns> to scope the maintenance, or set confirm_large=true to apply across ALL namespaces.".to_string(),
            }],
            is_error: true,
        });
    }

    // First pass: always compute the DRY report to learn projected changes.
    let preview = uteke
        .dream(namespace, true, &phases)
        .map_err(|e| format!("Failed: {e}"))?;

    if !dry_run && preview.total_changes > LARGE_BATCH_THRESHOLD && !confirm_large {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!(
                    "Refused: applying run projects {} changes (> {LARGE_BATCH_THRESHOLD}). Re-run with confirm_large=true to apply, or keep dry_run=true to preview.",
                    preview.total_changes
                ),
            }],
            is_error: true,
        });
    }

    // Dry-run request (or nothing to apply) → return the preview report.
    if dry_run || preview.total_changes == 0 {
        let mut lines = vec![format!(
            "Dream dry-run preview: {} changes, {} warnings, {} errors ({}ms){}",
            preview.total_changes,
            preview.total_warnings,
            preview.total_errors,
            preview.duration_ms,
            if namespace.is_none() {
                " [SCOPE: ALL NAMESPACES]"
            } else {
                ""
            }
        )];
        for phase in &preview.phases {
            lines.push(format!(
                "  {}: {} changes, {} warnings",
                phase.phase, phase.changes, phase.warnings
            ));
        }
        if !dry_run && preview.total_changes == 0 {
            lines.push("Nothing to apply — no changes projected.".to_string());
        } else {
            lines.push(
                "To apply: dry_run=false (scope a namespace, or confirm_large=true for all-namespaces / >100 changes)."
                    .to_string(),
            );
        }
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: lines.join("\n"),
            }],
            is_error: false,
        });
    }

    // Applying run (scoped, or confirm_large, and under threshold / confirmed).
    let report = uteke
        .dream(namespace, false, &phases)
        .map_err(|e| format!("Failed: {e}"))?;

    let mut lines = vec![format!(
        "Dream cycle applied: {} changes, {} warnings, {} errors ({}ms){}",
        report.total_changes,
        report.total_warnings,
        report.total_errors,
        report.duration_ms,
        if namespace.is_none() {
            " [SCOPE: ALL NAMESPACES]"
        } else {
            ""
        }
    )];

    for phase in &report.phases {
        lines.push(format!(
            "  {}: {} changes, {} warnings",
            phase.phase, phase.changes, phase.warnings
        ));
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

/// Parsed arguments of `uteke_room_recall` (same names and defaults as
/// `uteke_recall` where they overlap).
#[derive(Debug, PartialEq)]
struct RoomRecallArgs {
    limit: usize,
    min_score: f32,
    tags: Vec<String>,
    pack: bool,
    budget_chars: usize,
    exclude_ids: Vec<String>,
    full_ids: bool,
}

fn parse_room_recall_args(args: &Value) -> RoomRecallArgs {
    let strings = |key: &str| -> Vec<String> {
        args[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    RoomRecallArgs {
        limit: args["limit"].as_u64().unwrap_or(5) as usize,
        min_score: args["min_score"].as_f64().unwrap_or(0.0).max(0.0) as f32,
        tags: strings("tags"),
        pack: args["pack"].as_bool().unwrap_or(false),
        budget_chars: args["budget_chars"].as_u64().unwrap_or(4000) as usize,
        exclude_ids: strings("exclude_ids"),
        full_ids: args["full_ids"].as_bool().unwrap_or(false),
    }
}

/// Plain-list output: the historical `[score] content` line, followed by an
/// indented `(id: …)` line so the result can be passed to `uteke_provenance`,
/// `uteke_get` or `exclude_ids`.
fn format_room_recall_lines(results: &[uteke_core::SearchResult], full_ids: bool) -> Vec<String> {
    let mut lines = Vec::with_capacity(results.len() * 2);
    for sr in results {
        lines.push(format!("[{:.2}] {}", sr.score, sr.memory.content));
        lines.push(format!(
            "       (id: {})",
            display_id(&sr.memory.id, full_ids)
        ));
    }
    lines
}

fn exec_room_recall(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;
    let query = args["query"].as_str().ok_or("Missing 'query'")?;
    let a = parse_room_recall_args(args);
    let tag_refs: Vec<&str> = a.tags.iter().map(String::as_str).collect();
    let tags = (!tag_refs.is_empty()).then_some(tag_refs.as_slice());

    let text_result = |text: String| ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    };

    // Budgeted context pack (#1335, same envelope as uteke_recall #1281).
    if a.pack {
        let pack = uteke
            .recall_room_packed(
                room_id,
                query,
                a.limit,
                None,
                a.min_score,
                tags,
                a.budget_chars,
                &a.exclude_ids,
            )
            .map_err(|e| format!("Failed: {e}"))?;
        // Nothing matched at all: say so. Otherwise return the pack even when
        // `selected` is empty, so `skipped` explains why (excluded / budget).
        if pack.selected.is_empty() && pack.skipped.is_empty() {
            return Ok(text_result("No memories found in room.".to_string()));
        }
        return Ok(text_result(
            serde_json::to_string_pretty(&pack).unwrap_or_else(|_| "{}".to_string()),
        ));
    }

    let results = uteke
        .recall_room_semantic_filtered(room_id, query, a.limit, None, a.min_score, tags)
        .map_err(|e| format!("Failed: {e}"))?;

    if results.is_empty() {
        return Ok(text_result("No memories found in room.".to_string()));
    }
    Ok(text_result(
        format_room_recall_lines(&results, a.full_ids).join("\n"),
    ))
}

fn exec_room_memories(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;
    let author = args["author"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(100) as usize;
    let full_ids = args["full_ids"].as_bool().unwrap_or(false);

    let memories = uteke
        .recall_room(room_id, author, limit)
        .map_err(|e| format!("Failed: {e}"))?;

    if memories.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No memories found in room.".to_string(),
            }],
            is_error: false,
        });
    }

    let lines: Vec<String> = memories
        .iter()
        .map(|m| {
            // #1052/#1048: include the short id so the next tool call
            // (pin/forget/graph edges) can act on the row directly.
            let created = m.created_at.format("%Y-%m-%d %H:%M");
            let id = display_id(&m.id, full_ids);
            format!("[{created} | {} | {}] {}", id, m.namespace, m.content)
        })
        .collect();
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_room_create(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;
    let title = args["title"].as_str();
    let namespace = args["namespace"].as_str().unwrap_or("default");

    uteke
        .create_room(room_id, title, namespace)
        .map_err(|e| format!("Failed to create room: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!(
                "Room created: {room_id} (namespace: {namespace}){}",
                namespace_deprecation_suffix(Some(namespace))
            ),
        }],
        is_error: false,
    })
}

/// Keep rooms whose id or title contains `name` (case-insensitive, #1336).
/// A blank `name` keeps everything.
fn filter_rooms_by_name(rooms: Vec<uteke_core::Room>, name: Option<&str>) -> Vec<uteke_core::Room> {
    let needle = name.map(|n| n.trim().to_lowercase()).unwrap_or_default();
    if needle.is_empty() {
        return rooms;
    }
    rooms
        .into_iter()
        .filter(|r| {
            r.id.to_lowercase().contains(&needle)
                || r.title
                    .as_deref()
                    .is_some_and(|t| t.to_lowercase().contains(&needle))
        })
        .collect()
}

fn exec_room_list(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();

    let rooms = filter_rooms_by_name(
        uteke
            .list_rooms(namespace)
            .map_err(|e| format!("Failed: {e}"))?,
        args["name"].as_str(),
    );

    if rooms.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No rooms found.".to_string(),
            }],
            is_error: false,
        });
    }

    let lines: Vec<String> = rooms
        .iter()
        .map(|r| {
            let title = r.title.as_deref().unwrap_or("(no title)");
            format!("[{}] {} (ns: {})", r.id, title, r.namespace)
        })
        .collect();

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("Rooms ({}):\n{}", rooms.len(), lines.join("\n")),
        }],
        is_error: false,
    })
}

fn exec_room_delete(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;

    let unlinked = uteke
        .delete_room(room_id)
        .map_err(|e| format!("Failed to delete room: {e}"))?;

    let text = format!(
        "Room '{room_id}' unregistered. {unlinked} memory link(s) removed; the memories are retained, no longer linked to any room. To delete a memory, use uteke_forget with its id."
    );

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_room_rename(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let from = args["from"].as_str().ok_or("Missing 'from'")?;
    let to = args["to"].as_str().ok_or("Missing 'to'")?;

    uteke
        .rename_room(from, to)
        .map_err(|e| format!("Failed to rename room: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("✓ Room '{from}' renamed to '{to}' — all links moved atomically"),
        }],
        is_error: false,
    })
}

fn exec_room_update(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;
    let title = args["title"].as_str();
    let description = args["description"].as_str();

    let room = uteke
        .update_room(room_id, title, description)
        .map_err(|e| format!("Failed to update room: {e}"))?
        .ok_or_else(|| format!("Room not found: {room_id}"))?;

    let title_out = room.title.as_deref().unwrap_or("(none)");
    let desc_out = room.description.as_deref().unwrap_or("(none)");
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!(
                "✓ Room '{room_id}' updated — title: {title_out}, description: {desc_out}"
            ),
        }],
        is_error: false,
    })
}

fn exec_room_memory_move(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let memory_id = args["memory_id"].as_str().ok_or("Missing 'memory_id'")?;
    let from_room = args["from_room"].as_str().ok_or("Missing 'from_room'")?;
    let to_room = args["to_room"].as_str().ok_or("Missing 'to_room'")?;

    let moved = uteke
        .move_memory_to_room(memory_id, from_room, to_room)
        .map_err(|e| format!("Failed to move memory: {e}"))?;
    if moved == 0 {
        return Err(format!(
            "Memory '{memory_id}' has no link in room '{from_room}'"
        ));
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("✓ Memory '{memory_id}' moved from '{from_room}' to '{to_room}'"),
        }],
        is_error: false,
    })
}

fn exec_room_stats(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;

    let stats = uteke
        .room_stats(room_id)
        .map_err(|e| format!("Failed: {e}"))?;

    let stats = match stats {
        Some(s) => s,
        None => {
            return Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text: format!("Room not found: {room_id}"),
                }],
                is_error: false,
            });
        }
    };

    let text = format!(
        "Room: {} (title: {})\nMemories: {}\nParticipants ({}): {}\nCreated: {}\nLast activity: {}",
        stats.room_id,
        stats.title.as_deref().unwrap_or("(none)"),
        stats.memory_count,
        stats.participant_count,
        stats.participants.join(", "),
        stats.created_at,
        stats.last_activity.as_deref().unwrap_or("N/A"),
    );

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_room_summary(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;

    let summary = uteke
        .room_summary(room_id)
        .map_err(|e| format!("Failed: {e}"))?;

    let summary = match summary {
        Some(s) => s,
        None => {
            return Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text: format!("Room not found: {room_id}"),
                }],
                is_error: false,
            });
        }
    };

    let mut lines = vec![format!(
        "Room: {} — {} memories, {} participants ({}..{})",
        summary.room_id,
        summary.total_memories,
        summary.participants.len(),
        summary.time_range.earliest,
        summary.time_range.latest,
    )];

    if !summary.clusters.is_empty() {
        lines.push("".to_string());
        lines.push("Topic Clusters:".to_string());
        for c in &summary.clusters {
            lines.push(format!(
                "  [{:.1}] {} ({} memories, tags: {})",
                c.score,
                c.topic,
                c.memory_count,
                c.tags.join(", "),
            ));
        }
    }

    if !summary.recent_decisions.is_empty() {
        lines.push("".to_string());
        lines.push("Recent Decisions:".to_string());
        for d in &summary.recent_decisions {
            lines.push(format!("  - {d}"));
        }
    }

    if !summary.pinned_highlights.is_empty() {
        lines.push("".to_string());
        lines.push("Pinned Highlights:".to_string());
        for h in &summary.pinned_highlights {
            lines.push(format!("  * {h}"));
        }
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_room_document(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;

    let doc = uteke
        .room_summary_document(room_id)
        .map_err(|e| format!("Failed: {e}"))?;

    let doc = match doc {
        Some(d) => d,
        None => {
            return Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text: format!("Room not found: {room_id}"),
                }],
                is_error: false,
            });
        }
    };

    let mut lines = vec![format!(
        "Document for: {} (generated: {})",
        doc.room_id, doc.generated_at,
    )];

    for section in &doc.sections {
        lines.push("".to_string());
        lines.push(format!("{} {}", section.icon, section.heading));
        for entry in &section.entries {
            lines.push(format!(
                "  [{}] {} — {}",
                entry.author, entry.created_at, entry.content,
            ));
            if !entry.tags.is_empty() {
                lines.push(format!("    tags: {}", entry.tags.join(", ")));
            }
        }
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_room_add_document(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;
    let doc_slug = args["doc_slug"].as_str().ok_or("Missing 'doc_slug'")?;

    uteke
        .room_add_document(room_id, doc_slug)
        .map_err(|e| format!("Failed: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("Linked document '{doc_slug}' to room '{room_id}'."),
        }],
        is_error: false,
    })
}

fn exec_room_remove_document(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;
    let doc_slug = args["doc_slug"].as_str().ok_or("Missing 'doc_slug'")?;

    uteke
        .room_remove_document(room_id, doc_slug)
        .map_err(|e| format!("Failed: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("Unlinked document '{doc_slug}' from room '{room_id}'."),
        }],
        is_error: false,
    })
}

fn exec_room_list_documents(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;

    let docs = uteke
        .room_list_documents(room_id)
        .map_err(|e| format!("Failed: {e}"))?;

    let text = if docs.is_empty() {
        format!("No documents linked to room '{room_id}'.")
    } else {
        format!(
            "Documents linked to room '{}':\n{}",
            room_id,
            docs.iter()
                .map(|s| format!("  • {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_doc_list_rooms(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let doc_slug = args["doc_slug"].as_str().ok_or("Missing 'doc_slug'")?;

    let rooms = uteke
        .document_list_rooms(doc_slug)
        .map_err(|e| format!("Failed: {e}"))?;

    let text = if rooms.is_empty() {
        format!("No rooms reference document '{doc_slug}'.")
    } else {
        format!(
            "Rooms referencing document '{}':\n{}",
            doc_slug,
            rooms
                .iter()
                .map(|s| format!("  • {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_tags_list(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();
    let sort = args["sort"].as_str().unwrap_or("count");

    let mut tags = uteke
        .tags_with_counts(namespace)
        .map_err(|e| format!("Failed: {e}"))?;

    if sort == "alpha" {
        tags.sort_by(|a, b| a.name.cmp(&b.name));
    }

    if tags.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No tags found.".to_string(),
            }],
            is_error: false,
        });
    }

    let lines: Vec<String> = tags
        .iter()
        .map(|t| format!("{} ({})", t.name, t.count))
        .collect();

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_tags_rename(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let old_tag = args["old_tag"].as_str().ok_or("Missing 'old_tag'")?;
    let new_tag = args["new_tag"].as_str().ok_or("Missing 'new_tag'")?;
    let namespace = args["namespace"].as_str();

    let count = uteke
        .rename_tag(old_tag, new_tag, namespace)
        .map_err(|e| format!("Failed: {e}"))?;

    if count == 0 {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Tag '{}' not found in scope.", old_tag),
            }],
            is_error: true,
        });
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!(
                "Renamed tag '{}' -> '{}' ({} memories updated)",
                old_tag, new_tag, count
            ),
        }],
        is_error: false,
    })
}

fn exec_tags_delete(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let tag = args["tag"].as_str().ok_or("Missing 'tag'")?;
    let namespace = args["namespace"].as_str();

    let count = uteke
        .delete_tag(tag, namespace)
        .map_err(|e| format!("Failed: {e}"))?;

    if count == 0 {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Tag '{}' not found in scope.", tag),
            }],
            is_error: true,
        });
    }

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("Deleted tag '{}' ({} memories updated)", tag, count),
        }],
        is_error: false,
    })
}

/// Rename a namespace, merging into an existing target (#1181).
fn exec_namespace_rename(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let from = args["from"].as_str().ok_or("Missing 'from'")?;
    let to = args["to"].as_str().ok_or("Missing 'to'")?;

    let result = uteke
        .rename_namespace(from, to)
        .map_err(|e| format!("Failed: {e}"))?;

    let kind = if result.target_existed {
        "merged into existing"
    } else {
        "renamed to"
    };
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!(
                "✓ Namespace '{}' {} '{}' — {} memories moved",
                result.from, kind, result.to, result.moved
            ),
        }],
        is_error: false,
    })
}

/// Delete a namespace with an explicit memory-fate strategy (#1181).
fn exec_namespace_delete(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let name = args["name"].as_str().ok_or("Missing 'name'")?;
    let strategy = args["strategy"].as_str().unwrap_or("refuse");
    let target = args["target"].as_str();

    let result = uteke
        .delete_namespace(name, strategy, target)
        .map_err(|e| format!("Failed: {e}"))?;

    let text = match result.strategy.as_str() {
        "merge" => format!(
            "✓ Moved {} memories from '{}' to '{}' — namespace removed",
            result.affected,
            result.name,
            result.target.as_deref().unwrap_or("?")
        ),
        "deprecate" => format!(
            "✓ Soft-deleted {} memories in '{}' (restorable via promote; the name stays visible as deprecated-only)",
            result.affected, result.name
        ),
        other => format!("✓ Namespace '{}' deleted (strategy={other})", result.name),
    };
    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_pin(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let id = resolve_id(uteke, id_arg)?;

    match uteke.pin(&id) {
        Ok(true) => Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Pinned memory: {id}"),
            }],
            is_error: false,
        }),
        Ok(false) => Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Memory not found: {id}"),
            }],
            is_error: true,
        }),
        Err(e) => Err(format!("Failed: {e}")),
    }
}

fn exec_unpin(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let id = resolve_id(uteke, id_arg)?;

    match uteke.unpin(&id) {
        Ok(true) => Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Unpinned memory: {id}"),
            }],
            is_error: false,
        }),
        Ok(false) => Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: format!("Memory not found: {id}"),
            }],
            is_error: true,
        }),
        Err(e) => Err(format!("Failed: {e}")),
    }
}

#[cfg(test)]
mod id_resolution_tests {
    use super::*;
    use uteke_core::Uteke;
    use uteke_core::memory::types::Memory;

    fn scratch() -> (Uteke, std::path::PathBuf) {
        // Unique per call (tests run in parallel threads; shared names hit
        // "database busy" on WAL setup).
        let dir = std::env::temp_dir().join(format!(
            "mcp-id-{}-{}-{}",
            module_path!().len(), // stable per test module
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();
        (uteke, dir)
    }

    fn seed(uteke: &Uteke) -> String {
        // Direct store insert (remember_precomputed is pub(crate)); the MCP
        // executor under test only reads/resolves/updates, which is the
        // behavior under test here.
        let id = uuid::Uuid::new_v4().to_string();
        let m = Memory {
            id: id.clone(),
            content: "id resolution probe content".to_string(),
            embedding: vec![0.21; 768],
            tags: vec!["probe".to_string()],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "mcp-id-ns".to_string(),
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
            author: None,
        };
        uteke.store_for_tests().insert(&m).unwrap();
        id
    }

    #[test]
    fn resolve_id_accepts_prefix_and_full() {
        let (uteke, dir) = scratch();
        let id = seed(&uteke);
        let short: String = id.chars().take(8).collect();

        assert_eq!(resolve_id(&uteke, &id).unwrap(), id);
        assert_eq!(resolve_id(&uteke, &short).unwrap(), id);
        assert!(
            resolve_id(&uteke, "00000000").is_err(),
            "unknown prefix errors"
        );
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exec_get_returns_full_record() {
        let (uteke, dir) = scratch();
        let id = seed(&uteke);
        let short: String = id.chars().take(8).collect();

        let result = exec_get(&uteke, &serde_json::json!({"id": short})).unwrap();
        assert!(!result.is_error);
        let text = match &result.content[0] {
            McpContent::Text { text, .. } => text.clone(),
        };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["id"].as_str().unwrap(), id);
        assert_eq!(
            v["content"].as_str().unwrap(),
            "id resolution probe content"
        );
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_id_rejects_malformed_ids() {
        let (uteke, dir) = scratch();
        // 36 chars but not a UUID: must not be passed through as a full id.
        let fake = "z".repeat(36);
        assert!(resolve_id(&uteke, &fake).unwrap_err().contains("Invalid"));
        // LIKE wildcards never reach the prefix scan.
        assert!(resolve_id(&uteke, "%").is_err());
        assert!(resolve_id(&uteke, "ab_").is_err());
        assert!(resolve_id(&uteke, "").is_err());
        // 36 hex/dash chars that are not a UUID shape fall to the prefix path.
        let almost = "0".repeat(36);
        assert!(
            resolve_id(&uteke, &almost)
                .unwrap_err()
                .contains("Memory not found")
        );
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn display_id_prefix_or_full() {
        let id = "01a115a7-aaaa-7bbb-8ccc-0123456789ab";
        assert_eq!(display_id(id, false), "01a115a7");
        assert_eq!(display_id(id, true), id);
        assert_eq!(display_id("abc", false), "abc");
    }

    #[test]
    fn list_and_room_memories_full_ids_round_trip() {
        let (uteke, dir) = scratch();
        let id = seed(&uteke);
        let short = &id[..8];

        let text = |r: ToolResult| match &r.content[0] {
            McpContent::Text { text, .. } => text.clone(),
            #[allow(unreachable_patterns)]
            _ => String::new(),
        };

        let default = text(exec_list(&uteke, &serde_json::json!({})).unwrap());
        assert!(default.contains(&format!("[{short}]")), "{default}");
        assert!(!default.contains(&id));

        let full = text(exec_list(&uteke, &serde_json::json!({"full_ids": true})).unwrap());
        assert!(full.contains(&format!("[{id}]")), "{full}");

        // The full id printed by the tool is accepted by resolve_id as-is.
        assert_eq!(resolve_id(&uteke, &id).unwrap(), id);
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exec_pin_accepts_short_id() {
        let (uteke, dir) = scratch();
        let id = seed(&uteke);
        let short: String = id.chars().take(8).collect();

        let result = exec_pin(&uteke, &serde_json::json!({"id": short})).unwrap();
        assert!(!result.is_error, "short id must pin");
        let m = uteke.get_by_id(&id).unwrap().unwrap();
        assert!(m.pinned);
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exec_forget_short_id_no_longer_silent_noop() {
        let (uteke, dir) = scratch();
        let id = seed(&uteke);
        let short: String = id.chars().take(8).collect();

        // Happy path: short id resolves and forgets.
        let result = exec_forget(&uteke, &serde_json::json!({"id": short})).unwrap();
        assert!(!result.is_error);

        // Bogus prefix must now ERROR (was: silent "not found" no-op pre-fix).
        let err = exec_forget(&uteke, &serde_json::json!({"id": "ffffffff"}));
        assert!(err.is_err(), "unknown prefix must error loudly");
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    // #1343: the typed forget request keeps the MCP texts and policy.
    #[test]
    fn exec_forget_errors_keep_their_texts() {
        let (uteke, dir) = scratch();
        let id = seed(&uteke);
        let err = |args: serde_json::Value| exec_forget(&uteke, &args).err().unwrap();

        assert_eq!(err(serde_json::json!({})), "Missing 'id'");
        assert_eq!(err(serde_json::json!({"id": 7})), "Missing 'id'");
        assert_eq!(
            err(serde_json::json!({"id": "%"})),
            "Invalid memory id '%' (expected a UUID or hex prefix)"
        );
        assert_eq!(
            err(serde_json::json!({"id": ""})),
            "Invalid memory id '' (expected a UUID or hex prefix)"
        );
        assert_eq!(
            err(serde_json::json!({"id": "ffffffff"})),
            "Memory not found: ffffffff"
        );
        // A full UUID that is not stored is refused too (#762), not a silent no-op.
        let absent = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            err(serde_json::json!({"id": absent})),
            format!("Memory not found: {absent}")
        );
        // The tool has no tag or namespace: extra keys are refused and never
        // widen the target.
        assert_eq!(
            err(serde_json::json!({"tag": "probe", "namespace": "mcp-id-ns"})),
            "Unknown argument 'namespace'. Supported: id"
        );
        assert_eq!(
            err(serde_json::json!({"id": "ffffffff", "tag": "probe"})),
            "Unknown argument 'tag'. Supported: id"
        );
        assert!(uteke.get_by_id(&id).unwrap().is_some());
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exec_forget_normalises_full_uuid_forms() {
        let (uteke, dir) = scratch();
        let id = seed(&uteke);
        let upper = id.to_uppercase();
        let result = exec_forget(&uteke, &serde_json::json!({"id": upper})).unwrap();
        let McpContent::Text { text, .. } = &result.content[0];
        assert_eq!(text, &format!("✓ Forgotten: {id}"));
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exec_update_partial_fields() {
        let (uteke, dir) = scratch();
        let id = seed(&uteke);
        let short: String = id.chars().take(8).collect();

        // metadata-only update: content unchanged
        let result = exec_update(
            &uteke,
            &serde_json::json!({"id": short, "importance": 0.9, "pinned": true}),
        )
        .unwrap();
        assert!(!result.is_error);
        let m = uteke.get_by_id(&id).unwrap().unwrap();
        assert!((m.importance - 0.9).abs() < 1e-9);
        assert!(m.pinned);
        assert_eq!(
            m.content, "id resolution probe content",
            "content untouched"
        );
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod supersession_mcp_tests {
    use super::*;
    use uteke_core::Uteke;

    #[test]
    fn exec_supersede_via_short_ids() {
        let dir = std::env::temp_dir().join(format!(
            "ssmcp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();

        let mk = |content: &str| -> String {
            use uteke_core::memory::types::Memory;
            let id = uuid::Uuid::new_v4().to_string();
            let m = Memory {
                id: id.clone(),
                content: content.to_string(),
                embedding: vec![0.5; 768],
                tags: vec![],
                metadata: serde_json::json!({}),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                namespace: "ssmcp-ns".to_string(),
                access_count: 0,
                recall_count: 0,
                last_accessed: None,
                deprecated: false,
                deprecated_at: None,
                valid_from: None,
                valid_until: None,
                memory_type: "decision".to_string(),
                importance: 0.5,
                pinned: false,
                content_type: "text".to_string(),
                slug: None,
                source: None,
                source_type: "user".to_string(),

                author_type: "agent".to_string(),
                author: None,
            };
            uteke.store_for_tests().insert(&m).unwrap();
            id
        };
        let old = mk("old auth decision");
        let new = mk("new auth decision");
        let old_short: String = old.chars().take(8).collect();
        let new_short: String = new.chars().take(8).collect();

        let result = exec_supersede(
            &uteke,
            &serde_json::json!({"old_id": old_short, "new_id": new_short, "reason": "pivot"}),
        )
        .unwrap();
        assert!(!result.is_error);

        // Edge + deprecation landed.
        assert_eq!(
            uteke.supersession_of(&old).unwrap().as_deref(),
            Some(new.as_str())
        );
        assert!(uteke.get_by_id(&old).unwrap().unwrap().deprecated);

        // Self-supersession refused loudly.
        let err = exec_supersede(
            &uteke,
            &serde_json::json!({"old_id": &old_short, "new_id": &old_short}),
        );
        assert!(err.is_err());

        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod contradiction_mcp_tests {
    use super::*;
    use uteke_core::Uteke;

    #[test]
    fn exec_contradictions_list_and_undo_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "scmcp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let uteke = Uteke::open(dir.join("t.db").to_str().unwrap()).unwrap();

        let mk = |content: &str| -> String {
            use uteke_core::memory::types::Memory;
            let id = uuid::Uuid::new_v4().to_string();
            let m = Memory {
                id: id.clone(),
                content: content.to_string(),
                embedding: vec![0.5; 768],
                tags: vec![],
                metadata: serde_json::json!({}),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                namespace: "scmcp-ns".to_string(),
                access_count: 0,
                recall_count: 0,
                last_accessed: None,
                deprecated: false,
                deprecated_at: None,
                valid_from: None,
                valid_until: None,
                memory_type: "decision".to_string(),
                importance: 0.5,
                pinned: false,
                content_type: "text".to_string(),
                slug: None,
                source: None,
                source_type: "user".to_string(),
                author_type: "agent".to_string(),
                author: None,
            };
            uteke.store_for_tests().insert(&m).unwrap();
            id
        };
        let old = mk("old decision");
        let new = mk("new decision");
        uteke.supersede(&old, &new, Some("pivot")).unwrap();

        // Ledger lists the superseded memory.
        let result = exec_contradictions(
            &uteke,
            &serde_json::json!({"namespace": "scmcp-ns", "limit": 50}),
        )
        .unwrap();
        assert!(!result.is_error);
        let text = match &result.content[0] {
            McpContent::Text { text, .. } => text.clone(),
            #[allow(unreachable_patterns)]
            _ => panic!("expected text content"),
        };
        let ledger: serde_json::Value = serde_json::from_str(&text).unwrap();
        let entries = ledger.as_array().unwrap();
        assert_eq!(entries.len(), 1, "one entry expected: {text}");
        assert_eq!(entries[0]["id"], serde_json::json!(old));

        // Undo via short id (resolution contract).
        let old_short: String = old.chars().take(8).collect();
        let undone =
            exec_contradictions_undo(&uteke, &serde_json::json!({"id": old_short})).unwrap();
        assert!(!undone.is_error);

        // Ledger empty after undo; the memory is active again.
        let result = exec_contradictions(&uteke, &serde_json::json!({})).unwrap();
        let text = match &result.content[0] {
            McpContent::Text { text, .. } => text.clone(),
            #[allow(unreachable_patterns)]
            _ => panic!("expected text content"),
        };
        assert_eq!(text.trim(), "[]", "ledger must be empty after undo: {text}");
        assert!(!uteke.get_by_id(&old).unwrap().unwrap().deprecated);

        // Undo again → loud error (no supersession left).
        let err = exec_contradictions_undo(&uteke, &serde_json::json!({"id": &old}));
        assert!(err.is_err(), "second undo must error loudly");

        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod room_recall_tests {
    //! #1335 — `uteke_room_recall` parity with `uteke_recall`.
    use super::*;
    use uteke_core::SearchResult;
    use uteke_core::memory::types::Memory;

    fn sr(id: &str, content: &str, score: f32) -> SearchResult {
        SearchResult {
            memory: Memory {
                id: id.to_string(),
                content: content.to_string(),
                embedding: vec![],
                tags: vec![],
                metadata: serde_json::json!({}),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                namespace: "ns".to_string(),
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
                author: None,
            },
            score,
        }
    }

    #[test]
    fn defaults_are_unchanged() {
        let a = parse_room_recall_args(&serde_json::json!({"room_id": "r", "query": "q"}));
        assert_eq!(
            a,
            RoomRecallArgs {
                limit: 5,
                min_score: 0.0,
                tags: vec![],
                pack: false,
                budget_chars: 4000,
                exclude_ids: vec![],
                full_ids: false,
            }
        );
    }

    #[test]
    fn all_new_parameters_are_parsed() {
        let a = parse_room_recall_args(&serde_json::json!({
            "room_id": "r", "query": "q", "limit": 3, "min_score": 0.25,
            "tags": ["a", "b"], "pack": true, "budget_chars": 900,
            "exclude_ids": ["x", "y"], "full_ids": true
        }));
        assert_eq!(a.limit, 3);
        assert!((a.min_score - 0.25).abs() < 1e-6);
        assert_eq!(a.tags, ["a", "b"]);
        assert!(a.pack && a.full_ids);
        assert_eq!(a.budget_chars, 900);
        assert_eq!(a.exclude_ids, ["x", "y"]);
        // negative min_score is clamped, non-string array items ignored
        let a = parse_room_recall_args(&serde_json::json!({"min_score": -1.0, "tags": ["a", 7]}));
        assert_eq!(a.min_score, 0.0);
        assert_eq!(a.tags, ["a"]);
    }

    #[test]
    fn schema_advertises_every_new_parameter() {
        let tool = tool_room_recall();
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        for k in [
            "room_id",
            "query",
            "limit",
            "min_score",
            "tags",
            "pack",
            "budget_chars",
            "exclude_ids",
            "full_ids",
        ] {
            assert!(props.contains_key(k), "missing {k}");
        }
        // required fields did not change
        assert_eq!(
            tool["inputSchema"]["required"],
            serde_json::json!(["room_id", "query"])
        );
    }

    #[test]
    fn every_result_carries_its_memory_id_and_the_score_line_is_unchanged() {
        let id = "01a11a97-1814-747a-bff1-98db63fb137f";
        let lines = format_room_recall_lines(&[sr(id, "hello world", 0.8123)], false);
        assert_eq!(lines[0], "[0.81] hello world", "historical first line");
        assert_eq!(lines[1], "       (id: 01a11a97)");
        let full = format_room_recall_lines(&[sr(id, "hello world", 0.5)], true);
        assert_eq!(full[1], format!("       (id: {id})"));
        assert!(format_room_recall_lines(&[], false).is_empty());
    }
}

#[cfg(test)]
mod room_list_filter_tests {
    //! #1336 — name filter for `uteke_room_list`.
    use super::*;

    fn room(id: &str, title: Option<&str>) -> uteke_core::Room {
        uteke_core::Room {
            id: id.to_string(),
            title: title.map(str::to_string),
            namespace: "ns".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            description: None,
        }
    }

    fn ids(rooms: Vec<uteke_core::Room>) -> Vec<String> {
        rooms.into_iter().map(|r| r.id).collect()
    }

    fn sample() -> Vec<uteke_core::Room> {
        vec![
            room("gadio", None),
            room("project-gadio", None),
            room("project:gadio", Some("Gadio dev log")),
            room("uteke", Some("Memory engine")),
        ]
    }

    #[test]
    fn matches_id_and_title_case_insensitively() {
        assert_eq!(
            ids(filter_rooms_by_name(sample(), Some("GADIO"))),
            ["gadio", "project-gadio", "project:gadio"]
        );
        // title-only match
        assert_eq!(
            ids(filter_rooms_by_name(sample(), Some("engine"))),
            ["uteke"]
        );
    }

    #[test]
    fn blank_or_missing_name_keeps_everything() {
        assert_eq!(filter_rooms_by_name(sample(), None).len(), 4);
        assert_eq!(filter_rooms_by_name(sample(), Some("  ")).len(), 4);
    }

    #[test]
    fn no_match_returns_empty_and_schema_advertises_name() {
        assert!(filter_rooms_by_name(sample(), Some("zzz")).is_empty());
        let tool = tool_room_list();
        assert!(tool["inputSchema"]["properties"]["name"].is_object());
        assert!(tool["inputSchema"]["properties"]["namespace"].is_object());
    }
}

#[cfg(test)]
mod initialize_instructions_tests {
    //! #1395 — `initialize` carries the conventions-first rule.
    use super::*;

    #[test]
    fn instructions_carry_the_rule_and_point_to_the_full_guide() {
        let text = initialize_instructions();
        assert!(text.starts_with("Conventions first."));
        assert!(text.contains("no secrets"));
        assert!(text.contains("uteke guide"));
        // short on purpose: the whole guide is not injected into every client
        assert!(text.len() < 800, "{} chars", text.len());
    }
}

#[cfg(test)]
mod remember_decode_tests {
    use super::decode_remember_args;
    use serde_json::json;

    #[test]
    fn an_omitted_type_is_left_to_inference_and_author_stays_raw() {
        let r = decode_remember_args(&json!({"content": "x"})).unwrap();
        assert_eq!(r.memory_type, None);
        assert_eq!(r.metadata, None);
        assert_eq!(r.author, None);
        assert_eq!(r.room, None);
    }

    #[test]
    fn blank_content_a_non_string_type_and_odd_tags_are_errors() {
        assert_eq!(
            decode_remember_args(&json!({"content": "  "})).unwrap_err(),
            "Validation error: Content must not be empty"
        );
        assert_eq!(
            decode_remember_args(&json!({"content": "x", "type": 5})).unwrap_err(),
            "Invalid 'type': expected a string"
        );
        assert_eq!(
            decode_remember_args(&json!({"content": "x", "tags": ["a", 1]})).unwrap_err(),
            "Invalid 'tags': expected an array of strings"
        );
        assert!(
            decode_remember_args(&json!({"content": "x", "bogus": true}))
                .unwrap_err()
                .starts_with("Unknown argument 'bogus'. Supported: author, content,")
        );
    }

    #[test]
    fn an_unknown_type_is_refused_before_any_write() {
        let e = decode_remember_args(&json!({"content": "x", "type": "bogus"})).unwrap_err();
        assert!(e.starts_with("Unknown memory type 'bogus'"), "{e}");
    }

    #[test]
    fn missing_content_message_is_unchanged() {
        assert_eq!(
            decode_remember_args(&json!({"tags": ["a"]})).unwrap_err(),
            "Missing 'content'"
        );
    }
}

#[cfg(test)]
mod namespace_deprecation_tests {
    use super::*;
    use uteke_core::namespace_deprecation::NAMESPACE_DEPRECATION_WARNING;

    fn text(r: ToolResult) -> String {
        match &r.content[0] {
            McpContent::Text { text, .. } => text.clone(),
        }
    }

    fn store() -> Uteke {
        Uteke::open_with_backend(":memory:", None).expect("open without embedder")
    }

    #[test]
    fn remember_non_default_namespace_appends_warning() {
        let u = store();
        let t = text(
            exec_remember(
                &u,
                &serde_json::json!({"content": "x", "namespace": "ns-mcp-a"}),
            )
            .unwrap(),
        );
        assert!(t.starts_with("✓ Stored memory with ID: "), "{t}");
        assert!(
            t.ends_with(&format!("\n⚠ {NAMESPACE_DEPRECATION_WARNING}")),
            "{t}"
        );
    }

    #[test]
    fn remember_default_or_omitted_is_unchanged() {
        let u = store();
        for args in [
            serde_json::json!({"content": "a"}),
            serde_json::json!({"content": "b", "namespace": "default"}),
        ] {
            let t = text(exec_remember(&u, &args).unwrap());
            assert!(!t.contains("deprecated"), "{t}");
            assert!(!t.contains('\n'), "{t}");
        }
    }

    #[test]
    fn recall_has_the_same_rules_as_http_for_limit_blank_query_and_empty_tags() {
        // Blank query: rejected (#1439).
        for q in ["", "   \t"] {
            let e = decode_recall_args(&serde_json::json!({"query": q})).unwrap_err();
            assert_eq!(e, "Query must not be empty or whitespace-only");
        }
        // Limit: capped at 100, an invalid value is an error, not a default.
        let r = decode_recall_args(&serde_json::json!({"query": "q", "limit": 5000})).unwrap();
        assert_eq!(r.limit, 100);
        for bad in [
            serde_json::json!("ten"),
            serde_json::json!(-1),
            serde_json::json!(2.5),
        ] {
            let e =
                decode_recall_args(&serde_json::json!({"query": "q", "limit": bad})).unwrap_err();
            assert!(e.starts_with("Invalid 'limit'"), "{e}");
        }
        assert_eq!(
            decode_recall_args(&serde_json::json!({"query": "q"}))
                .unwrap()
                .limit,
            10
        );
        // `tags: []` is no filter.
        let r = decode_recall_args(&serde_json::json!({"query": "q", "tags": []})).unwrap();
        assert_eq!(r.tag_filter(), None);
    }

    #[test]
    fn recall_refuses_an_argument_the_tool_does_not_declare() {
        let u = store();
        let e = exec_recall(&u, &serde_json::json!({"query": "q", "limt": 5}))
            .err()
            .expect("must fail");
        assert!(e.starts_with("Unknown argument 'limt'. Supported: "), "{e}");
        for name in [
            "limit", "entity", "category", "at", "after", "before", "strict", "enrich", "full_ids",
        ] {
            assert!(e.contains(name), "{name} missing from {e}");
        }
        // The declared names, including the new ones, are accepted.
        let ok = decode_recall_args(&serde_json::json!({
            "query": "q", "entity": "e", "category": "c", "strict": true, "enrich": true,
            "at": "2026-01-01T00:00:00Z", "after": "2025-01-01T00:00:00Z", "before": "2027-01-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(ok.entity.as_deref(), Some("e"));
        assert!(
            ok.strict && ok.enrich && ok.at.is_some() && ok.after.is_some() && ok.before.is_some()
        );
    }

    #[test]
    fn recall_honours_entity_and_time_filters_over_mcp() {
        let u = store();
        u.remember(
            "rust gives memory safety",
            &[],
            Some(serde_json::json!({"entity": "rust"})),
            None,
        )
        .unwrap();
        let find = |extra: serde_json::Value| {
            let mut a =
                serde_json::json!({"query": "memory safety", "type": "memory", "strategy": "fts5"});
            for (k, v) in extra.as_object().unwrap() {
                a[k] = v.clone();
            }
            text(exec_recall(&u, &a).unwrap())
        };
        assert!(find(serde_json::json!({})).contains("rust gives memory safety"));
        assert!(find(serde_json::json!({"entity": "rust"})).contains("rust gives memory safety"));
        assert_eq!(
            find(serde_json::json!({"entity": "go"})),
            "No results found."
        );
        assert_eq!(
            find(serde_json::json!({"after": "2999-01-01T00:00:00Z"})),
            "No results found."
        );
        // Combinations the HTTP rules refuse.
        let e = exec_recall(
            &u,
            &serde_json::json!({"query": "q", "pack": true, "at": "2026-01-01T00:00:00Z"}),
        )
        .err()
        .unwrap();
        assert!(
            e.starts_with("pack is not supported with at/after/before"),
            "{e}"
        );
        let e = exec_recall(
            &u,
            &serde_json::json!({"query": "q", "explain": true, "entity": "x"}),
        )
        .err()
        .unwrap();
        assert_eq!(e, "explain cannot be combined with entity/category/enrich");
    }

    #[test]
    fn list_over_mcp_has_the_http_options_and_refuses_unknown_arguments() {
        let u = store();
        for i in 0..5 {
            u.remember(&format!("note number {i}"), &["t"], None, None)
                .unwrap();
        }
        // include_meta appends the pagination footer.
        let t =
            text(exec_list(&u, &serde_json::json!({"limit": 2, "include_meta": true})).unwrap());
        assert!(
            t.ends_with("-- total 5, has_more true, next_offset 2"),
            "{t}"
        );
        let t = text(
            exec_list(
                &u,
                &serde_json::json!({"limit": 2, "offset": 4, "include_meta": true}),
            )
            .unwrap(),
        );
        assert!(
            t.ends_with("-- total 5, has_more false, next_offset none"),
            "{t}"
        );
        // A point in time before everything lists nothing.
        let t = text(exec_list(&u, &serde_json::json!({"at": "2000-01-01T00:00:00Z"})).unwrap());
        assert_eq!(t, "No memories found.");
        let e = exec_list(&u, &serde_json::json!({"at": "yesterday"}))
            .err()
            .unwrap();
        assert!(e.starts_with("Invalid 'at' timestamp: yesterday"), "{e}");
        // Wrong types and unknown arguments are errors.
        assert!(exec_list(&u, &serde_json::json!({"limit": "x"})).is_err());
        let e = exec_list(&u, &serde_json::json!({"limt": 3}))
            .err()
            .unwrap();
        assert!(e.starts_with("Unknown argument 'limt'. Supported: "), "{e}");
    }

    #[test]
    fn explain_with_any_type_is_refused_like_http() {
        let u = store();
        let err = exec_recall(
            &u,
            &serde_json::json!({"query": "q", "explain": true, "type": "memory"}),
        )
        .err()
        .expect("must fail");
        assert_eq!(err, "explain is memory-only: omit type");
        // Without `type` the explain guard does not fire (the store here has no
        // embedder, so the call may still fail later, but not on that rule).
        if let Err(e) = exec_recall(&u, &serde_json::json!({"query": "q", "explain": true})) {
            assert!(!e.contains("memory-only"), "{e}");
        }
    }

    #[test]
    fn room_delete_says_the_memories_are_retained_and_how_to_delete_them() {
        let u = store();
        u.create_room("gone", None, "default").unwrap();
        let t = text(exec_room_delete(&u, &serde_json::json!({"room_id": "gone"})).unwrap());
        assert!(t.contains("unregistered"), "{t}");
        assert!(t.contains("retained"), "{t}");
        assert!(t.contains("uteke_forget"), "{t}");
        assert!(!t.contains("deleted"), "{t}");
    }

    #[test]
    fn room_create_warns_only_for_non_default() {
        let u = store();
        let t = text(exec_room_create(&u, &serde_json::json!({"room_id": "r1"})).unwrap());
        assert_eq!(t, "Room created: r1 (namespace: default)");
        let t = text(
            exec_room_create(
                &u,
                &serde_json::json!({"room_id": "r2", "namespace": "ns-mcp-r"}),
            )
            .unwrap(),
        );
        assert!(t.contains(NAMESPACE_DEPRECATION_WARNING), "{t}");
    }

    #[test]
    fn read_only_tools_list_is_exactly_the_read_tools() {
        let u = store();
        let names = |access| -> std::collections::BTreeSet<String> {
            handle_request(&u, "tools/list", None, access).unwrap()["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        let read_only = names(Access::ReadOnly);
        // `uteke_room_document` is a dispatch alias that tools/list never advertises.
        let expected: std::collections::BTreeSet<String> = READ_ONLY_TOOLS
            .iter()
            .filter(|n| **n != "uteke_room_document")
            .map(|n| n.to_string())
            .collect();
        assert_eq!(read_only, expected);
        let full = names(Access::Full);
        assert!(read_only.is_subset(&full));
        assert_eq!(full.len(), 49, "full catalog must not change");
        for write in ["uteke_remember", "uteke_forget", "uteke_doc_create"] {
            assert!(
                full.contains(write) && !read_only.contains(write),
                "{write}"
            );
        }
    }

    #[test]
    fn every_tool_with_a_namespace_argument_says_deprecated() {
        let mut checked = 0;
        let list = handle_request(&store(), "tools/list", None, Access::Full).unwrap();
        for t in list["tools"].as_array().unwrap() {
            let name = t["name"].as_str().unwrap().to_string();
            if let Some(arg) = t["inputSchema"]["properties"]["namespace"]["description"].as_str() {
                assert!(arg.contains("DEPRECATED"), "{name}: {arg}");
                checked += 1;
            }
        }
        assert!(checked >= 10, "only {checked} namespace args found");
    }
}

#[cfg(test)]
mod read_only_gate_tests {
    use super::*;

    fn store() -> Uteke {
        Uteke::open_with_backend(":memory:", None).expect("open without embedder")
    }

    fn call(u: &Uteke, access: Access, tool: &str, args: serde_json::Value) -> serde_json::Value {
        let raw = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": args}
        })
        .to_string();
        let out = handle_jsonrpc_as(u, &raw, access).expect("request, not notification");
        serde_json::from_str(&out).unwrap()
    }

    fn listed_tools(u: &Uteke) -> Vec<String> {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let out: serde_json::Value =
            serde_json::from_str(&handle_jsonrpc(u, raw).unwrap()).unwrap();
        out["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn every_read_only_tool_exists() {
        let u = store();
        let listed = listed_tools(&u);
        for t in READ_ONLY_TOOLS {
            // uteke_room_document is a dispatch alias of uteke_room_summary_document.
            if *t == "uteke_room_document" {
                continue;
            }
            assert!(listed.iter().any(|l| l == t), "{t} is not a listed tool");
        }
    }

    #[test]
    fn read_only_refuses_a_write_tool_and_writes_nothing() {
        let u = store();
        let resp = call(
            &u,
            Access::ReadOnly,
            "uteke_remember",
            serde_json::json!({"content": "x"}),
        );
        assert!(resp.get("error").is_none(), "{resp}");
        assert_eq!(resp["result"]["isError"], true);
        assert_eq!(
            resp["result"]["content"][0]["text"],
            "Read-only token cannot use uteke_remember"
        );
        assert_eq!(u.count(None).unwrap(), 0);
    }

    #[test]
    fn read_only_tool_set_is_the_reviewed_one() {
        // Adding a name to READ_ONLY_TOOLS opens a tool to read-only tokens:
        // it must be a deliberate edit of this test too.
        let expected: std::collections::BTreeSet<&str> = [
            "uteke_recall",
            "uteke_search",
            "uteke_list",
            "uteke_get",
            "uteke_provenance",
            "uteke_timeline",
            "uteke_contradictions",
            "uteke_stats",
            "uteke_doc_get",
            "uteke_doc_list",
            "uteke_doc_search",
            "uteke_doc_list_rooms",
            "uteke_graph",
            "uteke_room_list",
            "uteke_room_recall",
            "uteke_room_memories",
            "uteke_room_stats",
            "uteke_room_summary",
            "uteke_room_summary_document",
            "uteke_room_document",
            "uteke_room_list_documents",
            "uteke_tags_list",
        ]
        .into_iter()
        .collect();
        let actual: std::collections::BTreeSet<&str> = READ_ONLY_TOOLS.iter().copied().collect();
        assert_eq!(actual, expected);
        assert_eq!(READ_ONLY_TOOLS.len(), expected.len(), "duplicate entry");
    }

    #[test]
    fn read_only_room_document_alias_does_not_write() {
        let u = store();
        let resp = call(
            &u,
            Access::ReadOnly,
            "uteke_room_document",
            serde_json::json!({"room_id": "nope"}),
        );
        assert!(
            !resp.to_string().contains("Read-only token cannot use"),
            "{resp}"
        );
        assert_eq!(u.count(None).unwrap(), 0);
    }

    #[test]
    fn read_only_notification_on_write_tool_runs_nothing() {
        let u = store();
        let raw = serde_json::json!({
            "jsonrpc": "2.0", "method": "tools/call",
            "params": {"name": "uteke_remember", "arguments": {"content": "x"}}
        })
        .to_string();
        assert!(handle_jsonrpc_as(&u, &raw, Access::ReadOnly).is_none());
        assert_eq!(u.count(None).unwrap(), 0);
    }

    #[test]
    fn read_only_allows_a_read_tool() {
        let u = store();
        let resp = call(&u, Access::ReadOnly, "uteke_stats", serde_json::json!({}));
        assert!(resp.get("error").is_none(), "{resp}");
    }

    #[test]
    fn full_access_is_unchanged() {
        let u = store();
        let resp = call(
            &u,
            Access::Full,
            "uteke_remember",
            serde_json::json!({"content": "x"}),
        );
        assert!(resp.get("error").is_none(), "{resp}");
        assert_eq!(u.count(None).unwrap(), 1);
    }
}

/// Scope-isolation regression gate over MCP (#1467, part 3).
///
/// The two-namespace store of `uteke_core::test_support::seed_two_scopes`
/// (records of `alpha` and `beta` share one vocabulary, each ends in
/// `of <namespace>`) is read through every read-only tool that takes a scope
/// and must never return the other namespace's records or project tag.
/// `every_read_tool_is_covered_or_exempt` fails when a read-only tool is added
/// without an isolation check or a documented exemption.
#[cfg(test)]
mod isolation_tests {
    use super::*;
    use serde_json::{Value, json};
    use uteke_core::test_support::{ISOLATION_QUERY as Q, ISOLATION_SCOPES, seed_two_scopes};

    fn call(u: &Uteke, tool: &str, args: Value) -> String {
        let raw = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": args}
        })
        .to_string();
        handle_jsonrpc_as(u, &raw, Access::ReadOnly).expect("a response")
    }

    /// (tool, arguments, the response must hold an `of <scope>` marker).
    fn surfaces(scope: &str) -> Vec<(&'static str, Value, bool)> {
        let mut v = vec![
            (
                "uteke_search",
                json!({"query": Q, "namespace": scope, "limit": 20}),
                true,
            ),
            ("uteke_list", json!({"namespace": scope, "limit": 50}), true),
            (
                "uteke_list",
                json!({"namespace": scope, "tag": "shared", "limit": 50}),
                true,
            ),
            ("uteke_graph", json!({"namespace": scope}), true),
            (
                "uteke_room_recall",
                json!({"room_id": format!("room-{scope}"), "query": Q}),
                true,
            ),
            (
                "uteke_room_memories",
                json!({"room_id": format!("room-{scope}")}),
                true,
            ),
            ("uteke_tags_list", json!({"namespace": scope}), false),
            ("uteke_room_list", json!({"namespace": scope}), false),
            ("uteke_stats", json!({"namespace": scope}), false),
            ("uteke_contradictions", json!({"namespace": scope}), false),
        ];
        for strategy in ["hybrid", "vector", "fts5", "graph", "fusion"] {
            for kind in ["memory", "all"] {
                v.push((
                    "uteke_recall",
                    json!({"query": Q, "namespace": scope, "limit": 20, "strategy": strategy, "type": kind}),
                    true,
                ));
            }
        }
        v.push((
            "uteke_recall",
            json!({"query": Q, "namespace": scope, "pack": true, "budget_chars": 4000}),
            true,
        ));
        v.push((
            "uteke_recall",
            json!({"query": Q, "namespace": scope, "explain": true}),
            true,
        ));
        v.push((
            "uteke_recall",
            json!({"query": Q, "namespace": scope, "tags": ["shared"]}),
            true,
        ));
        v
    }

    #[test]
    fn every_scoped_read_tool_returns_only_its_own_namespace() {
        let u = seed_two_scopes();
        for scope in ISOLATION_SCOPES {
            let foreign = if scope == "alpha" { "beta" } else { "alpha" };
            for (tool, args, expects_hits) in surfaces(scope) {
                let text = call(&u, tool, args.clone());
                let what = format!("{tool} {args}");
                let shown: String = text.chars().take(300).collect();
                assert!(!text.contains("\"isError\":true"), "{what}: {shown}");
                assert!(
                    !text.contains(&format!("of {foreign}")),
                    "{what} scoped to {scope} returned {foreign} records: {shown}"
                );
                assert!(
                    !text.contains(&format!("project:{foreign}")),
                    "{what} scoped to {scope} exposed the {foreign} project tag: {shown}"
                );
                if expects_hits {
                    assert!(
                        text.contains(&format!("of {scope}")),
                        "{what} found no {scope} record, the check would be vacuous: {shown}"
                    );
                }
            }
        }
    }

    /// Read-only tools the suite does not scope-check, each with the reason.
    const EXEMPT: &[(&str, &str)] = &[
        ("uteke_get", "fetch by ID: IDs are global handles"),
        ("uteke_provenance", "by memory ID"),
        ("uteke_timeline", "by memory ID"),
        (
            "uteke_doc_get",
            "by document slug; documents are global (#1271)",
        ),
        (
            "uteke_doc_list",
            "documents are global (#1271), no memory content",
        ),
        (
            "uteke_doc_search",
            "documents are global (#1271), no memory content",
        ),
        ("uteke_doc_list_rooms", "by document slug"),
        ("uteke_room_stats", "by room ID; the room is the scope"),
        ("uteke_room_summary", "by room ID; the room is the scope"),
        (
            "uteke_room_summary_document",
            "by room ID; the room is the scope",
        ),
        (
            "uteke_room_document",
            "dispatch alias of uteke_room_summary_document, never advertised",
        ),
        (
            "uteke_room_list_documents",
            "by room ID; documents are global",
        ),
    ];

    /// Every read-only tool must be covered by `surfaces` or exempt with a
    /// reason, and a tool that advertises a `namespace` argument can never be
    /// exempt: a new retrieval tool cannot skip the isolation suite unnoticed.
    #[test]
    fn every_read_tool_is_covered_or_exempt() {
        let covered: std::collections::BTreeSet<&str> =
            surfaces("alpha").iter().map(|(t, _, _)| *t).collect();
        let exempt: std::collections::BTreeSet<&str> = EXEMPT.iter().map(|(t, _)| *t).collect();
        for tool in READ_ONLY_TOOLS {
            assert!(
                covered.contains(tool) || exempt.contains(tool),
                "{tool} is a read-only tool with no isolation check: add it to `surfaces` or to `EXEMPT` with a reason (#1467)"
            );
        }
        for tool in covered.iter().chain(exempt.iter()) {
            assert!(
                READ_ONLY_TOOLS.contains(tool),
                "{tool} is not a read-only tool"
            );
        }
        assert!(
            covered.is_disjoint(&exempt),
            "a tool is both covered and exempt"
        );
        let u = seed_two_scopes();
        let list = handle_request(&u, "tools/list", None, Access::ReadOnly).unwrap();
        for t in list["tools"].as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            if t["inputSchema"]["properties"]["namespace"].is_object() {
                assert!(
                    covered.contains(name),
                    "{name} takes a namespace but is not covered by the isolation suite"
                );
            }
        }
    }
}

/// `author` over MCP (#1427 phase 2): `uteke_remember` records it on the memory
/// with or without a room, and rejects an invalid one before writing.
#[cfg(test)]
mod author_tests {
    use super::*;
    use serde_json::{Value, json};

    fn remember(u: &Uteke, args: Value) -> (bool, String) {
        let raw = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "uteke_remember", "arguments": args}
        })
        .to_string();
        let resp: Value = serde_json::from_str(&handle_jsonrpc(u, &raw).unwrap()).unwrap();
        let result = &resp["result"];
        let is_error = result["isError"].as_bool().unwrap_or(false) || resp.get("error").is_some();
        // A tool error comes back either as an error result or as a JSON-RPC error.
        let text = result["content"][0]["text"]
            .as_str()
            .or_else(|| resp["error"]["message"].as_str())
            .unwrap_or_default();
        (is_error, text.to_string())
    }

    fn stored_id(text: &str) -> String {
        text.split("ID: ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn remember_records_the_author_with_or_without_a_room() {
        let u = Uteke::open_with_backend(":memory:", None).unwrap();
        let author_of = |text: &str| u.get_by_id(&stored_id(text)).unwrap().unwrap().author;

        let (err, text) = remember(&u, json!({"content": "no room", "author": " hermes "}));
        assert!(!err, "{text}");
        assert_eq!(author_of(&text).as_deref(), Some("hermes"));

        let (err, text) = remember(
            &u,
            json!({"content": "in a room", "room": "r1", "author": "alice"}),
        );
        assert!(!err, "{text}");
        assert_eq!(author_of(&text).as_deref(), Some("alice"));

        // A room write that names no author leaves the memory unattributed.
        let (err, text) = remember(&u, json!({"content": "unnamed", "room": "r1"}));
        assert!(!err, "{text}");
        assert_eq!(author_of(&text), None);
    }

    #[test]
    fn the_stored_type_is_inferred_explicit_or_fact_on_every_write_path() {
        let u = Uteke::open_with_backend(":memory:", None).unwrap();
        let type_of = |text: &str| {
            u.get_by_id(&stored_id(text))
                .unwrap()
                .unwrap()
                .memory_type
                .as_str()
                .to_string()
        };
        let cases = [
            (json!({"content": "We decided to use SQLite"}), "decision"),
            (json!({"content": "plain words"}), "fact"),
            (
                json!({"content": "We decided to use SQLite", "type": "note"}),
                "note",
            ),
            (
                json!({"content": "We decided to use SQLite", "room": "r"}),
                "decision",
            ),
            (json!({"content": "plain words", "room": "r"}), "fact"),
        ];
        for (args, want) in cases {
            let (err, text) = remember(&u, args.clone());
            assert!(!err, "{args}: {text}");
            assert_eq!(type_of(&text), want, "{args}");
        }
    }

    #[test]
    fn an_invalid_author_is_refused_before_anything_is_stored() {
        let u = Uteke::open_with_backend(":memory:", None).unwrap();
        for bad in ["", "  ", "line\nbreak", &"y".repeat(129)] {
            let (err, text) = remember(&u, json!({"content": "must not be stored", "author": bad}));
            assert!(err, "{bad:?} was accepted: {text}");
            assert!(text.contains("author"), "{text}");
        }
        assert_eq!(u.count(None).unwrap(), 0, "nothing is stored");
    }
}

/// The privacy filter over MCP (#1464): masked writes say so, strict mode is a
/// tool error that starts with `secret_detected` and names kinds, not values.
#[cfg(test)]
mod privacy_mcp_tests {
    use super::*;
    use uteke_core::Uteke;
    use uteke_core::privacy::{PrivacyConfig, PrivacyMode};

    fn store(mode: PrivacyMode) -> Uteke {
        let mut u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        u.set_privacy_config(PrivacyConfig {
            mode,
            allowlist: Vec::new(),
        });
        u
    }

    fn token() -> String {
        format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5")
    }

    fn text(r: &ToolResult) -> String {
        match &r.content[0] {
            McpContent::Text { text, .. } => text.clone(),
        }
    }

    #[test]
    fn remember_says_when_a_secret_was_masked() {
        let u = store(PrivacyMode::Redact);
        let r = exec_remember(
            &u,
            &serde_json::json!({"content": format!("key {}", token())}),
        )
        .unwrap();
        let t = text(&r);
        assert!(
            t.contains("Secrets masked") && t.contains("github-token x1"),
            "{t}"
        );
        assert!(!t.contains("aB3dE5gH7jK9"), "{t}");

        let clean = exec_remember(&u, &serde_json::json!({"content": "nothing secret"})).unwrap();
        assert!(!text(&clean).contains("Secrets masked"));
    }

    #[test]
    fn update_says_when_a_secret_was_masked() {
        let u = store(PrivacyMode::Redact);
        let ok = exec_remember(&u, &serde_json::json!({"content": "plain"})).unwrap();
        let id = text(&ok).rsplit(' ').next().unwrap().trim().to_string();
        let r = exec_update(&u, &serde_json::json!({"id": id, "tags": [token()]})).unwrap();
        let t = text(&r);
        assert!(
            t.contains("Secrets masked") && t.contains("github-token x1"),
            "{t}"
        );
        let r = exec_update(&u, &serde_json::json!({"id": id, "importance": 0.9})).unwrap();
        assert!(!text(&r).contains("Secrets masked"));
    }

    #[test]
    fn strict_mode_is_a_secret_detected_tool_error_for_remember_and_update() {
        let u = store(PrivacyMode::Strict);
        let err = exec_remember(
            &u,
            &serde_json::json!({"content": format!("key {}", token())}),
        )
        .err()
        .expect("strict rejects");
        assert!(err.starts_with("secret_detected:"), "{err}");
        assert!(
            err.contains("github-token") && !err.contains("aB3dE5gH7jK9"),
            "{err}"
        );

        let ok = exec_remember(&u, &serde_json::json!({"content": "plain"})).unwrap();
        let id = text(&ok).rsplit(' ').next().unwrap().trim().to_string();
        let err = exec_update(
            &u,
            &serde_json::json!({"id": id, "content": format!("now {}", token())}),
        )
        .err()
        .expect("strict rejects the update");
        assert!(err.starts_with("secret_detected:"), "{err}");
    }
}

/// `uteke_recall` falls back to the `[recall]` settings of `uteke.toml` (#1515):
/// the similarity floor and the strategy, as `POST /recall` does.
#[cfg(test)]
mod recall_settings_mcp_tests {
    use super::*;
    use uteke_core::test_support::open_keyword;
    use uteke_core::{EngineSettings, Uteke};

    fn store(configure: impl FnOnce(&mut EngineSettings)) -> Uteke {
        let mut u = open_keyword();
        for text in ["rust ownership and borrowing", "python garbage collector"] {
            u.remember_detailed(text, &[], None, None).unwrap();
        }
        let mut e = EngineSettings::default();
        configure(&mut e);
        u.apply_settings(&e);
        u
    }

    fn recall(u: &Uteke, args: Value) -> String {
        match &exec_recall(u, &args).unwrap().content[0] {
            McpContent::Text { text, .. } => text.clone(),
        }
    }

    #[test]
    fn without_a_recall_section_nothing_changes() {
        let u = store(|_| {});
        let t = recall(&u, serde_json::json!({"query": "rust borrowing"}));
        assert!(t.contains("rust ownership"), "{t}");
    }

    #[test]
    fn the_configured_floor_filters_and_a_call_floor_wins_over_it() {
        let u = store(|e| e.recall.min_score = 0.99);
        let t = recall(&u, serde_json::json!({"query": "rust borrowing"}));
        assert!(!t.contains("rust ownership"), "the floor is 0.99: {t}");
        let t = recall(
            &u,
            serde_json::json!({"query": "rust borrowing", "min_score": 0.0}),
        );
        assert!(
            t.contains("rust ownership"),
            "the call's own floor wins: {t}"
        );
    }

    #[test]
    fn the_configured_default_strategy_is_used_and_a_call_strategy_wins() {
        let u = store(|e| e.recall.default_strategy = "vector".into());
        // Both strategies answer; what matters is that neither is rejected and
        // an explicit strategy still overrides the configured one.
        assert!(recall(&u, serde_json::json!({"query": "rust"})).contains("rust ownership"));
        assert!(
            recall(&u, serde_json::json!({"query": "rust", "strategy": "fts5"}))
                .contains("rust ownership")
        );
        // An unknown configured strategy was sanitized to hybrid at apply time.
        let u = store(|e| e.recall.default_strategy = "nonsense".into());
        assert!(recall(&u, serde_json::json!({"query": "rust"})).contains("rust ownership"));
    }
}
