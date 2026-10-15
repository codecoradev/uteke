//! Tag handlers: list, rename and delete.

use super::*;

pub(super) fn get_tags(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let ns = parse_query_namespace(path);
    match uteke.tags_with_counts(ns.as_deref()) {
        Ok(tags) => ctx.ok_response_for(req, &tags),
        Err(e) => {
            error!("Tags list error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_tags_rename(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<TagRenameRequest>(req.as_reader()) {
        Ok(req_data) => {
            let count =
                match uteke.rename_tag(&req_data.old, &req_data.new, ns(&req_data.namespace)) {
                    Ok(count) => count,
                    Err(e) => {
                        error!("Tag rename error: {e}");
                        return ctx.error_response_for(req, 500, "Internal server error");
                    }
                };
            ctx.ok_response_for(
                req,
                &serde_json::json!({
                    "renamed": count > 0,
                    "count": count,
                    "old": req_data.old,
                    "new": req_data.new,
                }),
            )
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_tags_delete(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<TagDeleteRequest>(req.as_reader()) {
        Ok(req_data) => {
            let count = match uteke.delete_tag(&req_data.tag, ns(&req_data.namespace)) {
                Ok(count) => count,
                Err(e) => {
                    error!("Tag delete error: {e}");
                    return ctx.error_response_for(req, 500, "Internal server error");
                }
            };
            ctx.ok_response_for(
                req,
                &serde_json::json!({
                    "deleted": count > 0,
                    "count": count,
                    "tag": req_data.tag,
                }),
            )
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}
