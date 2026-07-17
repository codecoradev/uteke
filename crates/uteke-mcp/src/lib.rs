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
                tool_room_recall(),
                tool_room_memories(),
                tool_room_stats(),
                tool_room_summary(),
                tool_room_document(),
                tool_tags_list(),
                tool_tags_rename(),
                tool_tags_delete(),
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
                "uteke_room_recall" => exec_room_recall(uteke, &arguments)?,
                "uteke_room_memories" => exec_room_memories(uteke, &arguments)?,
                "uteke_room_stats" => exec_room_stats(uteke, &arguments)?,
                "uteke_room_summary" => exec_room_summary(uteke, &arguments)?,
                "uteke_room_document" => exec_room_document(uteke, &arguments)?,
                "uteke_tags_list" => exec_tags_list(uteke, &arguments)?,
                "uteke_tags_rename" => exec_tags_rename(uteke, &arguments)?,
                "uteke_tags_delete" => exec_tags_delete(uteke, &arguments)?,
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
                "type": { "type": "string", "enum": ["all", "memory", "doc"], "description": "Search type: 'all' (default, unified), 'memory', or 'doc'" }
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

fn tool_forget() -> Value {
    serde_json::json!({
        "name": "uteke_forget",
        "description": "Delete a memory by its ID.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The memory ID (UUID)" }
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
        "description": "Run the dream cycle maintenance pipeline: lint → backlinks → dedup → orphans → compact → verify. Cleans up and optimizes the memory store. Safe to run periodically.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "Namespace to process (default: all)" },
                "dry_run": { "type": "boolean", "description": "Preview changes without applying (default: false)" },
                "phases": { "type": "array", "items": { "type": "string" }, "description": "Specific phases: lint, backlinks, dedup, orphans, compact, verify (default: all)" }
            }
        }
    })
}

fn tool_room_recall() -> Value {
    serde_json::json!({
        "name": "uteke_room_recall",
        "description": "Semantic recall within a room context. Searches across all namespaces in the room using hybrid RRF ranking.",
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
        "name": "uteke_room_document",
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

fn tool_pin() -> Value {
    serde_json::json!({
        "name": "uteke_pin",
        "description": "Pin a memory so it never decays. Pinned memories are immune to aging and pruning during maintenance cycles.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The memory ID (UUID)" }
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
                "id": { "type": "string", "description": "The memory ID (UUID)" }
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
            ))
        }
    };

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

fn exec_forget(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id = args["id"].as_str().ok_or("Missing 'id'")?;

    uteke.forget(id).map_err(|e| format!("Failed: {e}"))?;

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

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!(
                "Total: {} | Tags: {} | DB: {} bytes",
                stats.total_memories, stats.unique_tags, stats.db_size_bytes
            ),
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
        .map(|d| format!("{} — {}", d.document.slug, d.document.title))
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
        .doc_move(id, parent)
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
    let source = args["source"].as_str().ok_or("Missing 'source'")?;
    let target = args["target"].as_str().ok_or("Missing 'target'")?;
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
    match uteke.get_by_id(source) {
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
    match uteke.get_by_id(target) {
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
    gs.add_edge(source, target, edge_type, weight)
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
    let source = args["source"].as_str().ok_or("Missing 'source'")?;
    let target = args["target"].as_str().ok_or("Missing 'target'")?;

    let conn = uteke.graph_store();
    let gs = uteke_core::graph::GraphStore::new(conn);
    let removed = gs
        .remove_edge(source, target)
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
    let dry_run = args["dry_run"].as_bool().unwrap_or(false);

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

    let report = uteke
        .dream(namespace, dry_run, &phases)
        .map_err(|e| format!("Failed: {e}"))?;

    let mut lines = vec![format!(
        "Dream cycle complete: {} changes, {} warnings, {} errors ({}ms{})",
        report.total_changes,
        report.total_warnings,
        report.total_errors,
        report.duration_ms,
        if dry_run { " [DRY RUN]" } else { "" }
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
            let created = m.created_at.format("%Y-%m-%d %H:%M");
            format!("[{created} | {}] {}", m.namespace, m.content)
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

    uteke
        .delete_room(room_id)
        .map_err(|e| format!("Failed to delete room: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("Room deleted: {room_id}"),
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

fn exec_pin(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id = args["id"].as_str().ok_or("Missing 'id'")?;

    match uteke.pin(id) {
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
    let id = args["id"].as_str().ok_or("Missing 'id'")?;

    match uteke.unpin(id) {
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
