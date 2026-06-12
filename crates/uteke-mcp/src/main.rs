//! Uteke MCP Server — Model Context Protocol interface for AI agents.
//!
//! Communicates via JSON-RPC over stdin/stdout (stdio transport).
//! Exposes uteke memory operations as MCP tools that AI coding agents
//! (Claude Code, Cursor, Copilot, etc.) can call directly.
//!
//! ## MCP Tools Exposed
//!
//! | Tool | Description |
//! |------|-------------|
//! | `uteke_remember` | Store a new memory |
//! | `uteke_recall` | Semantic search memories |
//! | `uteke_list` | List/filter memories |
//! | `uteke_forget` | Delete a memory |
//! | `uteke_stats` | Get memory statistics |
//!
//! ## Usage
//!
//! Add to your MCP client config (e.g., Claude Code's `.claude/settings.json`):
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "uteke": {
//!       "command": "uteke-mcp",
//!       "args": []
//!     }
//!   }
//! }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write};
use uteke_core::Uteke;

// ── JSON-RPC types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

// ── MCP Protocol types ──────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "role")]
enum McpContent {
    #[serde(rename = "text")]
    Text { r#type: String, text: String },
}

#[derive(Serialize)]
// Tool definitions are used via serde_json::json! in tool_*() functions below.
// ToolDefinition struct kept for documentation purposes.
#[allow(dead_code)]
struct ToolDefinition {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Serialize)]
struct ToolResult {
    content: Vec<McpContent>,
    #[serde(rename = "isError")]
    is_error: bool,
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    // Open uteke store
    let store_path = dirs::home_dir()
        .map(|h| h.join(".uteke"))
        .expect("Cannot determine home directory");

    let uteke = match Uteke::open(&store_path) {
        Ok(u) => u,
        Err(e) => {
            let resp = JsonRpcResponse {
                jsonrpc: "2.0",
                id: Value::Null,
                result: None,
                error: Some(JsonRpcError {
                    code: -32603,
                    message: format!("Failed to open uteke store: {e}"),
                }),
            };
            let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap());
            std::process::exit(1);
        }
    };

    // Read JSON-RPC messages from stdin (one per line)
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {e}"),
                    }),
                };
                let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap());
                stdout.flush().ok();
                continue;
            }
        };

        let id = request.id.unwrap_or(Value::Null);
        let result = handle_request(&uteke, &request.method, request.params);

        let resp = match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(value),
                error: None,
            },
            Err(msg) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32603,
                    message: msg,
                }),
            },
        };

        let _ = writeln!(stdout, "{}", serde_json::to_string(&resp).unwrap());
        stdout.flush().ok();
    }
}

fn handle_request(uteke: &Uteke, method: &str, params: Option<Value>) -> Result<Value, String> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2024-11-05",
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
                tool_list(),
                tool_forget(),
                tool_stats(),
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
                "uteke_list" => exec_list(uteke, &arguments)?,
                "uteke_forget" => exec_forget(uteke, &arguments)?,
                "uteke_stats" => exec_stats(uteke, &arguments)?,
                _ => {
                    return Err(format!("Unknown tool: {tool_name}"));
                }
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
        "description": "Store a new memory in uteke. The content will be embedded and indexed for semantic search. Optionally specify tags for categorization and a room for collaborative memory.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "The content to remember" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for categorization" },
                "type": { "type": "string", "enum": ["fact", "procedure", "preference", "decision", "context"], "description": "Memory type (default: fact)" },
                "namespace": { "type": "string", "description": "Namespace for multi-agent isolation (default: 'default')" },
                "room": { "type": "string", "description": "Room ID to link this memory to a collaborative context" },
                "author": { "type": "string", "description": "Author attribution when storing in a room" }
            },
            "required": ["content"]
        }
    })
}

fn tool_recall() -> Value {
    serde_json::json!({
        "name": "uteke_recall",
        "description": "Search memories by semantic similarity. Returns the most relevant memories matching the query. Uses hybrid search (vector + full-text) for best results.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query" },
                "limit": { "type": "integer", "description": "Maximum results (default: 5)" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Filter by tags" },
                "namespace": { "type": "string", "description": "Filter by namespace" },
                "min_score": { "type": "number", "description": "Minimum similarity score 0.0-1.0 (default: 0.0)" }
            },
            "required": ["query"]
        }
    })
}

