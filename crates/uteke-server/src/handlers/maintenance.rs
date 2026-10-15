//! Maintenance and lifecycle handlers: verify, repair, prune, consolidate, aging, importance, orphans, extract, import/export, dream, context, lifecycle and backlink rebuilds.

use super::*;

pub(super) fn get_lifecycle_status(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let query = req.url().split('?').nth(1).unwrap_or("");
    let params: std::collections::HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            Some((kv.next()?.to_string(), kv.next()?.to_string()))
        })
        .collect();
    let ns_param = params.get("namespace").map(|s| s.as_str());
    let active = match uteke.store().count_active(ns_param) {
        Ok(v) => v,
        Err(e) => {
            error!("lifecycle status error: {e}");
            return ctx.error_response_for(req, 500, "Internal server error");
        }
    };
    let deprecated = match uteke.store().count_deprecated(ns_param) {
        Ok(v) => v,
        Err(e) => {
            error!("lifecycle status error: {e}");
            return ctx.error_response_for(req, 500, "Internal server error");
        }
    };
    #[derive(serde::Serialize)]
    struct LifecycleStatusResponse {
        active: usize,
        deprecated: usize,
    }
    ctx.ok_response_for(req, &LifecycleStatusResponse { active, deprecated })
}

pub(super) fn get_lifecycle_deprecated(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let query = req.url().split('?').nth(1).unwrap_or("");
    let params: std::collections::HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            Some((kv.next()?.to_string(), kv.next()?.to_string()))
        })
        .collect();
    let ns_param = params.get("namespace").map(|s| s.as_str());
    let limit: u32 = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100)
        .min(MAX_LIST_LIMIT as u32);
    match uteke.store().list_deprecated(ns_param, limit) {
        Ok(items) => {
            #[derive(serde::Serialize)]
            struct DeprecatedListResponse {
                deprecated: Vec<uteke_core::DeprecatedMemoryInfo>,
                count: usize,
            }
            let count = items.len();
            ctx.ok_response_for(
                req,
                &DeprecatedListResponse {
                    deprecated: items,
                    count,
                },
            )
        }
        Err(e) => {
            error!("lifecycle deprecated list error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_lifecycle_cycle(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct CycleReq {
        namespace: Option<String>,
    }
    match read_body::<CycleReq>(req.as_reader()) {
        Ok(req_data) => match uteke.lifecycle_cycle(ns(&req_data.namespace)) {
            Ok(result) => ctx.ok_response_for(req, &result),
            Err(e) => {
                error!("lifecycle cycle error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_lifecycle_promote(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct PromoteReq {
        id: String,
    }
    match read_body::<PromoteReq>(req.as_reader()) {
        Ok(req_data) => match uteke.promote(&req_data.id) {
            Ok(restored) => {
                #[derive(serde::Serialize)]
                struct PromoteResponse {
                    promoted: bool,
                    id: String,
                }
                ctx.ok_response_for(
                    req,
                    &PromoteResponse {
                        promoted: restored,
                        id: req_data.id,
                    },
                )
            }
            Err(e) => {
                error!("lifecycle promote error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_context(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<serde_json::Value>(req.as_reader()) {
        Ok(body) => {
            let ns = body.get("namespace").and_then(|v| v.as_str());
            match uteke.build_context(ns) {
                Ok(context) => {
                    let resp = serde_json::json!({ "context": context });
                    ctx.ok_response_for(req, &resp)
                }
                Err(e) => {
                    error!("Context error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(_) => ctx.error_response_for(req, 400, "Invalid JSON body"),
    }
}

pub(super) fn post_dream(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<serde_json::Value>(req.as_reader()) {
        Ok(body) => {
            let ns = body.get("namespace").and_then(|v| v.as_str());
            let dry_run = body
                .get("dry_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match uteke.dream(ns, dry_run, &[]) {
                Ok(report) => ctx.ok_response_for(req, &report),
                Err(e) => {
                    error!("Dream error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(_) => ctx.error_response_for(req, 400, "Invalid JSON body"),
    }
}

pub(super) fn post_extract(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<ExtractRequest>(req.as_reader()) {
        Ok(req_data) => {
            if validate_content_size(&req_data.content, 1_048_576).is_err() {
                return ctx.error_response_for(req, 413, "Content too large (max 1MB)");
            }
            if let Err(e) = uteke_core::validate_input(&req_data.content, &req_data.tags) {
                return ctx.error_response_for(req, 400, e.to_string());
            }

            let ext_config = resolve_extraction_config(
                ctx,
                req_data.model.as_deref(),
                req_data.max_facts,
                None, // api_key from config only (not from request body)
            );

            let extractor = match uteke_core::extraction::Extractor::new(&ext_config) {
                Ok(e) => e,
                Err(e) => {
                    error!("Extractor init error: {e}");
                    return ctx.error_response_for(req, 400, e.to_string());
                }
            };

            let facts = match extractor.extract(&req_data.content) {
                Ok(f) => f,
                Err(e) => {
                    error!("Extraction error: {e}");
                    return ctx.error_response_for(req, 502, format!("Extraction failed: {e}"));
                }
            };

            // Store each extracted fact as a memory
            let mut stored_ids = Vec::new();
            let fact_ns = ns(&req_data.namespace);

            for fact in &facts {
                // Build tags: caller-provided tags + scene tag if present (#1009).
                let mut all_tags: Vec<String> =
                    req_data.tags.iter().map(|s| s.to_string()).collect();
                if let Some(ref scene) = fact.scene {
                    let scene_tag = format!("scene:{}", scene);
                    if !all_tags.contains(&scene_tag) {
                        all_tags.push(scene_tag);
                    }
                }
                let tag_refs: Vec<&str> = all_tags.iter().map(|s| s.as_str()).collect();

                let mut meta = serde_json::Map::new();
                meta.insert(
                    "source".into(),
                    serde_json::Value::String("extraction".into()),
                );
                let metadata = if meta.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(meta))
                };

                // Use remember_typed when the LLM or request provided a type (#1009).
                let effective_type = fact.fact_type.as_deref().or(req_data.r#type.as_deref());
                let result = if let Some(ft) = effective_type {
                    uteke.remember_typed(&fact.content, &tag_refs, metadata, fact_ns, ft)
                } else {
                    uteke.remember(&fact.content, &tag_refs, metadata, fact_ns)
                };

                if let Ok(id) = result {
                    // Set importance if the LLM provided a priority score (#1009).
                    if let Some(priority) = fact.priority {
                        let _ = uteke.set_importance(&id, priority);
                    }
                    // Auto-populate source provenance (#1013).
                    let _ = uteke.set_source(&id, Some("api:extraction"), "extract");
                    stored_ids.push(id);
                }
            }

            ctx.ok_response_for(
                req,
                &with_namespace_warning(
                    serde_json::json!({
                        "facts": facts.iter().map(|f| &f.content).collect::<Vec<_>>(),
                        "count": facts.len(),
                        "stored": stored_ids.len(),
                        "stored_ids": stored_ids,
                    }),
                    fact_ns,
                ),
            )
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_import(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<ImportRequest>(req.as_reader()) {
        Ok(req_data) => {
            // Same ceiling `read_body` enforces on the whole request, so the
            // limit the caller is told about is the limit that applies (#1328).
            let max_import = uteke_core::MAX_PAYLOAD_SIZE;
            if validate_content_size(&req_data.content, max_import).is_err() {
                return ctx.error_response_for(
                    req,
                    413,
                    format!("Content too large (max {}MB)", max_import / 1_048_576),
                );
            }

            // Merge request tags into the JSONL entries.
            // The core import method handles re-embedding.
            let import_ns = ns(&req_data.namespace);
            match uteke.import(&req_data.content, import_ns) {
                Ok(result) => ctx.ok_response_for(
                    req,
                    &with_namespace_warning(
                        serde_json::json!({
                            "imported": result.imported,
                            "skipped": result.skipped,
                        }),
                        import_ns,
                    ),
                ),
                Err(e) => {
                    error!("Import error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn get_export(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let export_ns = parse_query_namespace(path);
    match uteke.export(export_ns.as_deref()) {
        Ok(jsonl) => {
            let mut headers = ctx.cors_headers_for(req);
            headers.push(
                Header::from_bytes(&b"Content-Type"[..], &b"application/x-ndjson"[..]).unwrap(),
            );
            Response::new(
                StatusCode::from(200),
                headers,
                Cursor::new(jsonl.into_bytes()),
                None,
                None,
            )
        }
        Err(e) => {
            error!("Export error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

// POST /verify → VerifyReport (read-only check, write token like
// other maintenance endpoints). POST /repair → rebuilds the vector
// index from SQLite embeddings so stores can recover from desync
// (e.g. version-lagged serve binaries, #1245 class) WITHOUT a
// restart — the previous runbook required stopping uteke-serve
// (pitfall 0f lock contention).
pub(super) fn post_verify(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match uteke.verify() {
        Ok(r) => ctx.ok_response_for(req, &r),
        Err(e) => {
            error!("Request failed: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_repair(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match uteke.repair() {
        Ok(r) => ctx.ok_response_for(req, &r),
        Err(e) => {
            error!("Request failed: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_prune(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<PruneRequest>(req.as_reader()) {
        Ok(req_data) => {
            let result = uteke.prune(req_data.ttl_days, ns(&req_data.namespace), req_data.dry_run);
            match result {
                Ok(r) => ctx.ok_response_for(req, &r),
                Err(e) => {
                    error!("Request failed: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_consolidate(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<ConsolidateRequest>(req.as_reader()) {
        Ok(req_data) => {
            if req_data.dry_run {
                let pairs = uteke.find_duplicates(ns(&req_data.namespace), req_data.threshold);
                match pairs {
                    Ok(p) => ctx.ok_response_for(req, &p),
                    Err(e) => {
                        error!("Request failed: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            } else {
                let result = uteke.consolidate(ns(&req_data.namespace), req_data.threshold, false);
                match result {
                    Ok(r) => ctx.ok_response_for(req, &r),
                    Err(e) => {
                        error!("Request failed: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

// Per-pair dedup control (#1076): caller picks the survivor.
pub(super) fn post_consolidate_pair(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<ConsolidatePairRequest>(req.as_reader()) {
        Ok(req_data) => {
            let result =
                uteke.consolidate_pair(&req_data.id_keep, &req_data.id_remove, req_data.hard);
            match result {
                Ok(r) => ctx.ok_response_for(req, &r),
                Err(e) => {
                    let status = if matches!(e, uteke_core::Error::Validation(_)) {
                        400
                    } else {
                        500
                    };
                    ctx.error_response_for(req, status, e.to_string())
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_aging(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<AgingRequest>(req.as_reader()) {
        Ok(req_data) => {
            let result = match req_data.action.as_str() {
                "status" => {
                    let status = uteke.aging_status(ns(&req_data.namespace));
                    status.map(|s| serde_json::json!(s))
                }
                "preview" => {
                    let older = req_data.older_than_days.unwrap_or(90);
                    let max_acc = req_data.max_access_count.unwrap_or(1);
                    let mems = uteke.aging_preview(older, max_acc, ns(&req_data.namespace));
                    mems.map(|m| serde_json::json!(m))
                }
                "cleanup" => {
                    if req_data.dry_run {
                        let older = req_data.older_than_days.unwrap_or(90);
                        let max_acc = req_data.max_access_count.unwrap_or(1);
                        let mems = uteke.aging_preview(older, max_acc, ns(&req_data.namespace));
                        mems.map(|m| serde_json::json!({ "dry_run": true, "candidates": m.len(), "memories": m }))
                    } else {
                        let older = req_data.older_than_days.unwrap_or(90);
                        let max_acc = req_data.max_access_count.unwrap_or(1);
                        let r = uteke.aging_cleanup(older, max_acc, ns(&req_data.namespace));
                        r.map(|c| serde_json::json!(c))
                    }
                }
                other => {
                    return ctx.error_response_for(
                        req,
                        400,
                        format!("Unknown action: {other}. Use: status, preview, cleanup"),
                    );
                }
            };
            match result {
                Ok(r) => ctx.ok_response_for(req, &r),
                Err(e) => {
                    error!("Request failed: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_importance(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<ImportanceRequest>(req.as_reader()) {
        Ok(_req_data) => match uteke.recompute_importance() {
            Ok(count) => ctx.ok_response_for(req, &serde_json::json!({ "updated": count })),
            Err(e) => {
                error!("Request failed: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_orphans(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<OrphansRequest>(req.as_reader()) {
        Ok(req_data) => {
            match uteke.find_orphans(ns(&req_data.namespace), req_data.threshold, req_data.limit) {
                Ok(orphans) => ctx.ok_response_for(req, &orphans),
                Err(e) => {
                    error!("Request failed: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn post_rebuild_backlinks(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<RebuildBacklinksRequest>(req.as_reader()) {
        Ok(_req_data) => match uteke.rebuild_backlinks() {
            Ok(count) => {
                ctx.ok_response_for(req, &serde_json::json!({ "backlinks_created": count }))
            }
            Err(e) => {
                error!("Request failed: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}
