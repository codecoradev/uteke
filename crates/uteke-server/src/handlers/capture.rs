//! Capture queue handlers (#1466, LAB): queue a raw turn, read the queue.
//! Extraction runs on the server's schedule (`capture_worker`).

use super::*;

/// Refusal while `[capture] enabled` is off: capture is opt-in.
const CAPTURE_OFF: &str =
    "Capture is off: set `enabled = true` under [capture] in the global uteke.toml";

pub(super) fn post_capture(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    if !uteke.capture_settings().enabled {
        return ctx.error_response_for(req, 409, CAPTURE_OFF);
    }
    let body = match read_body::<CaptureRequest>(req.as_reader()) {
        Ok(b) => b,
        Err(e) => return ctx.error_response_for(req, 400, e),
    };
    if validate_content_size(&body.content, 1_048_576).is_err() {
        return ctx.error_response_for(req, 413, "Content too large (max 1MB)");
    }
    let role: uteke_core::capture::Role = match body.role.parse() {
        Ok(r) => r,
        Err(e) => return ctx.core_error(req, &e),
    };
    match uteke.capture(
        &body.session_id,
        role,
        &body.content,
        body.source.as_deref(),
        ns(&body.namespace),
    ) {
        Ok(out) => {
            let value = serde_json::to_value(&out).unwrap_or_default();
            let value = with_namespace_warning(value, ns(&body.namespace));
            ctx.ok_response_for(req, &value)
        }
        Err(e) => ctx.core_error(req, &e),
    }
}

pub(super) fn get_capture_status(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match uteke.capture_status() {
        Ok(status) => ctx.ok_response_for(req, &status),
        Err(e) => ctx.core_error(req, &e),
    }
}