fn tool_list() -> Value {
    serde_json::json!({
        "name": "uteke_list",
        "description": "List memories with optional filters. Useful for browsing stored memories.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tag": { "type": "string", "description": "Filter by tag" },
                "limit": { "type": "integer", "description": "Maximum results (default: 20)" },
                "offset": { "type": "integer", "description": "Pagination offset (default: 0)" },
                "namespace": { "type": "string", "description": "Filter by namespace" }
            }
        }
    })
}

fn tool_forget() -> Value {
    serde_json::json!({
        "name": "uteke_forget",
        "description": "Delete a memory by ID. This is permanent and cannot be undone.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Memory ID (UUID) to delete" }
            },
            "required": ["id"]
        }
    })
}

fn tool_stats() -> Value {
    serde_json::json!({
        "name": "uteke_stats",
        "description": "Get memory store statistics: total memories, unique tags, database size, aging tier counts.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "description": "Filter stats by namespace" }
            }
        }
    })
}

// ── Tool Execution ──────────────────────────────────────────────────────────

fn exec_remember(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let content = args["content"]
        .as_str()
        .ok_or("Missing 'content' parameter")?;
    let tags: Vec<&str> = args
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let memory_type = args["type"].as_str().unwrap_or("fact");
    let namespace = args["namespace"].as_str();
    let room = args["room"].as_str();
    let author = args["author"].as_str();

    let id = if let Some(room_id) = room {
        let author_name = author.unwrap_or("mcp-client");
        uteke
            .remember_in_room(
                content,
                &tags,
                None,
                namespace,
                memory_type,
                room_id,
                author_name,
            )
            .map_err(|e| format!("Failed to store memory: {e}"))?
    } else {
        uteke
            .remember(content, &tags, None, namespace)
            .map_err(|e| format!("Failed to store memory: {e}"))?
    };

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("Memory stored with ID: {id}"),
        }],
        is_error: false,
    })
}

fn exec_recall(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let query = args["query"].as_str().ok_or("Missing 'query' parameter")?;
    let limit = args["limit"].as_u64().unwrap_or(5) as usize;
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let namespace = args["namespace"].as_str();
    let min_score = args["min_score"].as_f64().unwrap_or(0.0) as f32;

    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

    let results = uteke
        .recall_hybrid(
            query,
            limit,
            if tag_refs.is_empty() {
                None
            } else {
                Some(&tag_refs)
            },
            namespace,
            uteke_core::RecallStrategy::Hybrid,
            min_score,
        )
        .map_err(|e| format!("Recall failed: {e}"))?;

    let output: Vec<Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.memory.id,
                "content": r.memory.content,
                "score": format!("{:.3}", r.score),
                "tags": r.memory.tags,
                "namespace": r.memory.namespace,
                "created_at": r.memory.created_at.to_rfc3339(),
            })
        })
        .collect();

    let text = if output.is_empty() {
        "No memories found.".to_string()
    } else {
        serde_json::to_string_pretty(&output).unwrap_or_default()
    };

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
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
        .map_err(|e| format!("List failed: {e}"))?;

    let output: Vec<Value> = memories
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "content": m.content,
                "tags": m.tags,
                "namespace": m.namespace,
                "created_at": m.created_at.to_rfc3339(),
            })
        })
        .collect();

    let text = if output.is_empty() {
        "No memories found.".to_string()
    } else {
        serde_json::to_string_pretty(&output).unwrap_or_default()
    };

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}

fn exec_forget(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let id = args["id"].as_str().ok_or("Missing 'id' parameter")?;

    uteke
        .forget(id)
        .map_err(|e| format!("Forget failed: {e}"))?;

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text: format!("Memory {id} deleted."),
        }],
        is_error: false,
    })
}

fn exec_stats(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let namespace = args["namespace"].as_str();

    let stats = uteke
        .stats(namespace)
        .map_err(|e| format!("Stats failed: {e}"))?;

    let text = serde_json::to_string_pretty(&stats).unwrap_or_default();

    Ok(ToolResult {
        content: vec![McpContent::Text {
            r#type: "text".to_string(),
            text,
        }],
        is_error: false,
    })
}
