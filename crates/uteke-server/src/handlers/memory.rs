//! Memory handlers: remember, recall, search, list, forget, stats, recent, single-memory reads/updates, pin and feedback.

use super::*;

pub(super) fn post_remember(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<RememberRequest>(req.as_reader()) {
        Ok(raw) => {
            // Validation (content/tags, author_type, type) and the
            // metadata map live in the typed request shared with MCP and
            // the CLI (#1343). Order: content/tags, author_type, type; all
            // before any write (#1083, #1302).
            let req_data = match raw.decode() {
                Ok(t) => t,
                Err(e) => return ctx.error_response_for(req, 400, e.message().to_string()),
            };
            // The write, the source pre-check for strict privacy mode (#1464)
            // and the provenance follow-ups (source, author_type, author) run
            // in the core, with one failure policy (#1516).
            match uteke.run_remember(&req_data) {
                Ok(run) => {
                    let outcome = run.outcome;
                    let redacted = outcome.redacted();
                    let id = outcome.id;
                    // Echo author_type so clients can confirm what was
                    // recorded (#1106) — data was already stored correctly,
                    // the response just omitted the field. Default mirrors
                    // the schema default (author_type DEFAULT 'agent', #1083).
                    // #1273: honest embedding status — never a bare success
                    // when the vector write failed.
                    let mut body = serde_json::json!({
                        "id": id,
                        "author_type": req_data.author_type.clone().unwrap_or_else(|| "agent".to_string()),
                        "embedding_written": outcome.embedding_written,
                        // #1464: true when the privacy filter masked a secret,
                        // so the stored text differs from the request.
                        "redacted": redacted,
                        "warning": uteke_core::namespace_deprecation::merge_warning(
                            outcome.warning,
                            uteke_core::namespace_deprecation::namespace_write_warning(
                                ns(&req_data.namespace),
                            ),
                        ),
                    });
                    // Echo the author when one was recorded (#1427 phase 2).
                    if let Some(a) = &req_data.author {
                        body["author"] = serde_json::json!(a);
                    }
                    ctx.ok_response_for(req, &body)
                }
                Err(uteke_core::Error::SecretDetected { kinds }) => {
                    ctx.secret_detected_response(req, &kinds)
                }
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_recall(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let api_version = c.api_version;
    match read_body::<RecallRequest>(req.as_reader()) {
        Ok(req_data) => {
            // Defaults, the limit cap (#903), the blank-query rejection (#907)
            // and the RFC3339 parsing of at/after/before live in the typed
            // request shared with MCP and the CLI (#1343); the decision tree
            // that runs it (explain, pack, unified, time travel, the
            // memory-only path) is shared with MCP in the core (#1439).
            let typed = match req_data.decode() {
                Ok(t) => t,
                Err(e) => return ctx.error_response_for(req, 400, e.to_string()),
            };
            // Threshold: min_score > [recall] min_score(_strict) > built-in
            // (strict 0.5, otherwise 0.0); strategy: request > [recall]
            // default_strategy > fusion.
            let defaults = uteke_core::recall_exec::RecallDefaults {
                min_score: ctx.recall_config.as_ref().map(|r| r.min_score),
                min_score_strict: ctx.recall_config.as_ref().map(|r| r.min_score_strict),
                default_strategy: ctx
                    .recall_config
                    .as_ref()
                    .map(|r| r.default_strategy.as_str()),
            };
            match uteke.run_recall(&typed, defaults) {
                // A guard or an unknown strategy/search_type: 400 with the bare text.
                Err(uteke_core::Error::Validation(msg)) => ctx.error_response_for(req, 400, msg),
                Err(e) => {
                    error!("Recall error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
                Ok(uteke_core::recall_exec::RecallOutput::Explained(explained)) => {
                    ctx.ok_response_for(req, &explained)
                }
                Ok(uteke_core::recall_exec::RecallOutput::Packed(pack)) => {
                    ctx.ok_response_for(req, &pack)
                }
                Ok(uteke_core::recall_exec::RecallOutput::Unified(results)) => {
                    if api_version == Some(ApiVersion::V1) {
                        // v1: flat format [{id, content, score, ...}]
                        let v1_results: Vec<serde_json::Value> =
                            results.iter().map(to_v1_flat).collect();
                        ctx.ok_response_for(req, &v1_results)
                    } else {
                        ctx.ok_response_for(req, &results)
                    }
                }
                Ok(uteke_core::recall_exec::RecallOutput::Memories { results, min_score }) => {
                    if results.is_empty() && min_score > 0.0 {
                        ctx.ok_response_for(
                            req,
                            &serde_json::json!({
                                "results": [],
                                "total": 0,
                                "threshold": min_score,
                                "message": "No memories above similarity threshold"
                            }),
                        )
                    } else {
                        ctx.ok_response_for(req, &results)
                    }
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_search(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<SearchRequest>(req.as_reader()) {
        Ok(req_data) => {
            let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
            let tags_filter = if tag_refs.is_empty() {
                None
            } else {
                Some(tag_refs.as_slice())
            };
            match uteke.search(
                &req_data.query,
                req_data.limit,
                tags_filter,
                ns(&req_data.namespace),
            ) {
                Ok(results) => ctx.ok_response_for(req, &results),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_list(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<ListParams>(req.as_reader()) {
        Ok(params) => {
            // #1343: limit cap and `at` parsing live in the typed request.
            let list_req = match params.decode() {
                Ok(r) => r,
                Err(e) => return ctx.error_response_for(req, 400, e.to_string()),
            };
            // The choice between a plain page, a point-in-time page and the
            // pagination envelope (#1188) is shared with MCP (#1439).
            match uteke.run_list(&list_req) {
                Ok(uteke_core::list_exec::ListOutput::Memories(memories)) => {
                    ctx.ok_response_for(req, &memories)
                }
                Ok(uteke_core::list_exec::ListOutput::Page {
                    memories,
                    total,
                    meta,
                }) => ctx.ok_response_for(
                    req,
                    &serde_json::json!({
                        "memories": memories,
                        "total": total,
                        "has_more": meta.has_more,
                        "next_offset": meta.next_offset,
                    }),
                ),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn delete_forget(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query = path.split('?').nth(1).unwrap_or("");
    // Typed request shared with MCP and the CLI (#1343).
    let request =
        match uteke_core::ForgetRequest::decode(uteke_core::ForgetInput::from_http_query(query)) {
            Ok(r) => r,
            Err(e) => return ctx.error_response_for(req, 400, e.message()),
        };
    match request {
        uteke_core::ForgetRequest::Id(_) => {
            // Accept full UUID or short ID prefix (#794); the
            // existence check (#762) is part of the HTTP policy
            // because forget() silently returns Ok(()) for unknown ids.
            let resolved_id = match request.resolve(uteke) {
                Ok(Some(id)) => id,
                Ok(None) => unreachable!("an id request resolves to an id"),
                Err(e) => {
                    return match e.kind() {
                        uteke_core::ForgetErrorKind::NotFound => {
                            ctx.error_response_for(req, 404, e.message())
                        }
                        _ => {
                            error!("Resolve error: {e}");
                            ctx.error_response_for(req, 400, e.message())
                        }
                    };
                }
            };
            match uteke.forget(&resolved_id) {
                Ok(()) => ctx.ok_response_for(req, &serde_json::json!({"forgotten": resolved_id})),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        uteke_core::ForgetRequest::Tag { tag, namespace } => {
            match uteke.bulk_forget_by_tag(&tag, namespace.as_deref()) {
                Ok(result) => ctx.ok_response_for(req, &result),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
    }
}

pub(super) fn get_stats(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    // Parse ?namespace= query parameter for scoped stats (#382).
    let query = req.url().split('?').nth(1).unwrap_or("");
    let params: std::collections::HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            Some((kv.next()?.to_string(), kv.next()?.to_string()))
        })
        .collect();
    let ns_param = params.get("namespace").map(|s| s.as_str());
    match uteke.stats(ns_param) {
        Ok(stats) => ctx.ok_response_for(req, &stats),
        Err(e) => {
            error!("Internal error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_stats(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct StatsReq {
        namespace: Option<String>,
    }
    match read_body::<StatsReq>(req.as_reader()) {
        Ok(req_data) => match uteke.stats(ns(&req_data.namespace)) {
            Ok(stats) => ctx.ok_response_for(req, &stats),
            Err(e) => {
                error!("Internal error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn get_recent(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let ns = parse_query_namespace(path);
    let query = path.split('?').nth(1).unwrap_or("");
    let limit = parse_query_param(query, "limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(20)
        .min(MAX_LIST_LIMIT);
    let offset = parse_query_param(query, "offset")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    match uteke.list(None, limit, offset, ns.as_deref()) {
        Ok(memories) => ctx.ok_response_for(req, &memories),
        Err(e) => {
            error!("Internal error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn get_memory(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query = path.split('?').nth(1).unwrap_or("");
    let id = parse_query_param(query, "id").unwrap_or_default();
    // Validate UUID format
    if uuid::Uuid::parse_str(&id).is_err() {
        return ctx.error_response_for(req, 400, format!("Invalid UUID format: {id}"));
    }
    match uteke.get_by_id(&id) {
        Ok(Some(memory)) => ctx.ok_response_for(req, &memory),
        Ok(None) => ctx.error_response_for(req, 404, format!("Memory not found: {id}")),
        Err(e) => {
            error!("Internal error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_memory_pin(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<MemoryPinRequest>(req.as_reader()) {
        Ok(req_data) => {
            if uuid::Uuid::parse_str(&req_data.id).is_err() {
                return ctx.error_response_for(
                    req,
                    400,
                    format!("Invalid UUID format: {}", req_data.id),
                );
            }
            let result = if req_data.pinned {
                uteke.pin(&req_data.id)
            } else {
                uteke.unpin(&req_data.id)
            };
            match result {
                Ok(true) => match uteke.get_by_id(&req_data.id) {
                    Ok(Some(memory)) => ctx.ok_response_for(req, &memory),
                    Ok(None) => ctx.error_response_for(
                        req,
                        500,
                        "Memory updated but could not be retrieved",
                    ),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Ok(false) => {
                    ctx.error_response_for(req, 404, format!("Memory not found: {}", req_data.id))
                }
                Err(e) => {
                    error!("Pin/unpin error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_memory_importance(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<MemoryImportanceRequest>(req.as_reader()) {
        Ok(req_data) => {
            if uuid::Uuid::parse_str(&req_data.id).is_err() {
                return ctx.error_response_for(
                    req,
                    400,
                    format!("Invalid UUID format: {}", req_data.id),
                );
            }
            match uteke.set_importance(&req_data.id, req_data.importance) {
                Ok(true) => match uteke.get_by_id(&req_data.id) {
                    Ok(Some(memory)) => ctx.ok_response_for(req, &memory),
                    Ok(None) => ctx.error_response_for(
                        req,
                        500,
                        "Memory updated but could not be retrieved",
                    ),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Ok(false) => {
                    ctx.error_response_for(req, 404, format!("Memory not found: {}", req_data.id))
                }
                Err(e) => match e {
                    uteke_core::Error::Validation(_) => {
                        ctx.error_response_for(req, 400, e.to_string())
                    }
                    _ => {
                        error!("Set importance error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_memory_feedback(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<MemoryFeedbackRequest>(req.as_reader()) {
        Ok(req_data) => {
            if uuid::Uuid::parse_str(&req_data.id).is_err() {
                return ctx.error_response_for(
                    req,
                    400,
                    format!("Invalid UUID format: {}", req_data.id),
                );
            }
            let result = match req_data.feedback.as_str() {
                "helpful" => uteke.feedback_helpful(&req_data.id).map(|imp| {
                    serde_json::json!({
                        "id": req_data.id,
                        "feedback": "helpful",
                        "delta": 0.05,
                        "importance": imp,
                    })
                }),
                "unhelpful" => uteke.feedback_unhelpful(&req_data.id).map(|imp| {
                    serde_json::json!({
                        "id": req_data.id,
                        "feedback": "unhelpful",
                        "delta": -0.10,
                        "importance": imp,
                    })
                }),
                _ => {
                    return ctx.error_response_for(
                        req,
                        400,
                        "Invalid feedback value. Use 'helpful' or 'unhelpful'.".to_string(),
                    );
                }
            };
            match result {
                Ok(data) => ctx.ok_response_for(req, &data),
                Err(e) => {
                    error!("Feedback error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn put_memory(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<MemoryUpdateRequest>(req.as_reader()) {
        Ok(req_data) => {
            // Validate UUID format
            if uuid::Uuid::parse_str(&req_data.id).is_err() {
                return ctx.error_response_for(
                    req,
                    400,
                    format!("Invalid UUID format: {}", req_data.id),
                );
            }
            // Check that at least one field is provided
            if req_data.content.is_none()
                && req_data.tags.is_none()
                && req_data.metadata.is_none()
                && req_data.importance.is_none()
                && req_data.pinned.is_none()
                && req_data.memory_type.is_none()
                && req_data.namespace.is_none()
            {
                return ctx.error_response_for(
                    req,
                    400,
                    "No fields to update. Provide at least one of: content, tags, metadata, importance, pinned, memory_type, namespace",
                );
            }
            let tag_refs: Option<Vec<String>> = req_data.tags;
            let tag_slice: Option<&[String]> = tag_refs.as_deref();
            match uteke.update_memory_detailed(
                &req_data.id,
                req_data.content.as_deref(),
                tag_slice,
                req_data.metadata.as_ref(),
                req_data.importance,
                req_data.pinned,
                req_data.memory_type.as_deref(),
            ) {
                Ok(outcome) if outcome.updated => {
                    // Namespace move (#1181): separate op after the field
                    // update — plain column change, no re-embed needed.
                    if let Some(ns) = req_data.namespace.as_deref() {
                        if let Err(e) = uteke.move_memory(&req_data.id, ns) {
                            return ctx.core_error(req, &e);
                        }
                    }
                    // #1464: true when the filter masked a secret in the new
                    // content, tags or metadata.
                    ctx.ok_response_for(
                        req,
                        &with_namespace_warning(
                            serde_json::json!({
                                "updated": req_data.id,
                                "redacted": outcome.redacted(),
                            }),
                            req_data.namespace.as_deref(),
                        ),
                    )
                }
                Ok(_) => {
                    ctx.error_response_for(req, 404, format!("Memory not found: {}", req_data.id))
                }
                Err(e) => ctx.core_error(req, &e),
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// POST /memory/doc-refs — get document slugs referenced by a memory
pub(super) fn post_memory_doc_refs(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct MemoryDocRefsReq {
        memory_id: String,
    }
    match read_body::<MemoryDocRefsReq>(req.as_reader()) {
        Ok(req_data) => match uteke.recall_documents_for_memory(&req_data.memory_id) {
            Ok(slugs) => ctx.ok_response_for(
                req,
                &serde_json::json!({
                    "memory_id": req_data.memory_id,
                    "doc_slugs": slugs,
                }),
            ),
            Err(e) => {
                error!("memory/doc-refs error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_pin(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<PinRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.pin(&req_data.id) {
            Ok(true) => ctx.ok_response_for(req, &serde_json::json!({"pinned": req_data.id})),
            Ok(false) => {
                ctx.error_response_for(req, 404, format!("Memory not found: {}", req_data.id))
            }
            Err(e) => {
                error!("Pin error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_unpin(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<PinRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.unpin(&req_data.id) {
            Ok(true) => ctx.ok_response_for(req, &serde_json::json!({"unpinned": req_data.id})),
            Ok(false) => {
                ctx.error_response_for(req, 404, format!("Memory not found: {}", req_data.id))
            }
            Err(e) => {
                error!("Unpin error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}
