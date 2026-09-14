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

/// Handle a single MCP JSON-RPC request (#381).
///
/// This is the shared handler used by both the stdio binary and the
/// HTTP endpoint. Returns `Some(JsonRpcResponse)` for regular requests
/// and `None` for notifications (which must not receive a response
/// per JSON-RPC 2.0 §4.1).
pub fn handle_jsonrpc(uteke: &Uteke, raw: &str) -> Option<String> {
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

    match handle_request(uteke, &req.method, req.params) {
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

fn handle_request(uteke: &Uteke, method: &str, params: Option<Value>) -> Result<Value, String> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "uteke",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),

        "notifications/initialized" => Ok(Value::Null),

        "tools/list" => Ok(serde_json::json!({
            "tools": [
                tool_remember(),
                tool_recall(),
                tool_search(),
                tool_list(),
                tool_get(),
                tool_provenance(),
                tool_contradictions(),
                tool_contradictions_undo(),
                tool_supersede(),
                tool_update(),
                tool_forget(),
                tool_stats(),
                tool_context(),
                tool_dream(),
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
            ]
        })),

        "tools/call" => {
            let params = params.ok_or("Missing params for tools/call")?;
            let tool_name = params["name"].as_str().ok_or("Missing tool name")?;
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));

            let result = match tool_name {
                "uteke_remember" => exec_remember(uteke, &arguments)?,
                "uteke_recall" => exec_recall(uteke, &arguments)?,
                "uteke_search" => exec_search(uteke, &arguments)?,
                "uteke_list" => exec_list(uteke, &arguments)?,
                "uteke_get" => exec_get(uteke, &arguments)?,
                "uteke_provenance" => exec_provenance(uteke, &arguments)?,
                "uteke_contradictions" => exec_contradictions(uteke, &arguments)?,
                "uteke_contradictions_undo" => exec_contradictions_undo(uteke, &arguments)?,
                "uteke_supersede" => exec_supersede(uteke, &arguments)?,
                "uteke_update" => exec_update(uteke, &arguments)?,
                "uteke_forget" => exec_forget(uteke, &arguments)?,
                "uteke_stats" => exec_stats(uteke, &arguments)?,
                "uteke_context" => exec_context(uteke, &arguments)?,
                "uteke_dream" => exec_dream(uteke, &arguments)?,
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
                "namespace": { "type": "string", "description": "Namespace for isolation (default: 'default')" },
                "type": { "type": "string", "description": "Memory type: fact, procedure, preference, decision, context, note, insight, reference, event (default: fact)" },
                "room": { "type": "string", "description": "Room ID for collaborative memory (optional)" },
                "author": { "type": "string", "description": "Author attribution when storing in a room (default: anonymous)" }
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
                "limit": { "type": "integer", "description": "Max results (default 5)", "default": 5 },
                "namespace": { "type": "string", "description": "Namespace to search (default: 'default')" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Filter by tags (optional)" },
                "min_score": { "type": "number", "description": "Minimum similarity score 0..1 (default: 0.0)" },
                "type": { "type": "string", "enum": ["all", "memory", "doc"], "description": "Search type: 'all' (default, unified), 'memory', or 'doc'" },
                "strategy": { "type": "string", "enum": ["fusion", "hybrid", "vector", "fts5", "graph"], "description": "Recall strategy: 'fusion' (default since 0.16.0, weighted RRF of vector×1.7 + hybrid×1, #1123), 'hybrid' (vector+FTS5 via RRF), 'vector' (similarity only), 'fts5' (keyword only), or 'graph' (hybrid + graph-signal reranking)", "default": "fusion" },
                "explain": { "type": "boolean", "description": "Return per-result ranking signals (#1160): vector similarity/rank, RRF contributions, jaccard/salience/recency/graph boosts. Memory-only — omitted type is treated as memory; explicit type=all/doc is rejected." }
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
                "limit": { "type": "integer", "description": "Max results (default 20)", "default": 20 },
                "offset": { "type": "integer", "description": "Pagination offset (default 0)", "default": 0 },
                "namespace": { "type": "string", "description": "Namespace (optional)" }
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

fn tool_contradictions() -> Value {
    serde_json::json!({
        "name": "uteke_contradictions",
        "description": "List the contradiction resolution ledger (#1172): memories that were superseded and soft-deprecated by conflict resolution, with winner, reason, and timestamp. Audit what the pipeline decided without digging through timeline events.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "Filter by namespace" },
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
                "namespace": { "type": "string", "description": "Move the memory to this namespace (#1181 — plain move, no re-embed)" }
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
                "namespace": { "type": "string", "description": "Namespace (optional)" }
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
                "namespace": { "type": "string", "description": "Namespace (optional)" },
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
        "description": "Get knowledge graph data (nodes + edges + stats) for visualization.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "Filter by namespace (optional)" }
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
                "namespace": { "type": "string", "description": "Namespace to summarize (default: 'default')" }
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
                "namespace": { "type": "string", "description": "Namespace to process. STRONGLY RECOMMENDED — omitting processes ALL namespaces" },
                "dry_run": { "type": "boolean", "description": "Preview changes without applying (default: TRUE — no mutations). Pass false explicitly to apply" },
                "phases": { "type": "array", "items": { "type": "string" }, "description": "Specific phases: lint, backlinks, dedup, orphans, compact, verify (default: all)" },
                "confirm_large": { "type": "boolean", "description": "Required when an APPLYING run projects more than 100 changes (default: false — run refuses instead)" }
            }
        }
    })
}

