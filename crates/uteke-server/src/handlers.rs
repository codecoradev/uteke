//! Route dispatcher — all HTTP endpoint handlers.
//!
//! This is the core router that dispatches incoming requests to the
//! appropriate handler based on method + path. Each endpoint's logic
//! lives inline in the match arms (no separate handler functions yet).

use std::io::{Cursor, Read as IoRead};
use std::sync::Mutex;

use serde::Deserialize;
use tiny_http::{Header, Method, Request, Response, StatusCode};
use tracing::error;

use uteke_core::Uteke;

use crate::context::{self, ApiRole, AuthResult, ReqCtx};
use crate::types::*;

pub fn route(uteke: &Mutex<Uteke>, ctx: &ReqCtx, req: &mut Request) -> Response<Cursor<Vec<u8>>> {
    let method = req.method().clone();
    let path = req.url().to_string();

    // CORS preflight — no auth required
    if method == Method::Options {
        return Response::new(
            StatusCode::from(204),
            ctx.preflight_headers(req),
            Cursor::new(Vec::new()),
            None,
            None,
        );
    }

    // Health endpoint — no auth required (useful for load balancers)
    let is_health = matches!((&method, path.as_str()), (Method::Get, "/health"));

    // Authenticate all non-health requests
    let auth_role = if !is_health {
        match context::check_auth(req, ctx) {
            Ok(role) => role,
            Err(resp) => return resp,
        }
    } else {
        AuthResult::Disabled
    };

    // Enforce read-only restriction (#409, #524):
    // ReadOnly tokens can use GET endpoints + read-only POST endpoints.
    // Write operations (POST /remember, POST /forget, etc.) are blocked.
    if let AuthResult::Authenticated(ApiRole::ReadOnly) = auth_role {
        // POST endpoints that are reads (semantic search, list, stats).
        // Exact match to avoid prefix-based bypass (e.g. /recallfoo).
        let read_only_post_paths = [
            "/list",
            "/recall",
            "/search",
            "/stats",
            "/room/recall",
            "/room/summary",
            "/room/document",
            "/room/stats",
            "/doc/get",
            "/doc/list",
            "/doc/search",
            "/orphans",
        ];
        let is_read = method == Method::Get || read_only_post_paths.iter().any(|ep| path == *ep);
        if !is_read {
            return ctx.error_response_for(
                req,
                403,
                "Read-only token cannot perform write operations",
            );
        }
    }

    // Lock the Uteke instance for the duration of this request.
    // This serializes requests but prevents data races on the SQLite connection.
    // Future: use rwlock for read-heavy workloads.
    let uteke = match uteke.lock() {
        Ok(u) => u,
        Err(e) => {
            return ctx.error_response_for(req, 500, format!("Internal error: {e}").as_str());
        }
    };

    match (method, path.as_str()) {
        // ── Health ──────────────────────────────────────────────────────
        (Method::Get, "/health") => {
            let total = uteke.count(None).unwrap_or(0);
            let namespaces = uteke.list_namespaces().unwrap_or_default().len();
            ctx.ok_response_for(
                req,
                &HealthResponse {
                    status: "ok",
                    memories: total,
                    namespaces,
                },
            )
        }

        // ── Remember ───────────────────────────────────────────────────
        (Method::Post, "/remember") => match read_body::<RememberRequest>(req.as_reader()) {
            Ok(req_data) => {
                // Validate input
                if let Err(e) = uteke_core::validate_input(&req_data.content, &req_data.tags) {
                    return ctx.error_response_for(req, 400, e.to_string());
                }

                let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();

                // Build metadata from optional fields
                let mut meta = serde_json::Map::new();
                if let Some(t) = &req_data.r#type {
                    meta.insert("type".into(), serde_json::Value::String(t.clone()));
                }
                if let Some(vf) = &req_data.valid_from {
                    meta.insert("valid_from".into(), serde_json::Value::String(vf.clone()));
                }
                if let Some(vu) = &req_data.valid_until {
                    meta.insert("valid_until".into(), serde_json::Value::String(vu.clone()));
                }
                let metadata = if meta.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(meta))
                };

                let result = if req_data.detect_contradiction {
                    uteke
                        .remember_with_contradiction(
                            &req_data.content,
                            &tag_refs,
                            ns(&req_data.namespace),
                            req_data.r#type.as_deref(),
                            true,
                            0.65,
                        )
                        .map(|(id, _)| id)
                } else {
                    uteke.remember(
                        &req_data.content,
                        &tag_refs,
                        metadata,
                        ns(&req_data.namespace),
                    )
                };

                match result {
                    Ok(id) => ctx.ok_response_for(req, &serde_json::json!({"id": id})),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Recall (semantic search) ────────────────────────────────────
        (Method::Post, "/recall") => match read_body::<RecallRequest>(req.as_reader()) {
            Ok(req_data) => {
                let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
                let tags_filter = if tag_refs.is_empty() {
                    None
                } else {
                    Some(tag_refs.as_slice())
                };
                // Resolve threshold: min_score > strict (→ from config or default 0.5) > 0.0
                // Server reads [recall] section from uteke.toml, matching CLI behavior.
                let min_score = if req_data.strict {
                    req_data.min_score.unwrap_or(
                        ctx.recall_config
                            .as_ref()
                            .and_then(|r| r.min_score_strict)
                            .unwrap_or(STRICT_THRESHOLD as f64) as f32,
                    )
                } else {
                    req_data.min_score.unwrap_or(
                        ctx.recall_config
                            .as_ref()
                            .and_then(|r| r.min_score)
                            .unwrap_or(DEFAULT_MIN_SCORE as f64) as f32,
                    )
                };
                // Strategy: when entity/category filters are present,
                // recall WITHOUT min_score to avoid discarding valid matches
                // that might satisfy metadata but be ranked lower. Apply
                // min_score after metadata filtering.
                let has_meta_filter = req_data.entity.is_some() || req_data.category.is_some();
                let fetch_min_score = if has_meta_filter { 0.0 } else { min_score };
                let fetch_limit = if has_meta_filter {
                    // Cap at 200 to prevent unbounded amplification.
                    // May return fewer than requested when matches are sparse.
                    (req_data.limit * 10).min(200)
                } else {
                    req_data.limit
                };

                // Time-travel mode: parse --at and use recall_at_time
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

                let recall_result = if let Some(pit) = point_in_time {
                    uteke.recall_at_time(
                        &req_data.query,
                        fetch_limit,
                        tags_filter,
                        ns(&req_data.namespace),
                        pit,
                        fetch_min_score,
                    )
                } else {
                    uteke.recall(
                        &req_data.query,
                        fetch_limit,
                        tags_filter,
                        ns(&req_data.namespace),
                        fetch_min_score,
                    )
                };

                // Unified search path (#531): when search_type is specified and
                // no memory-only metadata filters (entity/category) are present,
                // use recall_unified. Entity/category only apply to memories, so
                // their presence forces the legacy memory-only recall path.
                let has_meta_filter = req_data.entity.is_some() || req_data.category.is_some();
                let unified_result = if req_data.search_type.is_some()
                    && point_in_time.is_none()
                    && !has_meta_filter
                {
                    let search_type = match req_data.search_type.as_deref() {
                        Some("memory") => uteke_core::SearchType::Memory,
                        Some("doc") => uteke_core::SearchType::Document,
                        Some("all") | None => uteke_core::SearchType::All,
                        Some(other) => {
                            return ctx.error_response_for(
                                req,
                                400,
                                format!("Invalid search_type: '{other}'. Use 'all', 'memory', or 'doc'."),
                            );
                        }
                    };
                    Some(uteke.recall_unified(
                        &req_data.query,
                        req_data.limit,
                        tags_filter,
                        ns(&req_data.namespace),
                        min_score,
                        search_type,
                    ))
                } else {
                    None
                };

                // Prefer unified results when available (#531)
                match unified_result {
                    Some(Ok(results)) => ctx.ok_response_for(req, &results),
                    Some(Err(e)) => {
                        error!("Unified search error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                    None => {
                        // Fall through to existing memory-only recall path
                        match recall_result {
                            Ok(raw_results) => {
                                // Post-filter by entity/category metadata
                                let mut results: Vec<_> = raw_results
                                    .into_iter()
                                    .filter(|sr| {
                                        if let Some(ent) = &req_data.entity {
                                            let matches = sr
                                                .memory
                                                .metadata
                                                .get("entity")
                                                .and_then(|v| v.as_str())
                                                .is_some_and(|e| e == ent);
                                            if !matches {
                                                return false;
                                            }
                                        }
                                        if let Some(cat) = &req_data.category {
                                            let matches = sr
                                                .memory
                                                .metadata
                                                .get("category")
                                                .and_then(|v| v.as_str())
                                                .is_some_and(|c| c == cat);
                                            if !matches {
                                                return false;
                                            }
                                        }
                                        true
                                    })
                                    .collect::<Vec<_>>();
                                // Apply min_score filter after metadata filtering
                                // (deferred from recall call to avoid losing valid matches)
                                if min_score > 0.0 {
                                    results.retain(|sr| sr.score >= min_score);
                                }
                                // Trim to requested limit after filtering
                                results.truncate(req_data.limit);

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
        },

        // ── Search (keyword) ────────────────────────────────────────────
        (Method::Post, "/search") => match read_body::<SearchRequest>(req.as_reader()) {
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
        },

        // ── List ────────────────────────────────────────────────────────
        (Method::Post, "/list") => match read_body::<ListParams>(req.as_reader()) {
            Ok(req_data) => {
                // Time-travel mode: parse --at and use list_at_time
                let list_result = match req_data.at.as_deref() {
                    Some(at_str) => match chrono::DateTime::parse_from_rfc3339(at_str) {
                        Ok(dt) => {
                            let pit = dt.with_timezone(&chrono::Utc);
                            uteke.list_at_time(
                                req_data.tag.as_deref(),
                                req_data.limit,
                                req_data.offset,
                                ns(&req_data.namespace),
                                pit,
                            )
                        }
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
                    None => uteke.list(
                        req_data.tag.as_deref(),
                        req_data.limit,
                        req_data.offset,
                        ns(&req_data.namespace),
                    ),
                };
                match list_result {
                    Ok(memories) => ctx.ok_response_for(req, &memories),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Forget by ID or tag (DELETE /forget?id=xxx or ?tag=xxx) ────
        (Method::Delete, p) if p == "/forget" || p.starts_with("/forget?") => {
            let query = p.split('?').nth(1).unwrap_or("");
            let params: std::collections::HashMap<String, String> = query
                .split('&')
                .filter_map(|pair| {
                    let mut kv = pair.splitn(2, '=');
                    Some((kv.next()?.to_string(), kv.next()?.to_string()))
                })
                .collect();

            if let Some(id) = params.get("id") {
                // Validate UUID format
                if uuid::Uuid::parse_str(id).is_err() {
                    return ctx.error_response_for(req, 400, format!("Invalid UUID format: {id}"));
                }
                match uteke.forget(id) {
                    Ok(()) => ctx.ok_response_for(req, &serde_json::json!({"forgotten": id})),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            } else if let Some(tag) = params.get("tag") {
                let namespace = params.get("namespace").map(|s| s.as_str());
                match uteke.bulk_forget_by_tag(tag, namespace) {
                    Ok(result) => ctx.ok_response_for(req, &result),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            } else {
                ctx.error_response_for(req, 400, "Provide ?id= or ?tag= parameter")
            }
        }

        // ── Stats (GET = all or ?namespace=<name>) ───────────────────
        (Method::Get, "/stats") => {
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
        (Method::Post, "/stats") => {
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

        // ── Namespaces ──────────────────────────────────────────────────
        (Method::Get, p) if p == "/namespaces" || p.starts_with("/namespaces?") => {
            let with_counts = path.contains("with_counts=true");
            if with_counts {
                match uteke.list_namespaces_with_counts() {
                    Ok(counts) => {
                        #[derive(serde::Serialize)]
                        struct NamespaceCount {
                            name: String,
                            count: usize,
                        }
                        let result: Vec<NamespaceCount> = counts
                            .into_iter()
                            .map(|(name, count)| NamespaceCount { name, count })
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

        // ── Recent (#528) ──────────────────────────────────────────────
        (Method::Get, p) if p == "/recent" || p.starts_with("/recent?") => {
            let ns = parse_query_namespace(&path);
            let query = path.split('?').nth(1).unwrap_or("");
            let limit = parse_query_param(query, "limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(20);
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

        // ── Graph Visualization (#408) ───────────────────────────────────
        (Method::Get, p) if p == "/graph" || p.starts_with("/graph?") => {
            let ns = parse_query_namespace(&path);
            match uteke.graph_data(ns.as_deref()) {
                Ok(data) => ctx.ok_response_for(req, &data),
                Err(e) => {
                    error!("Graph data error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Graph Mutation: Add Edge (#542) ──────────────────────────────
        (Method::Post, "/graph/edge") => match read_body::<GraphEdgeRequest>(req.as_reader()) {
            Ok(req_data) => {
                // Reject self-loops
                if req_data.source == req_data.target {
                    return ctx.error_response_for(
                        req,
                        400,
                        "Self-loop edges are not allowed (source == target)",
                    );
                }

                // Validate both nodes exist as memories (issue #542 acceptance criteria)
                match uteke.get_by_id(&req_data.source) {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        return ctx.error_response_for(
                            req,
                            404,
                            format!("Source memory not found: {}", req_data.source),
                        );
                    }
                    Err(e) => {
                        error!("Internal error: {e}");
                        return ctx.error_response_for(req, 500, "Internal server error");
                    }
                }
                match uteke.get_by_id(&req_data.target) {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        return ctx.error_response_for(
                            req,
                            404,
                            format!("Target memory not found: {}", req_data.target),
                        );
                    }
                    Err(e) => {
                        error!("Internal error: {e}");
                        return ctx.error_response_for(req, 500, "Internal server error");
                    }
                }

                let conn = uteke.graph_store();
                let gs = uteke_core::graph::GraphStore::new(conn);
                let relation = req_data.edge_type.as_deref().unwrap_or("related");
                let weight = req_data.weight.unwrap_or(1.0);

                match gs.add_edge(&req_data.source, &req_data.target, relation, weight) {
                    Ok(()) => ctx.ok_response_for(req, &serde_json::json!({"ok": true})),
                    Err(e) => {
                        error!("Graph add_edge error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Graph Mutation: Remove Edge (#542) ────────────────────────────
        (Method::Delete, p) if p == "/graph/edge" || p.starts_with("/graph/edge?") => {
            let query = p.split('?').nth(1).unwrap_or("");
            let source = parse_query_param(query, "source");
            let target = parse_query_param(query, "target");

            match (&source, &target) {
                (Some(src), Some(tgt)) => {
                    let conn = uteke.graph_store();
                    let gs = uteke_core::graph::GraphStore::new(conn);
                    match gs.remove_edge(src, tgt) {
                        Ok(true) => ctx.ok_response_for(req, &serde_json::json!({"ok": true})),
                        Ok(false) => ctx.error_response_for(
                            req,
                            404,
                            format!("Edge not found: {src} -> {tgt}"),
                        ),
                        Err(e) => {
                            error!("Graph remove_edge error: {e}");
                            ctx.error_response_for(req, 500, "Internal server error")
                        }
                    }
                }
                _ => ctx.error_response_for(
                    req,
                    400,
                    "Provide both ?source=...&target=... query parameters",
                ),
            }
        }

        // ── Room Summary ────────────────────────────────────────────────
        (Method::Post, "/room/summary") => {
            #[derive(Deserialize)]
            struct RoomSummaryRequest {
                room_id: String,
            }
            match read_body::<RoomSummaryRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.room_summary(&req_data.room_id) {
                    Ok(Some(summary)) => ctx.ok_response_for(req, &summary),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("Room not found: {}", req_data.room_id),
                    ),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        // ── Room Document ────────────────────────────────────────────────
        (Method::Post, "/room/document") => {
            #[derive(Deserialize)]
            struct RoomDocumentRequest {
                room_id: String,
            }
            match read_body::<RoomDocumentRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.room_document(&req_data.room_id) {
                    Ok(Some(doc)) => ctx.ok_response_for(req, &doc),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("Room not found: {}", req_data.room_id),
                    ),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        // ── Get memory by ID ──────────────────────────────────────────
        (Method::Get, p) if p.starts_with("/memory?id=") => {
            let id = p.trim_start_matches("/memory?id=");
            // Validate UUID format
            if uuid::Uuid::parse_str(id).is_err() {
                return ctx.error_response_for(req, 400, format!("Invalid UUID format: {id}"));
            }
            match uteke.get_by_id(id) {
                Ok(Some(memory)) => ctx.ok_response_for(req, &memory),
                Ok(None) => ctx.error_response_for(req, 404, format!("Memory not found: {id}")),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Room Memories (chronological listing — GET /room/memories) ────
        (Method::Get, p) if p == "/room/memories" || p.starts_with("/room/memories?") => {
            let query_str = p.strip_prefix("/room/memories?");
            let room_id = query_str.and_then(|q| parse_query_param(q, "room_id"));
            let room_id = match room_id {
                Some(id) => id,
                None => {
                    return ctx.error_response_for(
                        req,
                        400,
                        "Missing required parameter: room_id. Usage: GET /room/memories?room_id=<id>[&author=<author>&limit=<n>]",
                    );
                }
            };
            let limit = query_str
                .and_then(|q| parse_query_param(q, "limit"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(100);
            let author = query_str.and_then(|q| parse_query_param(q, "author"));
            match uteke.recall_room(&room_id, author.as_deref(), limit) {
                Ok(memories) => ctx.ok_response_for(req, &memories),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Room Recall (semantic — requires non-empty query) ─────────────
        (Method::Post, "/room/recall") => match read_body::<RoomRecallRequest>(req.as_reader()) {
            Ok(req_data) => {
                if req_data.query.trim().is_empty() {
                    return ctx.error_response_for(
                        req,
                        400,
                        "Empty query is not allowed. Use GET /room/memories for chronological listing.",
                    );
                }
                let min_score = req_data.min_score.unwrap_or(
                    ctx.recall_config
                        .as_ref()
                        .and_then(|r| r.min_score)
                        .unwrap_or(DEFAULT_MIN_SCORE as f64) as f32,
                );
                match uteke.recall_room_semantic(
                    &req_data.room_id,
                    &req_data.query,
                    req_data.limit,
                    req_data.author.as_deref(),
                    min_score,
                ) {
                    Ok(results) => ctx.ok_response_for(req, &results),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Room management endpoints (#395) ────────────────────────────
        (Method::Post, "/room/create") => {
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
                            &serde_json::json!({
                                "created": req_data.room_id,
                                "namespace": req_data.namespace
                            }),
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

        (Method::Get, "/room/list") => {
            let ns_param = parse_query_namespace(&path);
            match uteke.list_rooms(ns_param.as_deref()) {
                Ok(rooms) => ctx.ok_response_for(req, &rooms),
                Err(e) => {
                    error!("Internal error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        (Method::Post, "/room/stats") => {
            #[derive(Deserialize)]
            struct RoomStatsRequest {
                room_id: String,
            }
            match read_body::<RoomStatsRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.room_stats(&req_data.room_id) {
                    Ok(Some(stats)) => ctx.ok_response_for(req, &stats),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("Room not found: {}", req_data.room_id),
                    ),
                    Err(e) => {
                        error!("Internal error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        (Method::Delete, p) if p == "/room/delete" || p.starts_with("/room/delete?") => {
            let room_id = if let Some(q) = p.strip_prefix("/room/delete?") {
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
                            return ctx.error_response_for(req, 400, "Missing 'room_id' parameter")
                        }
                    }
                }
            };
            match uteke.delete_room(&room_id) {
                Ok(()) => ctx.ok_response_for(req, &serde_json::json!({"deleted": room_id})),
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
        // ── Context Summary (#442) ───────────────────────────────────────
        (Method::Post, "/context") => match read_body::<serde_json::Value>(req.as_reader()) {
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
        },

        // ── Dream Cycle (#442) ─────────────────────────────────────────────
        (Method::Post, "/dream") => match read_body::<serde_json::Value>(req.as_reader()) {
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
        },

        (Method::Post, "/mcp") => {
            // Enforce a body size limit to prevent memory exhaustion
            // (CodeCora #397). 1 MiB is generous for JSON-RPC.
            const MAX_MCP_BODY: u64 = 1024 * 1024;
            // Check Content-Length and reject oversized requests.
            let content_length = req
                .headers()
                .iter()
                .find(|h| h.field.as_str() == "content-length")
                .and_then(|h| h.value.as_str().parse::<u64>().ok())
                .unwrap_or(0);
            if content_length > MAX_MCP_BODY {
                return ctx.error_response_for(req, 413, "Payload too large");
            }
            let mut body = String::new();
            if let Err(e) = req.as_reader().take(MAX_MCP_BODY).read_to_string(&mut body) {
                return ctx.error_response_for(req, 400, format!("Failed to read body: {e}"));
            }
            // None = notification (no response per JSON-RPC 2.0 §4.1) → 204 No Content
            match uteke_mcp::handle_jsonrpc(&uteke, &body) {
                Some(response) => tiny_http::Response::from_string(response)
                    .with_header(
                        tiny_http::Header::from_bytes(
                            &b"Content-Type"[..],
                            &b"application/json"[..],
                        )
                        .unwrap(),
                    )
                    .with_header(
                        tiny_http::Header::from_bytes(
                            &b"MCP-Protocol-Version"[..],
                            &b"2025-06-18"[..],
                        )
                        .unwrap(),
                    ),
                None => tiny_http::Response::from_string("")
                    .with_status_code(204)
                    .with_header(
                        tiny_http::Header::from_bytes(
                            &b"MCP-Protocol-Version"[..],
                            &b"2025-06-18"[..],
                        )
                        .unwrap(),
                    ),
            }
        }

        // ── Document: Create / Upsert ────────────────────────────────────
        (Method::Post, "/doc/create") => match read_body::<DocCreateRequest>(req.as_reader()) {
            Ok(req_data) => {
                let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
                let parent = req_data.parent.as_deref();
                match uteke.doc_upsert_with_parent(
                    &req_data.slug,
                    req_data.title.as_deref().unwrap_or(""),
                    &req_data.content,
                    &tag_refs,
                    ns(&req_data.namespace),
                    parent,
                ) {
                    Ok(id) => ctx.ok_response_for(
                        req,
                        &serde_json::json!({"id": id, "slug": req_data.slug}),
                    ),
                    Err(e) => {
                        if e.to_string().contains("already exists") {
                            ctx.error_response_for(
                                req,
                                409,
                                format!("document slug '{}' already exists", req_data.slug),
                            )
                        } else if e.to_string().contains("maximum")
                            || e.to_string().contains("parent")
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
        },

        // ── Document: Get ───────────────────────────────────────────────
        (Method::Post, "/doc/get") => match read_body::<DocGetRequest>(req.as_reader()) {
            Ok(req_data) => match resolve_doc_id(&req_data) {
                Ok(id_or_slug) => match uteke.doc_get(id_or_slug, ns(&req_data.namespace)) {
                    Ok(Some(doc)) => ctx.ok_response_for(req, &doc),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("document not found: {id_or_slug}"),
                    ),
                    Err(e) => {
                        error!("doc get error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            },
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Document: Update (partial) ────────────────────────────────
        (Method::Post, "/doc/update") => match read_body::<DocUpdateRequest>(req.as_reader()) {
            Ok(req_data) => match resolve_doc_id_update(&req_data) {
                Ok(id_or_slug) => {
                    let title = req_data.title.as_deref();
                    let content = req_data.content.as_deref();
                    let tags = req_data.tags.as_deref();
                    let metadata = req_data.metadata.as_ref();
                    match uteke.doc_update(
                        id_or_slug,
                        ns(&req_data.namespace),
                        title,
                        content,
                        tags,
                        metadata,
                    ) {
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
        },

        // ── Document: List ─────────────────────────────────────────────
        (Method::Post, "/doc/list") => match read_body::<DocListParams>(req.as_reader()) {
            Ok(params) => {
                let result = if params.roots_only {
                    uteke.doc_list_roots(ns(&params.namespace), params.limit)
                } else if let Some(ref parent) = params.parent {
                    uteke.doc_list_children(parent, ns(&params.namespace), params.limit)
                } else {
                    uteke.doc_list(ns(&params.namespace), params.limit)
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
        },

        // ── Document: Search ────────────────────────────────────────────
        (Method::Post, "/doc/search") => match read_body::<DocSearchRequest>(req.as_reader()) {
            Ok(req_data) => {
                match uteke.doc_search(
                    &req_data.query,
                    ns(&req_data.namespace),
                    req_data.limit,
                    &req_data.mode,
                ) {
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
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Document: Move ───────────────────────────────────────────────
        (Method::Post, "/doc/move") => match read_body::<DocMoveRequest>(req.as_reader()) {
            Ok(req_data) => match resolve_doc_id_move(&req_data) {
                Ok(id_or_slug) => {
                    let new_parent = req_data.new_parent.as_deref();
                    match uteke.doc_move(id_or_slug, new_parent, ns(&req_data.namespace)) {
                        Ok(moved) => ctx.ok_response_for(req, &serde_json::json!({"moved": moved})),
                        Err(e) => {
                            if e.to_string().contains("not found") {
                                ctx.error_response_for(req, 404, e.to_string())
                            } else {
                                error!("doc move error: {e}");
                                ctx.error_response_for(req, 500, "Internal server error")
                            }
                        }
                    }
                }
                Err(e) => ctx.error_response_for(req, 400, e),
            },
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Document: Delete ─────────────────────────────────────────────
        (Method::Delete, p) if p == "/doc/delete" || p.starts_with("/doc/delete?") => {
            let url = req.url().to_string();
            let ns_param = parse_query_param(&url, "namespace");
            let id = parse_query_param(&url, "id");
            let slug = parse_query_param(&url, "slug");

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

            match uteke.doc_delete(id_or_slug, ns_param.as_deref()) {
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

        // ── Tags: List with counts ───────────────────────────────────────
        (Method::Get, p) if p == "/tags" || p.starts_with("/tags?") => {
            let ns = parse_query_namespace(&path);
            match uteke.tags_with_counts(ns.as_deref()) {
                Ok(tags) => ctx.ok_response_for(req, &tags),
                Err(e) => {
                    error!("Tags list error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Tags: Rename ─────────────────────────────────────────────────
        (Method::Post, "/tags/rename") => match read_body::<TagRenameRequest>(req.as_reader()) {
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
        },

        // ── Tags: Delete ─────────────────────────────────────────────────
        (Method::Post, "/tags/delete") => match read_body::<TagDeleteRequest>(req.as_reader()) {
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
        },

        // ── Pin ───────────────────────────────────────────────────────────
        (Method::Post, "/pin") => match read_body::<PinRequest>(req.as_reader()) {
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
        },

        // ── Unpin ─────────────────────────────────────────────────────────
        (Method::Post, "/unpin") => match read_body::<PinRequest>(req.as_reader()) {
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
        },

        // ── Timeline ─────────────────────────────────────────────────────
        (Method::Get, p) if p.starts_with("/timeline?") || p.starts_with("/timeline?id=") => {
            let query = p.split('?').nth(1).unwrap_or("");
            let id = match parse_query_param(query, "id") {
                Some(id) => id,
                None => return ctx.error_response_for(req, 400, "Missing 'id' query parameter"),
            };
            let limit = parse_query_param(query, "limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(50);
            match uteke.timeline(&id, limit) {
                Ok(events) => ctx.ok_response_for(req, &events),
                Err(e) => {
                    error!("Timeline error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Edges ────────────────────────────────────────────────────────
        (Method::Get, p) if p.starts_with("/edges?") || p.starts_with("/edges?id=") => {
            let query = p.split('?').nth(1).unwrap_or("");
            let id = match parse_query_param(query, "id") {
                Some(id) => id,
                None => return ctx.error_response_for(req, 400, "Missing 'id' query parameter"),
            };
            match uteke.edges_for(&id) {
                Ok(edges) => ctx.ok_response_for(req, &edges),
                Err(e) => {
                    error!("Edges error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Extract (LLM fact extraction → store) ────────────────────────
        (Method::Post, "/extract") => match read_body::<ExtractRequest>(req.as_reader()) {
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
                let tag_refs: Vec<&str> = req_data.tags.iter().map(|s| s.as_str()).collect();
                let fact_ns = ns(&req_data.namespace);

                for fact in &facts {
                    let mut meta = serde_json::Map::new();
                    if let Some(t) = &req_data.r#type {
                        meta.insert("type".into(), serde_json::Value::String(t.clone()));
                    }
                    meta.insert(
                        "source".into(),
                        serde_json::Value::String("extraction".into()),
                    );
                    let metadata = if meta.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::Object(meta))
                    };

                    if let Ok(id) = uteke.remember(fact, &tag_refs, metadata, fact_ns) {
                        stored_ids.push(id);
                    }
                }

                ctx.ok_response_for(
                    req,
                    &serde_json::json!({
                        "facts": facts,
                        "count": facts.len(),
                        "stored": stored_ids.len(),
                        "stored_ids": stored_ids,
                    }),
                )
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Import (JSONL) ──────────────────────────────────────────────
        (Method::Post, "/import") => match read_body::<ImportRequest>(req.as_reader()) {
            Ok(req_data) => {
                if validate_content_size(&req_data.content, 5_242_880).is_err() {
                    return ctx.error_response_for(req, 413, "Content too large (max 5MB)");
                }

                // Merge request tags into the JSONL entries.
                // The core import method handles re-embedding.
                let import_ns = ns(&req_data.namespace);
                match uteke.import(&req_data.content, import_ns) {
                    Ok(result) => ctx.ok_response_for(
                        req,
                        &serde_json::json!({
                            "imported": result.imported,
                            "skipped": result.skipped,
                        }),
                    ),
                    Err(e) => {
                        error!("Import error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Export (JSONL) ──────────────────────────────────────────────
        (Method::Get, p) if p == "/export" || p.starts_with("/export?") => {
            let export_ns = parse_query_namespace(&path);
            match uteke.export(export_ns.as_deref()) {
                Ok(jsonl) => {
                    let mut headers = ctx.cors_headers_for(req);
                    headers.push(
                        Header::from_bytes(&b"Content-Type"[..], &b"application/x-ndjson"[..])
                            .unwrap(),
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

        // ── Prune (maintenance) ───────────────────────────────────────────
        (Method::Post, "/prune") => match read_body::<PruneRequest>(req.as_reader()) {
            Ok(req_data) => {
                let result =
                    uteke.prune(req_data.ttl_days, ns(&req_data.namespace), req_data.dry_run);
                match result {
                    Ok(r) => ctx.ok_response_for(req, &r),
                    Err(e) => ctx.error_response_for(req, 500, e.to_string()),
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Consolidate (maintenance) ────────────────────────────────────
        (Method::Post, "/consolidate") => match read_body::<ConsolidateRequest>(req.as_reader()) {
            Ok(req_data) => {
                if req_data.dry_run {
                    let pairs = uteke.find_duplicates(ns(&req_data.namespace), req_data.threshold);
                    match pairs {
                        Ok(p) => ctx.ok_response_for(req, &p),
                        Err(e) => ctx.error_response_for(req, 500, e.to_string()),
                    }
                } else {
                    let result =
                        uteke.consolidate(ns(&req_data.namespace), req_data.threshold, false);
                    match result {
                        Ok(r) => ctx.ok_response_for(req, &r),
                        Err(e) => ctx.error_response_for(req, 500, e.to_string()),
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Aging (maintenance) ─────────────────────────────────────────
        (Method::Post, "/aging") => match read_body::<AgingRequest>(req.as_reader()) {
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
                        )
                    }
                };
                match result {
                    Ok(r) => ctx.ok_response_for(req, &r),
                    Err(e) => ctx.error_response_for(req, 500, e.to_string()),
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Importance (monitoring) ────────────────────────────────────────
        (Method::Post, "/importance") => match read_body::<ImportanceRequest>(req.as_reader()) {
            Ok(_req_data) => match uteke.recompute_importance() {
                Ok(count) => ctx.ok_response_for(req, &serde_json::json!({ "updated": count })),
                Err(e) => ctx.error_response_for(req, 500, e.to_string()),
            },
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Orphans (monitoring — read-only) ─────────────────────────────
        (Method::Post, "/orphans") => match read_body::<OrphansRequest>(req.as_reader()) {
            Ok(req_data) => {
                match uteke.find_orphans(
                    ns(&req_data.namespace),
                    req_data.threshold,
                    req_data.limit,
                ) {
                    Ok(orphans) => ctx.ok_response_for(req, &orphans),
                    Err(e) => ctx.error_response_for(req, 500, e.to_string()),
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Rebuild Backlinks (monitoring) ────────────────────────────────
        (Method::Post, "/rebuild-backlinks") => {
            match read_body::<RebuildBacklinksRequest>(req.as_reader()) {
                Ok(_req_data) => match uteke.rebuild_backlinks() {
                    Ok(count) => {
                        ctx.ok_response_for(req, &serde_json::json!({ "backlinks_created": count }))
                    }
                    Err(e) => ctx.error_response_for(req, 500, e.to_string()),
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        // ── 404 ─────────────────────────────────────────────────────────
        _ => ctx.error_response_for(req, 404, "Not found"),
    }
}

/// Helper: validate content for extract/import to prevent abuse.
fn validate_content_size(content: &str, max_bytes: usize) -> Result<(), &'static str> {
    if content.len() > max_bytes {
        Err("Content too large")
    } else {
        Ok(())
    }
}

/// Helper: resolve extraction config with per-request overrides.
fn resolve_extraction_config(
    ctx: &ReqCtx,
    req_model: Option<&str>,
    req_max_facts: Option<usize>,
    req_api_key: Option<&str>,
) -> uteke_core::extraction::ExtractionConfig {
    let base = ctx.extraction_config.clone().unwrap_or_default();
    uteke_core::extraction::ExtractionConfig {
        model: req_model.map(String::from).unwrap_or(base.model),
        api_key: req_api_key.map(String::from).unwrap_or(base.api_key),
        base_url: base.base_url,
        endpoint_path: base.endpoint_path,
        max_facts: req_max_facts.unwrap_or(base.max_facts),
    }
}
