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
            let tag_refs = req_data.tag_refs();
            let metadata = req_data.metadata.clone();
            // #1464: the source is set after the memory is stored, so strict
            // mode has to refuse it before anything is written.
            if let Some(src) = req_data.source.as_deref()
                && let Err(uteke_core::Error::SecretDetected { kinds }) =
                    uteke.privacy_config().mask_text(src)
            {
                return ctx.secret_detected_response(req, &kinds);
            }

            let result: Result<uteke_core::RememberOutcome, uteke_core::Error> =
                if req_data.detect_contradiction {
                    uteke
                        .remember_with_contradiction(
                            &req_data.content,
                            &tag_refs,
                            metadata,
                            ns(&req_data.namespace),
                            req_data.memory_type.as_deref(),
                            true,
                            0.65,
                        )
                        .map(|(outcome, _)| outcome)
                } else if let Some(t) = req_data.memory_type.as_deref() {
                    uteke.remember_typed_detailed(
                        &req_data.content,
                        &tag_refs,
                        metadata,
                        ns(&req_data.namespace),
                        t,
                    )
                } else {
                    uteke.remember_detailed(
                        &req_data.content,
                        &tag_refs,
                        metadata,
                        ns(&req_data.namespace),
                    )
                };

            match result {
                Ok(outcome) => {
                    let redacted = outcome.redacted();
                    let id = outcome.id;
                    // Set source provenance after storage (#682) — matches CLI.
                    if req_data.source.is_some() || req_data.source_type.is_some() {
                        let st = req_data.source_type.as_deref().unwrap_or("user");
                        if let Err(e) = uteke.set_source(&id, req_data.source.as_deref(), st) {
                            error!("Failed to set source for {id}: {e}");
                        }
                    }
                    // Set author type after storage (#1083). Validate before
                    // writing so invalid values fail loudly (400, not silent).
                    if let Some(at) = req_data.author_type.as_deref() {
                        if let Err(e) = uteke.set_author_type(&id, at) {
                            return ctx.error_response_for(req, 400, e.to_string());
                        }
                    }
                    // Set the memory's own author after storage (#1427 phase 2).
                    // Already validated and trimmed at decode.
                    if let Some(a) = req_data.author.as_deref() {
                        if let Err(e) = uteke.set_author(&id, Some(a)) {
                            error!("Failed to set author for {id}: {e}");
                        }
                    }
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
            // request shared with MCP and the CLI (#1343).
            let typed = match req_data.decode() {
                Ok(t) => t,
                Err(e) => return ctx.error_response_for(req, 400, e.to_string()),
            };
            let limit = typed.limit;
            let tag_vec = typed.tag_filter();
            let tags_filter = tag_vec.as_deref();
            // Threshold: min_score > [recall] min_score(_strict) > built-in
            // (strict 0.5, otherwise 0.0).
            let min_score = typed.resolve_min_score(
                ctx.recall_config.as_ref().and_then(|r| r.min_score),
                ctx.recall_config.as_ref().and_then(|r| r.min_score_strict),
            );
            // Entity/category filters are now pushed into the core recall
            // candidate loop (#663) — no 10x fetch amplification needed.

            // Time-travel mode: recall_at_time
            let point_in_time = typed.at;

            let entity_filter = req_data.entity.as_deref();
            let category_filter = req_data.category.as_deref();

            // #902: temporal range filters (before/after).
            let after_ts = typed.after;
            let before_ts = typed.before;
            let has_temporal = after_ts.is_some() || before_ts.is_some();

            // Resolve recall strategy ONCE for every path (#1034):
            // request `strategy` > config `[recall] default_strategy` >
            // Hybrid — matching the CLI default. Unknown values are a
            // loud 400 on all paths, never a silent no-op.
            let strategy = match typed.strategy(
                ctx.recall_config
                    .as_ref()
                    .and_then(|r| r.default_strategy.as_deref()),
            ) {
                Ok(s) => s,
                Err(e) => return ctx.error_response_for(req, 400, e.to_string()),
            };

            // Explain mode (#1160): memory-only recall with per-result
            // ranking signals. Mutually exclusive with the unified /
            // time-travel / temporal surfaces.
            if req_data.explain {
                if req_data.search_type.is_some() {
                    return ctx.error_response_for(
                        req,
                        400,
                        "explain is memory-only: omit search_type",
                    );
                }
                if point_in_time.is_some() || has_temporal {
                    return ctx.error_response_for(
                        req,
                        400,
                        "explain cannot be combined with at/before/after",
                    );
                }
                if entity_filter.is_some() || category_filter.is_some() || req_data.enrich {
                    return ctx.error_response_for(
                        req,
                        400,
                        "explain cannot be combined with entity/category/enrich",
                    );
                }
                return match uteke.recall_explained(
                    &req_data.query,
                    limit,
                    tags_filter,
                    ns(&req_data.namespace),
                    strategy,
                    min_score,
                ) {
                    Ok(explained) => ctx.ok_response_for(req, &explained),
                    Err(e) => {
                        error!("Explain recall error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                };
            }

            // Budgeted context pack (#1281 Phase 1): unified recall +
            // greedy character-budget fill; deterministic, LLM-free,
            // rank-order preserving. Honours the caller's search_type
            // (validated with the same 400 as the plain path).
            if req_data.pack {
                // Loud reject like the CLI: pack is not composable with
                // point-in-time / range filters (the plain unified path
                // consumes them further down, after this early return).
                if req_data.at.is_some() || req_data.after.is_some() || req_data.before.is_some() {
                    return ctx.error_response_for(
                        req,
                        400,
                        "pack is not supported with at/after/before filters; rerun without time filters",
                    );
                }
                let parsed_search_type = match typed.search_type() {
                    Ok(t) => t,
                    Err(e) => return ctx.error_response_for(req, 400, e.to_string()),
                };
                let exclude_ids = typed.exclude_ids.clone();
                return match uteke.recall_unified_packed(
                    &req_data.query,
                    limit,
                    tags_filter,
                    ns(&req_data.namespace),
                    min_score,
                    parsed_search_type,
                    entity_filter,
                    category_filter,
                    req_data.enrich,
                    strategy,
                    typed.budget_chars_or_default(),
                    &exclude_ids,
                ) {
                    Ok(pack) => ctx.ok_response_for(req, &pack),
                    Err(e) => {
                        error!("Pack recall error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                };
            }

            // Unified search path (#531): when search_type is specified,
            // use recall_unified. Entity/category filters are passed
            // through to the core recall candidate loop (#663).
            let unified_result = if req_data.search_type.is_some() && point_in_time.is_none() {
                let search_type = match typed.search_type() {
                    Ok(t) => t,
                    Err(e) => return ctx.error_response_for(req, 400, e.to_string()),
                };
                Some(uteke.recall_unified(
                    &req_data.query,
                    limit,
                    tags_filter,
                    ns(&req_data.namespace),
                    min_score,
                    search_type,
                    entity_filter,
                    category_filter,
                    req_data.enrich,
                    strategy,
                ))
            } else {
                None
            };

            // Prefer unified results when available (#531)
            match unified_result {
                Some(Ok(mut results)) => {
                    // #902: Post-filter by temporal range (before/after).
                    if has_temporal {
                        results.retain(|r| {
                            let ts = r.created_at.as_ref();
                            ts.is_some_and(|t| {
                                after_ts.is_none_or(|a| t >= &a)
                                    && before_ts.is_none_or(|b| t <= &b)
                            })
                        });
                    }
                    if api_version == Some(ApiVersion::V1) {
                        // v1: flat format [{id, content, score, ...}]
                        let v1_results: Vec<serde_json::Value> =
                            results.iter().map(to_v1_flat).collect();
                        ctx.ok_response_for(req, &v1_results)
                    } else {
                        ctx.ok_response_for(req, &results)
                    }
                }
                Some(Err(e)) => {
                    error!("Unified search error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
                None => {
                    // Memory-only recall path (#1034): route through
                    // recall_hybrid so the resolved strategy applies to
                    // the default path too — not just when search_type is
                    // present. recall_hybrid has no native entity/category
                    // support, so apply them as post-filters with 3×
                    // over-fetch (same pattern as recall_unified_memories).
                    let needs_post_filter = entity_filter.is_some() || category_filter.is_some();
                    let fetch_limit = if needs_post_filter {
                        (limit.saturating_mul(3)).max(limit + 10)
                    } else {
                        limit
                    };
                    let recall_result = if let Some(pit) = point_in_time {
                        // Time-travel: strategy doesn't apply to
                        // recall_at_time (point-in-time semantics).
                        uteke.recall_at_time(
                            &req_data.query,
                            limit,
                            tags_filter,
                            ns(&req_data.namespace),
                            pit,
                            min_score,
                            entity_filter,
                            category_filter,
                        )
                    } else {
                        uteke.recall_hybrid(
                            &req_data.query,
                            fetch_limit,
                            tags_filter,
                            ns(&req_data.namespace),
                            strategy,
                            min_score,
                        )
                    };
                    // Entity/category post-filter (memory results only) —
                    // mirrors the CLI recall post-filter and core
                    // recall_unified_memories (#663, #900).
                    match recall_result {
                        Ok(mut results) => {
                            if needs_post_filter && point_in_time.is_none() {
                                results.retain(|sr| {
                                    entity_filter.is_none_or(|ent| {
                                        sr.memory
                                            .metadata
                                            .get("entity")
                                            .and_then(|v| v.as_str())
                                            .is_some_and(|e| e == ent)
                                    }) && category_filter.is_none_or(|cat| {
                                        sr.memory
                                            .metadata
                                            .get("category")
                                            .and_then(|v| v.as_str())
                                            .is_some_and(|c| c == cat)
                                    })
                                });
                                results.truncate(limit);
                            }
                            // #902: Post-filter by temporal range (before/after).
                            if has_temporal {
                                results.retain(|r| {
                                    let t = &r.memory.created_at;
                                    after_ts.is_none_or(|a| t >= &a)
                                        && before_ts.is_none_or(|b| t <= &b)
                                });
                            }
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
                        Err(e) => {
                            error!("Internal error: {e}");
                            ctx.error_response_for(req, 500, "Internal server error")
                        }
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
            // Time-travel mode: use list_at_time
            let list_result = match list_req.at {
                Some(pit) => uteke.list_at_time(
                    list_req.tag.as_deref(),
                    list_req.limit,
                    list_req.offset,
                    ns(&list_req.namespace),
                    pit,
                ),
                None => uteke.list(
                    list_req.tag.as_deref(),
                    list_req.limit,
                    list_req.offset,
                    ns(&list_req.namespace),
                ),
            };
            match list_result {
                Ok(memories) => {
                    // #1188: opt-in pagination envelope. The default
                    // response stays a bare array for existing clients.
                    if list_req.wants_envelope() {
                        // Total matching rows (same filters as the page).
                        let total = match &list_req.tag {
                            Some(tag) => uteke.count_by_tag(tag, ns(&list_req.namespace)),
                            None => uteke.count(ns(&list_req.namespace)),
                        }
                        .unwrap_or(0);
                        let meta = list_req.page_meta(total, memories.len());
                        return ctx.ok_response_for(
                            req,
                            &serde_json::json!({
                                "memories": memories,
                                "total": total,
                                "has_more": meta.has_more,
                                "next_offset": meta.next_offset,
                            }),
                        );
                    }
                    ctx.ok_response_for(req, &memories)
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

pub(super) fn delete_forget(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query = path.split('?').nth(1).unwrap_or("");
    // Typed request shared with MCP and the CLI (#1343).
    let request = match uteke_core::ForgetRequest::decode(
        uteke_core::ForgetInput::from_http_query(query),
        uteke_core::ForgetPolicy::HTTP,
    ) {
        Ok(r) => r,
        Err(e) => return ctx.error_response_for(req, 400, e.message()),
    };
    match request {
        uteke_core::ForgetRequest::Id(_) => {
            // Accept full UUID or short ID prefix (#794); the
            // existence check (#762) is part of the HTTP policy
            // because forget() silently returns Ok(()) for unknown ids.
            let resolved_id = match request.resolve(uteke, uteke_core::ForgetPolicy::HTTP) {
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
                            let status = match e {
                                uteke_core::Error::Validation(_) => 400,
                                _ => 500,
                            };
                            return ctx.error_response_for(req, status, e.to_string());
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
                Err(uteke_core::Error::SecretDetected { kinds }) => {
                    ctx.secret_detected_response(req, &kinds)
                }
                Err(e) => {
                    error!("Update memory error: {e}");
                    // Propagate validation errors with proper status codes
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