fn tool_room_recall() -> Value {
    serde_json::json!({
        "name": "uteke_room_recall",
        "description": "Semantic recall within a room context. Requires an EXISTING room_id (create via uteke_room_create first) — unknown room ids error at call time. Searches across all namespaces in the room using hybrid RRF ranking.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Room identifier" },
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default 5)", "default": 5 }
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
                "limit": { "type": "integer", "description": "Max results (default 100)", "default": 100 }
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
                "namespace": { "type": "string", "description": "Namespace for the room (default: 'default')" }
            },
            "required": ["room_id"]
        }
    })
}

fn tool_room_list() -> Value {
    serde_json::json!({
        "name": "uteke_room_list",
        "description": "List all rooms, optionally filtered by namespace.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "Filter by namespace (omit for all)" }
            }
        }
    })
}

fn tool_room_delete() -> Value {
    serde_json::json!({
        "name": "uteke_room_delete",
        "description": "Delete a room. Removes room links from memories but preserves the memories themselves.",
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
                "namespace": { "type": "string", "description": "Namespace to filter tags (default: all namespaces)" },
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
                "namespace": { "type": "string", "description": "Namespace scope (default: all namespaces)" }
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
                "namespace": { "type": "string", "description": "Namespace scope (default: all namespaces)" }
            },
            "required": ["tag"]
        }
    })
}

fn tool_namespace_rename() -> Value {
    serde_json::json!({
        "name": "uteke_namespace_rename",
        "description": "Rename a namespace, merging into the target when it already exists. Namespaces are a derived view — the old name vanishes when its last memory moved (#1181).",
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
        "description": "Delete a namespace with an explicit strategy for its memories: refuse (default — error while any memory uses the name), merge (move all memories to `target`), or deprecate (soft-delete — restorable via promote, never hard-deleted) (#1181).",
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

fn exec_remember(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let content = args["content"].as_str().ok_or("Missing 'content'")?;
    let tags: Vec<&str> = args["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let namespace = args["namespace"].as_str();
    let memory_type = args["type"].as_str().unwrap_or("fact");
    let room = args["room"].as_str();
    let author = args["author"].as_str().unwrap_or("anonymous");

    let id = if let Some(room_id) = room {
        uteke
            .remember_in_room(
                content,
                &tags,
                None,
                namespace,
                memory_type,
                room_id,
                author,
            )
            .map_err(|e| format!("Failed: {e}"))?
    } else {
        uteke
            .remember_typed(content, &tags, None, namespace, memory_type)
            .map_err(|e| format!("Failed: {e}"))?
    };

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("✓ Stored memory with ID: {id}"),
        }],
        is_error: false,
    })
}

fn exec_recall(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let query = args["query"].as_str().ok_or("Missing 'query'")?;
    let limit = args["limit"].as_u64().unwrap_or(5) as usize;
    let namespace = args["namespace"].as_str();

    let tags_filter: Option<Vec<&str>> = args["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>());
    let tags_ref = tags_filter.as_deref();
    let min_score = args["min_score"].as_f64().unwrap_or(0.0) as f32;

    // Parse optional search type (#531)
    let search_type = match args["type"].as_str() {
        Some("memory") => uteke_core::SearchType::Memory,
        Some("doc") => uteke_core::SearchType::Document,
        Some("all") | None => uteke_core::SearchType::All,
        Some(other) => {
            return Err(format!(
                "Invalid search type: '{other}'. Use 'all', 'memory', or 'doc'."
            ));
        }
    };

    // Parse optional recall strategy (#1035): default fusion since 0.16.0
    // (#1123), matching the CLI and HTTP defaults. Unknown values are a loud
    // error (JSON-RPC -32603), never a silent fallback.
    let strategy = match args["strategy"].as_str() {
        Some(name) => match uteke_core::RecallStrategy::from_str_opt(name) {
            Some(s) => s,
            None => {
                return Err(format!(
                    "Invalid strategy: '{name}'. Use 'vector', 'fts5', 'hybrid', 'graph', or 'fusion'."
                ));
            }
        },
        None => uteke_core::RecallStrategy::Fusion,
    };

    // Explain mode (#1160): memory-only, bypasses unified results and
    // returns full ranking signals per result. An omitted `type` (the
    // documented default invocation) is accepted and treated as memory
    // recall — only explicit all/doc are rejected (code-scanning fix:
    // None maps to SearchType::All above, so the previous enum comparison
    // wrongly rejected the default call).
    if args["explain"].as_bool().unwrap_or(false) {
        if !matches!(args["type"].as_str(), None | Some("memory")) {
            return Err(
                "explain is memory-only: pass \"type\": \"memory\" (or drop type)".to_string(),
            );
        }
        let explained = uteke
            .recall_explained(query, limit, tags_ref, namespace, strategy, min_score)
            .map_err(|e| format!("Failed: {e}"))?;
        if explained.is_empty() {
            return Ok(ToolResult {
                content: vec![McpContent::Text {
                    r#type: "text".to_string(),
                    text: "No results found.".to_string(),
                }],
                is_error: false,
            });
        }
        let text = serde_json::to_string_pretty(&explained).unwrap_or_else(|_| "[]".to_string());
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text,
            }],
            is_error: false,
        });
    }

    // Use unified search when type is specified or default (all).
    // Fall back to legacy recall only for backward compat with existing MCP consumers.
    let results = uteke
        .recall_unified(
            query,
            limit,
            tags_ref,
            namespace,
            min_score,
            search_type,
            None,
            None,
            false,
            strategy,
        )
        .map_err(|e| format!("Failed: {e}"))?;

    if results.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No results found.".to_string(),
            }],
            is_error: false,
        });
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
                .map(|id| format!(" (id: {})", &id[..id.len().min(8)]))
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
        // don't act on stale info. Deprecated memories are already excluded
        // from recall by default; this covers legacy/hard-restored rows.
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

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: lines.join("\n"),
        }],
        is_error: false,
    })
}

