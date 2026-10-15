//! MCP-over-HTTP handler.

use super::*;

pub(super) fn post_mcp(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let read_only = c.read_only;
    // Enforce a body size limit to prevent memory exhaustion
    // (CodeCora #397). 1 MiB is generous for JSON-RPC.
    const MAX_MCP_BODY: u64 = 1024 * 1024;
    // Check Content-Length and reject oversized requests.
    let content_length = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Content-Length"))
        .and_then(|h| h.value.as_str().parse::<u64>().ok())
        .unwrap_or(0);
    if content_length > MAX_MCP_BODY {
        return ctx.error_response_for(req, 413, "Payload too large");
    }
    // Read one byte past the cap so a chunked/unsized oversized body is
    // a 413 instead of a silently truncated JSON parse error (#1328).
    let mut body = String::new();
    if let Err(e) = req
        .as_reader()
        .take(MAX_MCP_BODY + 1)
        .read_to_string(&mut body)
    {
        return ctx.error_response_for(req, 400, format!("Failed to read body: {e}"));
    }
    if body.len() as u64 > MAX_MCP_BODY {
        return ctx.error_response_for(req, 413, "Payload too large");
    }
    let cors = ctx.cors_headers_for(req);
    // None = notification (no response per JSON-RPC 2.0 §4.1) → 204 No Content
    // Read-only tokens reach /mcp (#1439); uteke-mcp refuses the
    // tools outside READ_ONLY_TOOLS as tool errors.
    let access = if read_only {
        uteke_mcp::Access::ReadOnly
    } else {
        uteke_mcp::Access::Full
    };
    let mut resp = match uteke_mcp::handle_jsonrpc_as(uteke, &body, access) {
        Some(response) => tiny_http::Response::from_string(response)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap(),
            )
            .with_header(
                tiny_http::Header::from_bytes(&b"MCP-Protocol-Version"[..], &b"2025-06-18"[..])
                    .unwrap(),
            ),
        None => tiny_http::Response::from_string("")
            .with_status_code(204)
            .with_header(
                tiny_http::Header::from_bytes(&b"MCP-Protocol-Version"[..], &b"2025-06-18"[..])
                    .unwrap(),
            ),
    };
    for h in cors {
        resp.add_header(h);
    }
    resp
}
