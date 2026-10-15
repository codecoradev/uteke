//! Namespace handlers: list, rename and delete.

use super::*;

pub(super) fn get_namespaces(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let with_counts = path.contains("with_counts=true");
    if with_counts {
        match uteke.list_namespaces_with_lifecycle_counts() {
            Ok(counts) => {
                #[derive(serde::Serialize)]
                struct NamespaceCount {
                    name: String,
                    count: usize,
                    /// Memories not deprecated (#1181).
                    active: usize,
                    /// Deprecated memories — additive fields, `count` keeps
                    /// the total so existing clients stay correct (#1181).
                    deprecated: usize,
                }
                let result: Vec<NamespaceCount> = counts
                    .into_iter()
                    .map(|(name, active, deprecated)| NamespaceCount {
                        count: active + deprecated,
                        name,
                        active,
                        deprecated,
                    })
                    .collect();
                ctx.ok_response_for(req, &result)
            }
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        }
    } else {
        match uteke.list_namespaces() {
            Ok(namespaces) => ctx.ok_response_for(req, &namespaces),
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        }
    }
}

pub(super) fn post_namespaces_rename(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<NamespaceRenameRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.rename_namespace(&req_data.from, &req_data.to) {
            Ok(result) => ctx.ok_response_for(req, &result),
            Err(uteke_core::Error::Validation(msg)) => ctx.error_response_for(req, 400, msg),
            Err(e) => {
                error!("Namespace rename error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_namespaces_delete(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<NamespaceDeleteRequest>(req.as_reader()) {
        Ok(req_data) => {
            match uteke.delete_namespace(
                &req_data.name,
                &req_data.strategy,
                req_data.target.as_deref(),
            ) {
                Ok(result) => ctx.ok_response_for(req, &result),
                Err(uteke_core::Error::Validation(msg)) => {
                    // `refuse` on a non-empty namespace → 409 Conflict;
                    // other validation failures → 400.
                    let status = if req_data.strategy == "refuse" {
                        409
                    } else {
                        400
                    };
                    ctx.error_response_for(req, status, msg)
                }
                Err(e) => {
                    error!("Namespace delete error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}