fn exec_list(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let tag = args["tag"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(20) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let namespace = args["namespace"].as_str();

    let memories = uteke
        .list(tag, limit, offset, namespace)
        .map_err(|e| format!("Failed: {e}"))?;

    if memories.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No memories found.".to_string(),
            }],
            is_error: false,
        });
    }

    let lines: Vec<String> = memories
        .iter()
        .map(|m| {
            let short_id = m.id.get(..8).unwrap_or(&m.id);
            format!("[{short_id}] {} ({})", m.content, m.tags.join(", "))
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

/// Resolve an id argument to a full UUID (#1048).
///
/// Accepts the full UUID or any unambiguous prefix (e.g. the 8-char ids
/// printed by recall/list). Errors loudly on ambiguous prefixes instead of
/// silently no-oping. Exact UUIDs skip the prefix scan.
fn resolve_id<'a>(uteke: &'a Uteke, id: &'a str) -> Result<String, String> {
    if id.len() == 36 {
        return Ok(id.to_string());
    }
    match uteke.resolve_id_prefix(id) {
        Ok(Some(full)) => Ok(full),
        Ok(None) => Err(format!("No memory matches id prefix '{id}'")),
        Err(e) => Err(format!("{e}")),
    }
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

    let updated = uteke
        .update_memory(
            &id,
            content,
            tags_ref,
            metadata,
            importance,
            pinned,
            memory_type,
        )
        .map_err(|e| format!("Failed: {e}"))?;

    if !updated {
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
            text: format!("✓ Updated memory {id}"),
        }],
        is_error: false,
    })
}

fn exec_forget(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id_arg = args["id"].as_str().ok_or("Missing 'id'")?;
    let id = resolve_id(uteke, id_arg)?;

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

    let docs = uteke.doc_list(limit).map_err(|e| format!("Failed: {e}"))?;

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

    let data = uteke
        .graph_data(namespace)
        .map_err(|e| format!("Failed: {e}"))?;

    let text = format!(
        "Graph: {} nodes, {} edges, {} relation types",
        data.nodes.len(),
        data.edges.len(),
        data.stats.relation_types.len()
    );

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

    let conn = uteke.graph_store();
    let gs = uteke_core::graph::GraphStore::new(conn);
    gs.add_edge(&source, &target, edge_type, weight)
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

    let conn = uteke.graph_store();
    let gs = uteke_core::graph::GraphStore::new(conn);
    let removed = gs
        .remove_edge(&source, &target)
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

fn exec_room_recall(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;
    let query = args["query"].as_str().ok_or("Missing 'query'")?;
    let limit = args["limit"].as_u64().unwrap_or(5) as usize;

    let results = uteke
        .recall_room_semantic(room_id, query, limit, None, 0.0)
        .map_err(|e| format!("Failed: {e}"))?;

    if results.is_empty() {
        return Ok(ToolResult {
            content: vec![McpContent::Text {
                r#type: "text".to_string(),
                text: "No memories found in room.".to_string(),
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

fn exec_room_memories(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let room_id = args["room_id"].as_str().ok_or("Missing 'room_id'")?;
    let author = args["author"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(100) as usize;

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
            let short_id: String = m.id.chars().take(8).collect();
            format!("[{created} | {} | {}] {}", short_id, m.namespace, m.content)
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
            text: format!("Room created: {room_id} (namespace: {namespace})"),
        }],
        is_error: false,
    })
}

fn exec_room_list(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();

    let rooms = uteke
        .list_rooms(namespace)
        .map_err(|e| format!("Failed: {e}"))?;

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
        "Room '{room_id}' deleted. {unlinked} memory link(s) removed; the memories themselves are preserved in their namespaces (no longer linked to any room)."
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
        };
        uteke.store().insert(&m).unwrap();
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
            };
            uteke.store().insert(&m).unwrap();
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
            };
            uteke.store().insert(&m).unwrap();
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
