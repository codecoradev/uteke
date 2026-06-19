//! uteke-mcp library — shared MCP protocol handler.
//!
//! Used by both the stdio binary (`uteke-mcp`) and the HTTP endpoint
//! on `uteke-server` (`POST /mcp`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uteke_core::Uteke;

// ── JSON-RPC types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
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
/// HTTP endpoint. Returns a `JsonRpcResponse` ready for serialization.
pub fn handle_jsonrpc(uteke: &Uteke, raw: &str) -> String {
    let req: JsonRpcRequest = match serde_json::from_str(raw) {
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
            return serde_json::to_string(&resp).unwrap_or_default();
        }
    };

    let id = req.id.clone().unwrap_or(Value::Null);

    match handle_request(uteke, &req.method, req.params) {
        Ok(result) => serde_json::to_string(&JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        })
        .unwrap_or_default(),
        Err(msg) => serde_json::to_string(&JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32603,
                message: msg,
            }),
        })
        .unwrap_or_default(),
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
        "description": "Store a new memory in uteke. The content will be embedded and indexed for semantic search. Optionally specify tags for categorization and a room for collaborative memory.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "The text content to remember" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for categorization (optional)" },
                "namespace": { "type": "string", "description": "Namespace for isolation (default: 'default')" }
            },
            "required": ["content"]
        }
    })
}

fn tool_recall() -> Value {
    serde_json::json!({
        "name": "uteke_recall",
        "description": "Semantic search over stored memories. Returns the most relevant memories ranked by embedding similarity.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query" },
                "limit": { "type": "integer", "description": "Max results (default 5)", "default": 5 },
                "namespace": { "type": "string", "description": "Namespace to search (default: 'default')" }
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

// ── Tool Executors ──────────────────────────────────────────────────────────

fn exec_remember(uteke: &Uteke, args: &Value) -> Result<ToolResult, String> {
    let content = args["content"].as_str().ok_or("Missing 'content'")?;
    let tags: Vec<&str> = args["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let namespace = args["namespace"].as_str();

    let id = uteke
        .remember(content, &tags, None, namespace)
        .map_err(|e| format!("Failed: {e}"))?;

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

    let results = uteke
        .recall(query, limit, None, namespace, 0.0)
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

    let mut lines = Vec::new();
    for (i, sr) in results.iter().enumerate() {
        lines.push(format!("[{:.2}] {}", sr.score, sr.memory.content));
        let _ = i;
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
    let namespace = args["namespace"].as_str();

    let memories = uteke
        .list(tag, limit, 0, namespace)
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
        .map(|m| format!("[{}] {} ({})", &m.id[..8], m.content, m.tags.join(", ")))
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
