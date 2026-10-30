//! Document handlers: create, get, update, list, search, move, delete and memory references.

use super::*;

pub(super) fn post_doc_create(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<DocCreateRequest>(req.as_reader()) {
        Ok(req_data) => {
            let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
            let parent = req_data.parent.as_deref();
            match uteke.doc_upsert_with_parent(
                &req_data.slug,
                req_data.title.as_deref().unwrap_or(""),
                &req_data.content,
                &tag_refs,
                None,
                parent,
            ) {
                Ok(id) => {
                    ctx.ok_response_for(req, &serde_json::json!({"id": id, "slug": req_data.slug}))
                }
                Err(e) => {
                    if e.to_string().contains("already exists") {
                        ctx.error_response_for(
                            req,
                            409,
                            format!("document slug '{}' already exists", req_data.slug),
                        )
                    } else if e.to_string().contains("maximum") || e.to_string().contains("parent")
                    {
                        ctx.error_response_for(req, 400, e.to_string())
                    } else {
                        error!("doc create error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_doc_get(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<DocGetRequest>(req.as_reader()) {
        Ok(req_data) => match resolve_doc_id(&req_data) {
            Ok(id_or_slug) => match uteke.doc_get(id_or_slug) {
                Ok(Some(doc)) => ctx.ok_response_for(req, &doc),
                Ok(None) => {
                    ctx.error_response_for(req, 404, format!("document not found: {id_or_slug}"))
                }
                Err(e) => {
                    error!("doc get error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            },
            Err(e) => ctx.error_response_for(req, 400, e),
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_doc_update(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<DocUpdateRequest>(req.as_reader()) {
        Ok(req_data) => match resolve_doc_id_update(&req_data) {
            Ok(id_or_slug) => {
                let title = req_data.title.as_deref();
                let content = req_data.content.as_deref();
                let tags = req_data.tags.as_deref();
                let metadata = req_data.metadata.as_ref();
                match uteke.doc_update(id_or_slug, title, content, tags, metadata) {
                    Ok(Some(doc)) => ctx.ok_response_for(req, &doc),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("document not found: {id_or_slug}"),
                    ),
                    Err(e) => {
                        error!("doc update error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_doc_list(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<DocListParams>(req.as_reader()) {
        Ok(mut params) => {
            params.limit = params.limit.min(MAX_LIST_LIMIT);
            let result = if params.roots_only {
                uteke.doc_list_roots(params.namespace.as_deref(), params.limit)
            } else if let Some(ref parent) = params.parent {
                uteke.doc_list_children(parent, params.limit)
            } else {
                uteke.doc_list(params.namespace.as_deref(), params.limit)
            };
            match result {
                Ok(docs) => ctx.ok_response_for(req, &docs),
                Err(e) => {
                    error!("doc list error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_doc_search(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<DocSearchRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.doc_search(&req_data.query, req_data.limit, &req_data.mode) {
            Ok(results) => ctx.ok_response_for(req, &results),
            Err(e) => {
                if e.to_string().contains("embed") {
                    ctx.error_response_for(
                        req,
                        503,
                        "embedding model not available for semantic search",
                    )
                } else {
                    error!("doc search error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_doc_move(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<DocMoveRequest>(req.as_reader()) {
        Ok(req_data) => match resolve_doc_id_move(&req_data) {
            Ok(id_or_slug) => {
                let new_parent = req_data.new_parent.as_deref();
                let new_sort_order = req_data.new_sort_order;
                match uteke.doc_move(id_or_slug, new_parent, new_sort_order) {
                    Ok(moved) => ctx.ok_response_for(req, &serde_json::json!({"moved": moved})),
                    Err(e) if e.class() == uteke_core::ErrorClass::NotFound => {
                        ctx.error_response_for(req, 404, e.to_string())
                    }
                    Err(e) => {
                        error!("doc move error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn delete_doc_delete(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    // Extract query string only — req.url() returns full URL which
    // parse_query_param() cannot handle (#776).
    let query = req.url().split('?').nth(1).unwrap_or("");
    let id = parse_query_param(query, "id");
    let slug = parse_query_param(query, "slug");

    let id_or_slug = match (&id, &slug) {
        (Some(id), _) => id.as_str(),
        (_, Some(slug)) => slug.as_str(),
        _ => {
            return ctx.error_response_for(
                req,
                400,
                "provide either 'id' or 'slug' query parameter",
            );
        }
    };

    match uteke.doc_delete(id_or_slug) {
        Ok((deleted, subtree)) => ctx.ok_response_for(
            req,
            &serde_json::json!({"deleted": deleted, "subtree_size": subtree}),
        ),
        Err(e) => {
            error!("doc delete error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

// POST /doc/mem-refs — get memory IDs that reference a document
pub(super) fn post_doc_mem_refs(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct DocMemRefsReq {
        doc_slug: String,
    }
    match read_body::<DocMemRefsReq>(req.as_reader()) {
        Ok(req_data) => match uteke.recall_memories_for_document(&req_data.doc_slug) {
            Ok(memory_ids) => ctx.ok_response_for(
                req,
                &serde_json::json!({
                    "doc_slug": req_data.doc_slug,
                    "memory_ids": memory_ids,
                }),
            ),
            Err(e) => {
                error!("doc/mem-refs error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}
