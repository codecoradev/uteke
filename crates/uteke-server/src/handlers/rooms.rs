//! Room handlers: room CRUD, room memories/recall, room summaries and room-document links.

use super::*;

pub(super) fn post_room_summary(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomSummaryRequest {
        room_id: String,
    }
    match read_body::<RoomSummaryRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.room_summary(&req_data.room_id) {
            Ok(Some(summary)) => ctx.ok_response_for(req, &summary),
            Ok(None) => {
                ctx.error_response_for(req, 404, format!("Room not found: {}", req_data.room_id))
            }
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_room_summary_document(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomSummaryDocumentRequest {
        room_id: String,
    }
    match read_body::<RoomSummaryDocumentRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.room_summary_document(&req_data.room_id) {
            Ok(Some(doc)) => ctx.ok_response_for(req, &doc),
            Ok(None) => {
                ctx.error_response_for(req, 404, format!("Room not found: {}", req_data.room_id))
            }
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// Dry-run by default; `apply: true` executes with a hard budget cap.
// Blocked for read-only tokens (not in read_only_post_paths).
pub(super) fn post_room_consolidate(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomConsolidateRequest {
        room_id: String,
        /// Execute the plan (LLM calls + writes). Default false = dry-run.
        #[serde(default)]
        apply: bool,
        /// Max LLM requests for this run. Default 10, hard cap 100.
        #[serde(default = "default_max_calls")]
        max_calls: usize,
    }
    fn default_max_calls() -> usize {
        10
    }
    match read_body::<RoomConsolidateRequest>(req.as_reader()) {
        Ok(req_data) => {
            let max_calls = req_data.max_calls.min(100);
            let result = if req_data.apply {
                let ext = resolve_extraction_config(ctx, None, None, None);
                if ext.api_key.is_empty() {
                    return ctx.error_response_for(
                        req,
                        503,
                        "Consolidation apply requires server-side extraction LLM config",
                    );
                }
                match uteke_core::consolidation_api::consolidate_room(
                    uteke,
                    &req_data.room_id,
                    &ext,
                    max_calls,
                ) {
                    Ok(exec) => serde_json::to_value(&exec).unwrap_or_default(),
                    Err(e) => {
                        error!("Internal error: {e}");
                        return ctx.error_response_for(req, 500, "Internal server error");
                    }
                }
            } else {
                match uteke_core::consolidation_api::plan_room(uteke, &req_data.room_id) {
                    Ok(dry) => serde_json::json!({
                        "room_id": req_data.room_id,
                        "dry_run": true,
                        "total_memories": dry.plan.total_memories,
                        "batches": dry.plan.batches.len(),
                        "estimated_llm_calls": dry.plan.batches.len().min(max_calls),
                        "plan": dry.plan,
                    }),
                    Err(e) => {
                        error!("Internal error: {e}");
                        return ctx.error_response_for(req, 500, "Internal server error");
                    }
                }
            };
            ctx.ok_response_for(req, &result)
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_room_document(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    warn!("DEPRECATED: POST /room/document is renamed to POST /room/summary-document (see #735)");
    #[derive(Deserialize)]
    struct RoomDocumentRequest {
        room_id: String,
    }
    match read_body::<RoomDocumentRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.room_summary_document(&req_data.room_id) {
            Ok(Some(doc)) => ctx.ok_response_for(req, &doc),
            Ok(None) => {
                ctx.error_response_for(req, 404, format!("Room not found: {}", req_data.room_id))
            }
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn get_room_memories(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query_str = path.split('?').nth(1);
    let room_id = query_str.and_then(|q| parse_query_param(q, "room_id"));
    let room_id = match room_id {
        Some(id) => id,
        None => {
            return ctx.error_response_for(
                req,
                400,
                "Missing required parameter: room_id. Usage: GET /room/memories?room_id=<id>[&author=<author>&namespace=<ns>&limit=<n>]",
            );
        }
    };
    let limit = query_str
        .and_then(|q| parse_query_param(q, "limit"))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100);
    let author = query_str.and_then(|q| parse_query_param(q, "author"));
    let namespace = query_str.and_then(|q| parse_query_param(q, "namespace"));
    match uteke.recall_room_scoped(&room_id, author.as_deref(), namespace.as_deref(), limit) {
        Ok(memories) => ctx.ok_response_for(req, &memories),
        Err(e) => {
            error!("Internal error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_room_recall(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<RoomRecallRequest>(req.as_reader()) {
        Ok(req_data) => {
            // Time-travel: parse & validate `at` before any query so an
            // invalid timestamp fails loudly (400) instead of being
            // silently ignored (#1082).
            let point_in_time = match req_data.at.as_deref() {
                Some(at_str) => match chrono::DateTime::parse_from_rfc3339(at_str) {
                    Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
                    Err(_) => {
                        return ctx.error_response_for(
                            req,
                            400,
                            format!(
                                "Invalid 'at' timestamp: {at_str}. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
                            ),
                        );
                    }
                },
                None => None,
            };
            let query = req_data.query.as_deref().unwrap_or("").trim();
            if query.is_empty() {
                // No query provided — fall back to chronological recall (#785)
                // Over-fetch when time-traveling: the SQL LIMIT applies
                // before the temporal post-filter, so fetch extra rows
                // and truncate after filtering (cora MAJOR on #1085).
                let fetch_limit = if point_in_time.is_some() && req_data.limit > 0 {
                    req_data.limit.saturating_mul(3).max(req_data.limit + 10)
                } else {
                    req_data.limit
                };
                match uteke.recall_room(&req_data.room_id, req_data.author.as_deref(), fetch_limit)
                {
                    Ok(memories) => {
                        let mut memories = match point_in_time {
                            Some(pit) => filter_room_memories_at_time(memories, pit),
                            None => memories,
                        };
                        if point_in_time.is_some() && req_data.limit > 0 {
                            memories.truncate(req_data.limit);
                        }
                        ctx.ok_response_for(req, &memories)
                    }
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            } else {
                // Semantic recall with query
                let min_score = req_data.min_score.unwrap_or(
                    ctx.recall_config
                        .as_ref()
                        .and_then(|r| r.min_score)
                        .unwrap_or(DEFAULT_MIN_SCORE as f64) as f32,
                );
                match uteke.recall_room_semantic(
                    &req_data.room_id,
                    query,
                    req_data.limit,
                    req_data.author.as_deref(),
                    min_score,
                ) {
                    Ok(results) => {
                        let results = match point_in_time {
                            Some(pit) => {
                                let mut kept: Vec<_> = results
                                    .into_iter()
                                    .filter(|sr| memory_exists_at(&sr.memory, pit))
                                    .collect();
                                kept.truncate(req_data.limit.max(1));
                                kept
                            }
                            None => results,
                        };
                        ctx.ok_response_for(req, &results)
                    }
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_room_create(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomCreateRequest {
        room_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default = "default_namespace")]
        namespace: String,
    }
    match read_body::<RoomCreateRequest>(req.as_reader()) {
        Ok(req_data) => {
            match uteke.create_room(
                &req_data.room_id,
                req_data.title.as_deref(),
                &req_data.namespace,
            ) {
                Ok(()) => ctx.ok_response_for(
                    req,
                    &with_namespace_warning(
                        serde_json::json!({
                            "created": req_data.room_id,
                            "namespace": req_data.namespace
                        }),
                        Some(req_data.namespace.as_str()),
                    ),
                ),
                Err(e) => {
                    let msg = format!("Failed to create room: {e}");
                    ctx.error_response_for(req, 400, &msg)
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// POST /room/rename — rename a room, moving all member/document
// links in one transaction (#1202)
pub(super) fn post_room_rename(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomRenameRequest {
        from: String,
        to: String,
    }
    match read_body::<RoomRenameRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.rename_room(&req_data.from, &req_data.to) {
            Ok(room) => ctx.ok_response_for(
                req,
                &serde_json::json!({
                    "renamed": req_data.from,
                    "to": req_data.to,
                    "room": room,
                }),
            ),
            Err(e) => {
                let status = match e {
                    uteke_core::Error::Validation(_) => 400,
                    _ => 500,
                };
                ctx.error_response_for(req, status, e.to_string())
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// POST /room/update — update room title/description (#1202)
pub(super) fn post_room_update(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomUpdateRequest {
        room_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        description: Option<String>,
    }
    match read_body::<RoomUpdateRequest>(req.as_reader()) {
        Ok(req_data) => {
            match uteke.update_room(
                &req_data.room_id,
                req_data.title.as_deref(),
                req_data.description.as_deref(),
            ) {
                Ok(Some(room)) => ctx.ok_response_for(req, &serde_json::json!({ "room": room })),
                Ok(None) => ctx.error_response_for(
                    req,
                    404,
                    format!("Room not found: {}", req_data.room_id),
                ),
                Err(e) => {
                    let status = match e {
                        uteke_core::Error::Validation(_) => 400,
                        _ => 500,
                    };
                    ctx.error_response_for(req, status, e.to_string())
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// POST /room/memory/move — move a memory from one room to another,
// preserving link author/role/joined_at (#1202)
pub(super) fn post_room_memory_move(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomMemoryMoveRequest {
        memory_id: String,
        from_room: String,
        to_room: String,
    }
    match read_body::<RoomMemoryMoveRequest>(req.as_reader()) {
        Ok(req_data) => {
            match uteke.move_memory_to_room(
                &req_data.memory_id,
                &req_data.from_room,
                &req_data.to_room,
            ) {
                Ok(1) => ctx.ok_response_for(
                    req,
                    &serde_json::json!({
                        "moved": req_data.memory_id,
                        "from_room": req_data.from_room,
                        "to_room": req_data.to_room,
                    }),
                ),
                Ok(0) => ctx.error_response_for(
                    req,
                    404,
                    format!(
                        "Memory {} has no link in room {}",
                        req_data.memory_id, req_data.from_room
                    ),
                ),
                Ok(_) => unreachable!("move returns 0 or 1"),
                Err(e) => {
                    let status = match e {
                        uteke_core::Error::Validation(_) => 400,
                        _ => 500,
                    };
                    ctx.error_response_for(req, status, e.to_string())
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// POST /room/remember — store memory and link to room (#762)
pub(super) fn post_room_remember(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<RoomRememberRequest>(req.as_reader()) {
        Ok(req_data) => {
            if let Err(e) = uteke_core::validate_input(&req_data.content, &req_data.tags) {
                return ctx.error_response_for(req, 400, e.to_string());
            }
            let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
            let memory_type = req_data.r#type.as_deref().unwrap_or("fact");
            let author = req_data.author.as_deref().unwrap_or("user");
            match uteke.remember_in_room(
                &req_data.content,
                &tag_refs,
                req_data.metadata.clone(),
                ns(&req_data.namespace),
                memory_type,
                &req_data.room_id,
                author,
            ) {
                Ok(outcome) => ctx.ok_response_for(
                    req,
                    &serde_json::json!({
                        "id": outcome.id,
                        "room_id": req_data.room_id,
                        "embedding_written": outcome.embedding_written,
                        "warning": uteke_core::namespace_deprecation::merge_warning(
                            outcome.warning,
                            uteke_core::namespace_deprecation::namespace_write_warning(
                                ns(&req_data.namespace),
                            ),
                        ),
                    }),
                ),
                Err(e) => {
                    error!("room/remember error: {e}");
                    let status = match e {
                        uteke_core::Error::Validation(_) => 400,
                        _ => 500,
                    };
                    ctx.error_response_for(req, status, e.to_string())
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn get_room_list(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let ns_param = parse_query_namespace(path);
    match uteke.list_rooms(ns_param.as_deref()) {
        Ok(rooms) => ctx.ok_response_for(req, &rooms),
        Err(e) => {
            error!("Internal error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_room_stats(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomStatsRequest {
        room_id: String,
    }
    match read_body::<RoomStatsRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.room_stats(&req_data.room_id) {
            Ok(Some(stats)) => ctx.ok_response_for(req, &stats),
            Ok(None) => {
                ctx.error_response_for(req, 404, format!("Room not found: {}", req_data.room_id))
            }
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// POST /room/document/list — list documents linked to a room
pub(super) fn post_room_document_list(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomDocListReq {
        room_id: String,
    }
    match read_body::<RoomDocListReq>(req.as_reader()) {
        Ok(req_data) => match uteke.room_list_documents(&req_data.room_id) {
            Ok(slugs) => ctx.ok_response_for(
                req,
                &serde_json::json!({ "room_id": req_data.room_id, "doc_slugs": slugs }),
            ),
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// PUT /room/document/add — link a document to a room
pub(super) fn put_room_document_add(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomDocAddReq {
        room_id: String,
        doc_slug: String,
    }
    match read_body::<RoomDocAddReq>(req.as_reader()) {
        Ok(req_data) => match uteke.room_add_document(&req_data.room_id, &req_data.doc_slug) {
            Ok(()) => ctx.ok_response_for(req, &serde_json::json!({ "status": "linked", "room_id": req_data.room_id, "doc_slug": req_data.doc_slug })),
            Err(e) => match e {
                uteke_core::Error::Validation(_) => {
                    ctx.error_response_for(req, 400, e.to_string())
                }
                _ => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            },
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// DELETE /room/document/remove — unlink a document from a room
pub(super) fn delete_room_document_remove(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct RoomDocRemoveReq {
        room_id: String,
        doc_slug: String,
    }
    match read_body::<RoomDocRemoveReq>(req.as_reader()) {
        Ok(req_data) => match uteke.room_remove_document(&req_data.room_id, &req_data.doc_slug) {
            Ok(()) => ctx.ok_response_for(req, &serde_json::json!({ "status": "unlinked", "room_id": req_data.room_id, "doc_slug": req_data.doc_slug })),
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// POST /doc/room/list — list rooms linked to a document
pub(super) fn post_doc_room_list(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct DocRoomListReq {
        doc_slug: String,
    }
    match read_body::<DocRoomListReq>(req.as_reader()) {
        Ok(req_data) => match uteke.document_list_rooms(&req_data.doc_slug) {
            Ok(room_ids) => ctx.ok_response_for(
                req,
                &serde_json::json!({ "doc_slug": req_data.doc_slug, "room_ids": room_ids }),
            ),
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn delete_room_delete(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let room_id = if let Some(q) = path.split('?').nth(1) {
        parse_query_param(q, "room_id")
    } else {
        // Try reading from query params in headers or body
        None
    };
    let room_id = match room_id {
        Some(id) => id,
        None => {
            // Try body as JSON
            #[derive(Deserialize)]
            struct RoomDeleteRequest {
                room_id: String,
            }
            match read_body::<RoomDeleteRequest>(req.as_reader()) {
                Ok(data) => data.room_id,
                Err(_) => {
                    return ctx.error_response_for(req, 400, "Missing 'room_id' parameter");
                }
            }
        }
    };
    match uteke.delete_room(&room_id) {
        Ok(unlinked) => ctx.ok_response_for(
            req,
            &serde_json::json!({
                "deleted": room_id,
                "unlinked_memories": unlinked,
                "note": "memories and documents are preserved in their namespaces, no longer linked to any room"
            }),
        ),
        Err(e) => {
            let msg = format!("{e}");
            if msg.contains("not found") {
                ctx.error_response_for(req, 404, &msg)
            } else {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        }
    }
}
