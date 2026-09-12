//! Route dispatcher — all HTTP endpoint handlers.
//!
//! This is the core router that dispatches incoming requests to the
//! appropriate handler based on method + path. Each endpoint's logic
//! lives inline in the match arms (no separate handler functions yet).

use std::io::{Cursor, Read as IoRead};
use std::sync::Mutex;

use serde::Deserialize;
use tiny_http::{Header, Method, Request, Response, StatusCode};
use tracing::{error, warn};

use uteke_core::Uteke;
use uteke_core::memory::types::validate_author_type;

use crate::context::{self, ApiRole, AuthResult, ReqCtx};
use crate::types::*;

/// Current API version constant — used by health and versioned routes.
const API_LATEST: &str = "v2";
const API_VERSIONS: &[&str] = &["v1", "v2"];

pub fn route(uteke: &Mutex<Uteke>, ctx: &ReqCtx, req: &mut Request) -> Response<Cursor<Vec<u8>>> {
    let method = req.method().clone();
    let raw_path = req.url().to_string();

    // ── API Versioning (#737): parse /api/vN/ prefix ────────────────────
    let (api_version, path) = match ApiVersion::from_path(&raw_path) {
        Some((ver, stripped)) => (Some(ver), stripped.to_string()),
        None => (None, raw_path.clone()),
    };

    // Route matching uses path-only (without query string).
    // Handler functions still receive the full `path` with query params intact.
    let route_path = path.split('?').next().unwrap_or(&path);

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
    let is_health = matches!((&method, route_path), (Method::Get, "/health"));

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
            "/room/summary-document",
            "/room/stats",
            "/room/document/list",
            "/doc/get",
            "/doc/list",
            "/doc/search",
            "/doc/room/list",
            "/memory/doc-refs",
            "/doc/mem-refs",
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

    match (method, route_path) {
        // ── Health ──────────────────────────────────────────────────────
        (Method::Get, "/health") => {
            let total = uteke.count(None).unwrap_or(0);
            let namespaces = uteke.list_namespaces().unwrap_or_default().len();
            // Populate update_available from cache (non-blocking, no network).
            let update_available = uteke_core::update_check::check_cached()
                .filter(|i| i.is_update_available())
                .map(|i| i.latest);
            ctx.ok_response_for(
                req,
                &HealthResponse {
                    status: "ok",
                    version: env!("CARGO_PKG_VERSION"),
                    memories: total,
                    namespaces,
                    api_versions: Some(API_VERSIONS.to_vec()),
                    api_latest: Some(API_LATEST),
                    update_available,
                },
            )
        }

        // ── Memory Tools Guide (#1010) ──────────────────────────────────
        (Method::Get, "/guide") => {
            #[derive(serde::Serialize)]
            struct GuideResponse<'a> {
                guide: &'a str,
            }
            ctx.ok_response_for(
                req,
                &GuideResponse {
                    guide: &uteke_core::guide::default_guide(),
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

                // Build metadata from optional fields — matches CLI behavior.
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
                if let Some(entity) = &req_data.entity {
                    meta.insert("entity".into(), serde_json::Value::String(entity.clone()));
                }
                if let Some(category) = &req_data.category {
                    meta.insert(
                        "category".into(),
                        serde_json::Value::String(category.clone()),
                    );
                }
                // Merge caller-supplied metadata object into the map (#682).
                if let Some(serde_json::Value::Object(extra)) = &req_data.metadata {
                    for (k, v) in extra {
                        meta.insert(k.clone(), v.clone());
                    }
                }
                let metadata = if meta.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(meta))
                };

                // Validate author_type BEFORE any write (#1083, cora finding):
                // invalid values must reject the whole request — inserting then
                // failing would leave a persisted memory the client believes failed.
                if let Some(at) = req_data.author_type.as_deref() {
                    if let Err(e) = validate_author_type(at) {
                        return ctx.error_response_for(req, 400, e.to_string());
                    }
                }

                let result = if req_data.detect_contradiction {
                    uteke
                        .remember_with_contradiction(
                            &req_data.content,
                            &tag_refs,
                            metadata,
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
                    Ok(id) => {
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
                        // Echo author_type so clients can confirm what was
                        // recorded (#1106) — data was already stored correctly,
                        // the response just omitted the field. Default mirrors
                        // the schema default (author_type DEFAULT 'agent', #1083).
                        ctx.ok_response_for(
                            req,
                            &serde_json::json!({
                                "id": id,
                                "author_type": req_data.author_type.clone().unwrap_or_else(|| "agent".to_string()),
                            }),
                        )
                    }
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
                // #907: Reject empty/whitespace queries — they return misleading
                // results (top-N by recency, not relevance).
                if req_data.query.trim().is_empty() {
                    return ctx.error_response_for(
                        req,
                        400,
                        "Query must not be empty or whitespace-only",
                    );
                }
                // #903: Cap limit to prevent DoS via unbounded queries.
                let limit = req_data.limit.min(MAX_LIMIT);

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
                // Entity/category filters are now pushed into the core recall
                // candidate loop (#663) — no 10x fetch amplification needed.

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

                let entity_filter = req_data.entity.as_deref();
                let category_filter = req_data.category.as_deref();

                // #902: Parse temporal range filters (before/after RFC3339).
                let after_ts = match req_data.after.as_deref() {
                    Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
                        Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
                        Err(_) => {
                            return ctx.error_response_for(
                                req,
                                400,
                                format!(
                                    "Invalid 'after' timestamp: {ts}. Use RFC3339 format (e.g. 2026-01-01T00:00:00Z)"
                                ),
                            );
                        }
                    },
                    None => None,
                };
                let before_ts = match req_data.before.as_deref() {
                    Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
                        Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
                        Err(_) => {
                            return ctx.error_response_for(
                                req,
                                400,
                                format!(
                                    "Invalid 'before' timestamp: {ts}. Use RFC3339 format (e.g. 2026-01-01T00:00:00Z)"
                                ),
                            );
                        }
                    },
                    None => None,
                };
                let has_temporal = after_ts.is_some() || before_ts.is_some();

                // Resolve recall strategy ONCE for every path (#1034):
                // request `strategy` > config `[recall] default_strategy` >
                // Hybrid — matching the CLI default. Unknown values are a
                // loud 400 on all paths, never a silent no-op.
                let strategy_name = req_data.strategy.as_deref().or_else(|| {
                    ctx.recall_config
                        .as_ref()
                        .and_then(|r| r.default_strategy.as_deref())
                });
                let strategy = match strategy_name {
                    Some(name) => match uteke_core::RecallStrategy::from_str_opt(name) {
                        Some(s) => s,
                        None => {
                            return ctx.error_response_for(
                                req,
                                400,
                                format!(
                                    "Invalid strategy: '{name}'. Use 'vector', 'fts5', 'hybrid', 'graph', or 'fusion'."
                                ),
                            );
                        }
                    },
                    // Implicit default since 0.16.0 (#1123): fusion.
                    // Matches the core enum default and CLI default_strategy.
                    None => uteke_core::RecallStrategy::Fusion,
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

                // Unified search path (#531): when search_type is specified,
                // use recall_unified. Entity/category filters are passed
                // through to the core recall candidate loop (#663).
                let unified_result = if req_data.search_type.is_some() && point_in_time.is_none() {
                    let search_type = match req_data.search_type.as_deref() {
                        Some("memory") => uteke_core::SearchType::Memory,
                        Some("doc") => uteke_core::SearchType::Document,
                        Some("all") | None => uteke_core::SearchType::All,
                        Some(other) => {
                            return ctx.error_response_for(
                                req,
                                400,
                                format!(
                                    "Invalid search_type: '{other}'. Use 'all', 'memory', or 'doc'."
                                ),
                            );
                        }
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
                        let needs_post_filter =
                            entity_filter.is_some() || category_filter.is_some();
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
                    Ok(memories) => {
                        // #1188: opt-in pagination envelope. The default
                        // response stays a bare array for existing clients.
                        if req_data.include_meta && req_data.at.is_none() {
                            // Total matching rows (same filters as the page).
                            let total = match &req_data.tag {
                                Some(tag) => uteke.count_by_tag(tag, ns(&req_data.namespace)),
                                None => uteke.count(ns(&req_data.namespace)),
                            }
                            .unwrap_or(0);
                            let fetched = memories.len();
                            let has_more = req_data.offset + fetched < total;
                            let next_offset = if has_more {
                                Some(req_data.offset + fetched)
                            } else {
                                None
                            };
                            return ctx.ok_response_for(
                                req,
                                &serde_json::json!({
                                    "memories": memories,
                                    "total": total,
                                    "has_more": has_more,
                                    "next_offset": next_offset,
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
        },

        // ── Forget by ID or tag (DELETE /forget?id=xxx or ?tag=xxx) ────
        (Method::Delete, "/forget") => {
            let query = path.split('?').nth(1).unwrap_or("");
            let params: std::collections::HashMap<String, String> = query
                .split('&')
                .filter_map(|pair| {
                    let mut kv = pair.splitn(2, '=');
                    Some((kv.next()?.to_string(), kv.next()?.to_string()))
                })
                .collect();

            if let Some(id) = params.get("id") {
                // Accept full UUID or short ID prefix (#794).
                // `list` and `room_recall` display only 8-char prefixes, so
                // `forget` must resolve them back to full UUIDs.
                let resolved_id = if uuid::Uuid::parse_str(id).is_ok() {
                    id.clone()
                } else {
                    match uteke.resolve_id_prefix(id) {
                        Ok(Some(full)) => full,
                        Ok(None) => {
                            return ctx.error_response_for(
                                req,
                                404,
                                format!("Memory not found: {id}"),
                            );
                        }
                        Err(e) => {
                            error!("Resolve error: {e}");
                            return ctx.error_response_for(req, 400, e.to_string());
                        }
                    }
                };
                // Check existence before deleting (#762) — forget() silently
                // returns Ok(()) even when the ID doesn't exist.
                if uteke.get_by_id(&resolved_id).ok().flatten().is_none() {
                    return ctx.error_response_for(req, 404, format!("Memory not found: {id}"));
                }
                match uteke.forget(&resolved_id) {
                    Ok(()) => {
                        ctx.ok_response_for(req, &serde_json::json!({"forgotten": resolved_id}))
                    }
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

        // ── Lifecycle (#936): status, cycle, promote ──────────────────
        (Method::Get, "/lifecycle/status") => {
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

        (Method::Get, "/lifecycle/deprecated") => {
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
                .unwrap_or(100);
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

        (Method::Post, "/lifecycle/cycle") => {
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

        (Method::Post, "/lifecycle/promote") => {
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

        // ── Namespaces ──────────────────────────────────────────────────
        (Method::Get, "/namespaces") => {
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

        // ── Namespace Rename/Merge (#1181) ──────────────────────────────
        (Method::Post, "/namespaces/rename") => {
            match read_body::<NamespaceRenameRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.rename_namespace(&req_data.from, &req_data.to) {
                    Ok(result) => ctx.ok_response_for(req, &result),
                    Err(uteke_core::Error::Validation(msg)) => {
                        ctx.error_response_for(req, 400, msg)
                    }
                    Err(e) => {
                        error!("Namespace rename error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        // ── Namespace Delete with explicit strategy (#1181) ─────────────
        (Method::Post, "/namespaces/delete") => {
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

        // ── Recent (#528) ──────────────────────────────────────────────
        (Method::Get, "/recent") => {
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
        (Method::Get, "/graph") => {
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

                let conn = uteke.graph_store();
                let gs = uteke_core::graph::GraphStore::new(conn);

                // Resolve an input ID to its graph node without creating
                // anything: memory-linked nodes first, then explicit node IDs
                // (clients may pass IDs from GET /graph) (#1180).
                let resolve_node = |id: &str| -> Result<Option<String>, uteke_core::Error> {
                    if let Some(nid) = gs.node_id_for_memory(id)? {
                        return Ok(Some(nid));
                    }
                    gs.get_node(id).map(|n| n.map(|node| node.id))
                };

                // Validate both sides exist as memories or graph nodes —
                // memory IDs are ensured into nodes below (#1180, relaxes the
                // memory-only #542 check that made every valid call 500).
                for (role, id) in [("Source", &req_data.source), ("Target", &req_data.target)] {
                    if matches!(uteke.get_by_id(id), Ok(Some(_))) {
                        continue;
                    }
                    match resolve_node(id) {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            return ctx.error_response_for(
                                req,
                                404,
                                format!("{role} memory not found: {id}"),
                            );
                        }
                        Err(e) => {
                            error!("Graph resolve error: {e}");
                            return ctx.error_response_for(req, 500, "Internal server error");
                        }
                    }
                }

                let relation = req_data.edge_type.as_deref().unwrap_or("related");
                let weight = req_data.weight.unwrap_or(1.0);

                // Map memory IDs → graph node IDs before insertion (#1180).
                // `graph_edges.source_id/target_id` carry FKs to
                // `graph_nodes(id)`, so inserting raw memory IDs violates the
                // FK and always 500s. Validation above guarantees each ID is
                // a memory (node gets ensured) or an existing node.
                let ensure = |id: &str| -> Result<String, uteke_core::Error> {
                    match resolve_node(id)? {
                        Some(nid) => Ok(nid),
                        None => gs.ensure_node_for_memory(id),
                    }
                };
                let (src_node, tgt_node) =
                    match (ensure(&req_data.source), ensure(&req_data.target)) {
                        (Ok(s), Ok(t)) => (s, t),
                        (Err(e), _) | (_, Err(e)) => {
                            error!("Graph ensure_node error: {e}");
                            return ctx.error_response_for(req, 500, "Internal server error");
                        }
                    };

                match gs.add_edge(&src_node, &tgt_node, relation, weight) {
                    Ok(()) => ctx.ok_response_for(
                        req,
                        &serde_json::json!({
                            "ok": true,
                            "source_node": src_node,
                            "target_node": tgt_node,
                        }),
                    ),
                    Err(e) => {
                        error!("Graph add_edge error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Graph Mutation: Remove Edge (#542) ────────────────────────────
        (Method::Delete, "/graph/edge") => {
            let query = path.split('?').nth(1).unwrap_or("");
            let source = parse_query_param(query, "source");
            let target = parse_query_param(query, "target");

            match (&source, &target) {
                (Some(src), Some(tgt)) => {
                    let conn = uteke.graph_store();
                    let gs = uteke_core::graph::GraphStore::new(conn);
                    // Accept either memory IDs or graph node IDs (#1180) —
                    // resolve both sides to node IDs before deleting.
                    let resolve = |id: &str| -> Result<Option<String>, uteke_core::Error> {
                        if let Some(nid) = gs.node_id_for_memory(id)? {
                            return Ok(Some(nid));
                        }
                        gs.get_node(id).map(|n| n.map(|node| node.id))
                    };
                    let (src_node, tgt_node) = match (resolve(src), resolve(tgt)) {
                        (Ok(Some(s)), Ok(Some(t))) => (s, t),
                        (Ok(_), Ok(_)) => {
                            return ctx.error_response_for(
                                req,
                                404,
                                format!("Edge not found: {src} -> {tgt}"),
                            );
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            error!("Graph node resolve error: {e}");
                            return ctx.error_response_for(req, 500, "Internal server error");
                        }
                    };
                    match gs.remove_edge(&src_node, &tgt_node) {
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

        // ── Room Summary Document ────────────────────────────────────────
        (Method::Post, "/room/summary-document") => {
            #[derive(Deserialize)]
            struct RoomSummaryDocumentRequest {
                room_id: String,
            }
            match read_body::<RoomSummaryDocumentRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.room_summary_document(&req_data.room_id) {
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

        // ── Room Consolidation (#1088) ───────────────────────────────────
        // Dry-run by default; `apply: true` executes with a hard budget cap.
        // Blocked for read-only tokens (not in read_only_post_paths).
        (Method::Post, "/room/consolidate") => {
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
                            &uteke,
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
                        match uteke_core::consolidation_api::plan_room(&uteke, &req_data.room_id) {
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

        // ── DEPRECATED: POST /room/document → /room/summary-document (#735)
        (Method::Post, "/room/document") => {
            warn!(
                "DEPRECATED: POST /room/document is renamed to POST /room/summary-document (see #735)"
            );
            #[derive(Deserialize)]
            struct RoomDocumentRequest {
                room_id: String,
            }
            match read_body::<RoomDocumentRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.room_summary_document(&req_data.room_id) {
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
        (Method::Get, "/memory") => {
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

        // ── Memory Pin/Unpin (#660) ──────────────────────────────────────
        (Method::Post, "/memory/pin") => match read_body::<MemoryPinRequest>(req.as_reader()) {
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
                    Ok(false) => ctx.error_response_for(
                        req,
                        404,
                        format!("Memory not found: {}", req_data.id),
                    ),
                    Err(e) => {
                        error!("Pin/unpin error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                }
            }
            Err(e) => ctx.error_response_for(req, 400, e),
        },

        // ── Memory Set Importance (#660) ──────────────────────────────────
        (Method::Post, "/memory/importance") => {
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
                        Ok(false) => ctx.error_response_for(
                            req,
                            404,
                            format!("Memory not found: {}", req_data.id),
                        ),
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

        // ── Memory Feedback (Trust Scoring) (#718) ────────────────────
        (Method::Post, "/memory/feedback") => {
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

        // ── Update memory by ID (PUT /memory, #659) ──────────────────
        (Method::Put, "/memory") => match read_body::<MemoryUpdateRequest>(req.as_reader()) {
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
                match uteke.update_memory(
                    &req_data.id,
                    req_data.content.as_deref(),
                    tag_slice,
                    req_data.metadata.as_ref(),
                    req_data.importance,
                    req_data.pinned,
                    req_data.memory_type.as_deref(),
                ) {
                    Ok(true) => {
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
                        ctx.ok_response_for(req, &serde_json::json!({"updated": req_data.id}))
                    }
                    Ok(false) => ctx.error_response_for(
                        req,
                        404,
                        format!("Memory not found: {}", req_data.id),
                    ),
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
        },

        // ── Room Memories (chronological listing — GET /room/memories) ────
        (Method::Get, "/room/memories") => {
            let query_str = path.split('?').nth(1);
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

        // ── Room Recall (semantic with optional fallback to chronological) ──
        (Method::Post, "/room/recall") => match read_body::<RoomRecallRequest>(req.as_reader()) {
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
                    match uteke.recall_room(
                        &req_data.room_id,
                        req_data.author.as_deref(),
                        fetch_limit,
                    ) {
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

        // POST /room/rename — rename a room, moving all member/document
        // links in one transaction (#1202)
        (Method::Post, "/room/rename") => {
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
        (Method::Post, "/room/update") => {
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
                        Ok(Some(room)) => {
                            ctx.ok_response_for(req, &serde_json::json!({ "room": room }))
                        }
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
        (Method::Post, "/room/memory/move") => {
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
        (Method::Post, "/room/remember") => {
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
                        Ok(id) => ctx.ok_response_for(
                            req,
                            &serde_json::json!({
                                "id": id,
                                "room_id": req_data.room_id,
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

        // ── Room ↔ Document junction (#689) ──────────────────────────────
        // POST /room/document/list — list documents linked to a room
        (Method::Post, "/room/document/list") => {
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
        (Method::Put, "/room/document/add") => {
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
        (Method::Delete, "/room/document/remove") => {
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
        (Method::Post, "/doc/room/list") => {
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

        (Method::Delete, "/room/delete") => {
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
                    None,
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
                Ok(id_or_slug) => match uteke.doc_get(id_or_slug) {
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
        },

        // ── Document: List ─────────────────────────────────────────────
        (Method::Post, "/doc/list") => match read_body::<DocListParams>(req.as_reader()) {
            Ok(params) => {
                let result = if params.roots_only {
                    uteke.doc_list_roots(params.limit)
                } else if let Some(ref parent) = params.parent {
                    uteke.doc_list_children(parent, params.limit)
                } else {
                    uteke.doc_list(params.limit)
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
                match uteke.doc_search(&req_data.query, req_data.limit, &req_data.mode) {
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
                    let new_sort_order = req_data.new_sort_order;
                    match uteke.doc_move(id_or_slug, new_parent, new_sort_order) {
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
        (Method::Delete, "/doc/delete") => {
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

        // ── Cross-entity references (#689) ───────────────────────────────
        // POST /memory/doc-refs — get document slugs referenced by a memory
        (Method::Post, "/memory/doc-refs") => {
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

        // POST /doc/mem-refs — get memory IDs that reference a document
        (Method::Post, "/doc/mem-refs") => {
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

        // ── Tags: List with counts ───────────────────────────────────────
        (Method::Get, "/tags") => {
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

        // ── Provenance chain (#1172) ─────────────────────────────────────
        (Method::Get, "/provenance") => {
            let query = path.split('?').nth(1).unwrap_or("");
            let id = match parse_query_param(query, "id") {
                Some(id) => id,
                None => return ctx.error_response_for(req, 400, "Missing 'id' query parameter"),
            };
            match uteke.provenance(&id) {
                Ok(Some(report)) => ctx.ok_response_for(req, &report),
                Ok(None) => ctx.error_response_for(req, 404, format!("Memory not found: {id}")),
                Err(e) => {
                    error!("Provenance error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Contradiction resolutions ledger (#1172) ────────────────────
        (Method::Get, "/contradictions") => {
            let query = path.split('?').nth(1).unwrap_or("");
            let ns = parse_query_namespace(&path);
            let limit = parse_query_param(query, "limit")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(50);
            match uteke.contradiction_resolutions(ns.as_deref(), limit) {
                Ok(resolutions) => ctx.ok_response_for(req, &resolutions),
                Err(e) => {
                    error!("Contradiction resolutions error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }

        // ── Undo a contradiction resolution (#1172) ─────────────────────
        (Method::Post, "/contradictions/undo") => {
            #[derive(Deserialize)]
            struct ContradictionUndoRequest {
                /// ID of the retired (superseded) memory to restore.
                id: String,
            }
            match read_body::<ContradictionUndoRequest>(req.as_reader()) {
                Ok(req_data) => match uteke.undo_supersession(&req_data.id) {
                    Ok(Some(winner)) => ctx.ok_response_for(
                        req,
                        &serde_json::json!({
                            "undone": true,
                            "restored": req_data.id,
                            "was_superseded_by": winner,
                        }),
                    ),
                    Ok(None) => ctx.error_response_for(
                        req,
                        404,
                        format!("No supersession found for memory: {}", req_data.id),
                    ),
                    Err(e) => {
                        error!("Contradiction undo error: {e}");
                        ctx.error_response_for(req, 500, "Internal server error")
                    }
                },
                Err(e) => ctx.error_response_for(req, 400, e),
            }
        }

        // ── Timeline ─────────────────────────────────────────────────────
        (Method::Get, "/timeline") => {
            let query = path.split('?').nth(1).unwrap_or("");
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
        (Method::Get, "/edges") => {
            let query = path.split('?').nth(1).unwrap_or("");
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
                    &serde_json::json!({
                        "facts": facts.iter().map(|f| &f.content).collect::<Vec<_>>(),
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
        (Method::Get, "/export") => {
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

        // Per-pair dedup control (#1076): caller picks the survivor.
        (Method::Post, "/consolidate/pair") => {
            match read_body::<ConsolidatePairRequest>(req.as_reader()) {
                Ok(req_data) => {
                    let result = uteke.consolidate_pair(
                        &req_data.id_keep,
                        &req_data.id_remove,
                        req_data.hard,
                    );
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
                        );
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
        mode: base.mode,
        model: req_model.map(String::from).unwrap_or(base.model),
        api_key: req_api_key.map(String::from).unwrap_or(base.api_key),
        base_url: base.base_url,
        endpoint_path: base.endpoint_path,
        max_facts: req_max_facts.unwrap_or(base.max_facts),
    }
}

/// Point-in-time predicate for room time-travel (#1082).
/// Mirrors the core `recall_at_time` temporal rules: memory must have been
/// created at or before `pit`, not yet invalidated (valid_until), not
/// deprecated, and valid_from must not be in the future relative to `pit`.
///
/// Known limitation (#1086): the schema stores no deprecation timestamp,
/// so a memory deprecated *after* `pit` is also excluded here — time-travel
/// treats `deprecated` as "never existed". Same rule as core `recall_at_time`.
fn memory_exists_at(
    memory: &uteke_core::memory::types::Memory,
    pit: chrono::DateTime<chrono::Utc>,
) -> bool {
    if memory.created_at > pit {
        return false;
    }
    if let Some(valid_until) = memory.valid_until {
        if valid_until <= pit {
            return false;
        }
    }
    if memory.deprecated {
        // Deprecated before the point-in-time → did not exist then;
        // deprecated after it → existed (#1086). Matches core semantics:
        // NULL deprecated_at is treated as "deprecated after the pit"
        // (the v17 migration backfills a timestamp onto every deprecated
        // row, so NULL + deprecated should not occur in practice).
        let gone_by_then = match memory.deprecated_at {
            Some(dep_at) => dep_at <= pit,
            None => false,
        };
        if gone_by_then {
            return false;
        }
    }
    if let Some(valid_from) = memory.valid_from {
        if valid_from > pit {
            return false;
        }
    }
    true
}

/// Apply the time-travel predicate to chronological room recall results
/// (no-query path), preserving chronological order (#1082).
fn filter_room_memories_at_time(
    memories: Vec<uteke_core::memory::types::Memory>,
    pit: chrono::DateTime<chrono::Utc>,
) -> Vec<uteke_core::memory::types::Memory> {
    memories
        .into_iter()
        .filter(|m| memory_exists_at(m, pit))
        .collect()
}

#[cfg(test)]
mod room_recall_at_tests {
    use super::*;
    use chrono::Utc;

    fn mem(created_offset_secs: i64) -> uteke_core::memory::types::Memory {
        uteke_core::memory::types::Memory {
            id: uuid::Uuid::new_v4().to_string(),
            content: format!("m{created_offset_secs}"),
            embedding: Vec::new(),
            tags: Vec::new(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            created_at: Utc::now() + chrono::Duration::seconds(created_offset_secs),
            updated_at: Utc::now(),
            namespace: "default".into(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".into(),
            importance: 0.5,
            pinned: false,
            content_type: "text".into(),
            slug: None,
            source: None,
            source_type: "user".into(),
            author_type: "agent".into(),
        }
    }

    #[test]
    fn at_time_excludes_future_and_invalidated() {
        let now = Utc::now();
        let old = mem(-3600); // created an hour ago — existed at `now`
        let future = mem(3600); // created an hour from now — must be excluded
        let mut invalidated = mem(-7200);
        invalidated.valid_until = Some(now - chrono::Duration::seconds(60)); // expired before `now`
        let mut not_yet_valid = mem(-7200);
        not_yet_valid.valid_from = Some(now + chrono::Duration::seconds(60));

        assert!(memory_exists_at(&old, now));
        assert!(!memory_exists_at(&future, now));
        assert!(!memory_exists_at(&invalidated, now));
        assert!(!memory_exists_at(&not_yet_valid, now));
    }

    #[test]
    fn at_time_excludes_deprecated() {
        let now = Utc::now();
        let mut dep = mem(-60);
        dep.deprecated = true;
        dep.deprecated_at = Some(now - chrono::Duration::seconds(30)); // deprecated before pit
        assert!(!memory_exists_at(&dep, now));
    }

    #[test]
    fn at_time_includes_deprecated_after_pit() {
        // #1086: deprecated AFTER the pit — the memory existed then.
        let now = Utc::now();
        let mut dep = mem(-60);
        dep.deprecated = true;
        dep.deprecated_at = Some(now + chrono::Duration::seconds(30)); // deprecated after pit
        assert!(memory_exists_at(&dep, now));
    }

    #[test]
    fn at_time_boundary_inclusive_created_at() {
        let exact = mem(0);
        // created_at == pit: memory existed at that instant (inclusive, matches core recall_at_time rule `> pit → reject`)
        assert!(memory_exists_at(&exact, exact.created_at));
    }

    #[test]
    fn filter_room_memories_preserves_order() {
        let now = Utc::now();
        let older = mem(-100);
        let newer = mem(-50);
        let future = mem(100);
        let out = filter_room_memories_at_time(vec![older, newer, future], now);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].content, "m-100");
        assert_eq!(out[1].content, "m-50");
    }

    #[test]
    fn room_recall_request_deserializes_at() {
        // Regression for #1082: `at` must be a recognized field, not silently dropped.
        let req: RoomRecallRequest =
            serde_json::from_str(r#"{"room_id":"r1","limit":3,"at":"2020-01-01T00:00:00Z"}"#)
                .expect("at field must deserialize");
        assert_eq!(req.at.as_deref(), Some("2020-01-01T00:00:00Z"));
        assert_eq!(req.room_id, "r1");
        // Omitted → None (backwards compatible)
        let req: RoomRecallRequest =
            serde_json::from_str(r#"{"room_id":"r1"}"#).expect("no-at body must parse");
        assert!(req.at.is_none());
    }

    /// #1076: ConsolidatePairRequest deserialization — happy path + defaults.
    #[test]
    fn consolidate_pair_request_parses() {
        let req: ConsolidatePairRequest =
            serde_json::from_str(r#"{"id_keep":"abc","id_remove":"def","hard":true}"#)
                .expect("pair body must parse");
        assert_eq!(req.id_keep, "abc");
        assert_eq!(req.id_remove, "def");
        assert!(req.hard);

        // hard defaults to false
        let req: ConsolidatePairRequest =
            serde_json::from_str(r#"{"id_keep":"abc","id_remove":"def"}"#).unwrap();
        assert!(!req.hard);

        // missing id_remove → 400 at deserialize
        assert!(serde_json::from_str::<ConsolidatePairRequest>(r#"{"id_keep":"abc"}"#).is_err());
    }

    // ── /graph/edge memory-ID contract (#1180) ─────────────────────────

    use tiny_http::TestRequest;

    struct GraphEdgeApp {
        uteke: Mutex<Uteke>,
    }

    impl GraphEdgeApp {
        fn new() -> Self {
            // No embedder: these endpoints are storage-only, and tests must
            // run in CI builds without the ONNX runtime lib.
            Self {
                uteke: Mutex::new(
                    Uteke::open_with_backend(":memory:", None)
                        .expect("open in-memory uteke without embedder"),
                ),
            }
        }

        fn call(
            &self,
            method: Method,
            url: &str,
            body: Option<String>,
        ) -> (u16, serde_json::Value) {
            let mut req = match body {
                Some(b) => {
                    let leaked: &'static str = Box::leak(b.into_boxed_str());
                    TestRequest::new()
                        .with_method(method)
                        .with_path(url)
                        .with_body(leaked)
                        .into()
                }
                None => TestRequest::new().with_method(method).with_path(url).into(),
            };
            let ctx = ReqCtx {
                auth_token_hash: None,
                read_only_token_hash: None,
                cors_origins: Vec::new(),
                recall_config: None,
                extraction_config: None,
            };
            let resp = route(&self.uteke, &ctx, &mut req);
            let status = resp.status_code().0;
            let bytes = resp.into_reader().into_inner();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }

        fn remember(&self, content: &str) -> String {
            let body = serde_json::json!({ "content": content }).to_string();
            let (status, resp) = self.call(Method::Post, "/remember", Some(body));
            assert_eq!(status, 200, "remember must succeed: {resp}");
            resp["id"]
                .as_str()
                .unwrap_or_else(|| panic!("remember response must carry id: {resp}"))
                .to_string()
        }

        fn add_edge(&self, source: &str, target: &str) -> (u16, serde_json::Value) {
            let body = serde_json::json!({ "source": source, "target": target }).to_string();
            self.call(Method::Post, "/graph/edge", Some(body))
        }

        fn delete_edge(&self, source: &str, target: &str) -> (u16, serde_json::Value) {
            let url = format!("/graph/edge?source={source}&target={target}");
            self.call(Method::Delete, &url, None)
        }
    }

    #[test]
    fn graph_edge_post_with_memory_ids_returns_ok() {
        let app = GraphEdgeApp::new();
        let id1 = app.remember("Uteke is a local-first memory engine");
        let id2 = app.remember("Corin renders the memory graph");

        let (status, resp) = app.add_edge(&id1, &id2);
        assert_eq!(
            status, 200,
            "valid memory IDs must not 500 (FK mismatch #1180): {resp}"
        );
        assert_eq!(resp["ok"], serde_json::json!(true));
    }

    #[test]
    fn graph_edge_post_resolves_memory_ids_to_nodes() {
        let app = GraphEdgeApp::new();
        let id1 = app.remember("alpha memory");
        let id2 = app.remember("beta memory");

        let (status, resp) = app.add_edge(&id1, &id2);
        assert_eq!(status, 200, "{resp}");

        let src_node = resp["source_node"].as_str().expect("source_node id");
        let tgt_node = resp["target_node"].as_str().expect("target_node id");

        // The resolved nodes must exist in GET /graph and carry the memory
        // link, so visualization can map edges back to memories.
        let (_, graph) = app.call(Method::Get, "/graph", None);
        let nodes = graph["nodes"].as_array().expect("nodes array");
        assert!(
            nodes.iter().any(|n| n["id"] == serde_json::json!(src_node)
                && n["memory_id"] == serde_json::json!(id1)),
            "source node must link back to memory {id1}: {graph}"
        );
        assert!(
            nodes.iter().any(|n| n["id"] == serde_json::json!(tgt_node)
                && n["memory_id"] == serde_json::json!(id2)),
            "target node must link back to memory {id2}: {graph}"
        );

        // The edge itself must be visible in the graph payload.
        let edges = graph["edges"].as_array().expect("edges array");
        assert!(
            edges
                .iter()
                .any(|e| e["source_id"] == serde_json::json!(src_node)
                    && e["target_id"] == serde_json::json!(tgt_node)
                    && e["relation"] == serde_json::json!("related")),
            "edge must appear in GET /graph: {graph}"
        );
    }

    #[test]
    fn graph_edge_post_accepts_graph_node_ids_too() {
        let app = GraphEdgeApp::new();
        let id1 = app.remember("source memory");
        let id2 = app.remember("target memory");

        let (_, first) = app.add_edge(&id1, &id2);
        let src_node = first["source_node"].as_str().unwrap().to_string();

        // A client holding IDs from GET /graph must still be able to link a
        // graph node directly (e.g. entity nodes without a memory link).
        let (status, resp) = app.add_edge(&src_node, &id2);
        assert_eq!(status, 200, "node IDs must stay accepted: {resp}");
    }

    #[test]
    fn graph_edge_post_with_unknown_memory_returns_404() {
        let app = GraphEdgeApp::new();
        let ghost1 = "00000000-0000-4000-8000-000000000000";
        let ghost2 = "11111111-1111-4111-8111-111111111111";

        let (status, resp) = app.add_edge(ghost1, ghost2);
        assert_eq!(status, 404, "unknown source must 404: {resp}");
    }

    #[test]
    fn graph_edge_post_self_loop_rejected() {
        let app = GraphEdgeApp::new();
        let id1 = app.remember("self loop probe");

        let (status, _) = app.add_edge(&id1, &id1);
        assert_eq!(status, 400, "self-loop must stay rejected");
    }

    #[test]
    fn graph_edge_delete_accepts_memory_ids() {
        let app = GraphEdgeApp::new();
        let id1 = app.remember("edge delete probe a");
        let id2 = app.remember("edge delete probe b");

        let (status, resp) = app.add_edge(&id1, &id2);
        assert_eq!(status, 200, "{resp}");

        // DELETE with the same memory IDs must resolve back to the nodes.
        let (status, resp) = app.delete_edge(&id1, &id2);
        assert_eq!(status, 200, "delete by memory IDs: {resp}");
        assert_eq!(resp["ok"], serde_json::json!(true));

        // Second delete: edge gone → 404, not 500.
        let (status, resp) = app.delete_edge(&id1, &id2);
        assert_eq!(status, 404, "deleting a removed edge must 404: {resp}");
    }

    #[test]
    fn graph_edge_delete_accepts_node_ids_and_unknown_ids() {
        let app = GraphEdgeApp::new();
        let id1 = app.remember("node id delete probe a");
        let id2 = app.remember("node id delete probe b");

        let (_, resp) = app.add_edge(&id1, &id2);
        let src_node = resp["source_node"].as_str().unwrap().to_string();
        let tgt_node = resp["target_node"].as_str().unwrap().to_string();

        // Node IDs (from GET /graph payloads) must keep working.
        let (status, resp) = app.delete_edge(&src_node, &tgt_node);
        assert_eq!(status, 200, "delete by node IDs: {resp}");

        // Fully unknown IDs must 404 — never 500.
        let (status, resp) = app.delete_edge("ghost-a", "ghost-b");
        assert_eq!(status, 404, "unknown IDs must 404: {resp}");
    }

    #[test]
    fn graph_edge_delete_requires_both_params() {
        let app = GraphEdgeApp::new();
        // URL built separately so the api_registry route scanner does not
        // mistake this test call for a handler route arm.
        let url = "/graph/edge?source=only-one";
        let (status, _) = app.call(Method::Delete, url, None);
        assert_eq!(status, 400, "missing target param must 400");
    }

    // ── Namespace management API (#1181) ───────────────────────────────

    struct NamespaceApp {
        uteke: Mutex<Uteke>,
    }

    impl NamespaceApp {
        fn new() -> Self {
            Self {
                uteke: Mutex::new(
                    Uteke::open_with_backend(":memory:", None)
                        .expect("open in-memory uteke without embedder"),
                ),
            }
        }

        fn call(
            &self,
            method: Method,
            url: &str,
            body: Option<String>,
        ) -> (u16, serde_json::Value) {
            let mut req = match body {
                Some(b) => {
                    let leaked: &'static str = Box::leak(b.into_boxed_str());
                    TestRequest::new()
                        .with_method(method)
                        .with_path(url)
                        .with_body(leaked)
                        .into()
                }
                None => TestRequest::new().with_method(method).with_path(url).into(),
            };
            let ctx = ReqCtx {
                auth_token_hash: None,
                read_only_token_hash: None,
                cors_origins: Vec::new(),
                recall_config: None,
                extraction_config: None,
            };
            let resp = route(&self.uteke, &ctx, &mut req);
            let status = resp.status_code().0;
            let bytes = resp.into_reader().into_inner();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }

        fn remember_in(&self, content: &str, namespace: &str) -> String {
            let body =
                serde_json::json!({ "content": content, "namespace": namespace }).to_string();
            let (status, resp) = self.call(Method::Post, "/remember", Some(body));
            assert_eq!(status, 200, "remember must succeed: {resp}");
            resp["id"].as_str().expect("id").to_string()
        }
    }

    #[test]
    fn namespace_put_memory_moves_namespace() {
        let app = NamespaceApp::new();
        let id = app.remember_in("to be moved", "alpha");

        let body = serde_json::json!({ "id": id, "namespace": "beta" }).to_string();
        let (status, resp) = app.call(Method::Put, "/memory", Some(body));
        assert_eq!(status, 200, "PUT with namespace must succeed: {resp}");

        // The move is visible via GET /memory.
        let get_url = format!("/memory?id={id}");
        let (status, resp) = app.call(Method::Get, &get_url, None);
        assert_eq!(status, 200);
        assert_eq!(resp["namespace"], serde_json::json!("beta"));

        // The old name vanishes from listings (derived view).
        let (_, list) = app.call(Method::Get, "/namespaces", None);
        let names: Vec<&str> = list
            .as_array()
            .expect("namespaces array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(!names.contains(&"alpha"), "old namespace gone: {list:?}");
        assert!(names.contains(&"beta"), "{list:?}");
    }

    #[test]
    fn namespace_rename_and_merge_endpoint() {
        let app = NamespaceApp::new();
        app.remember_in("one", "old");
        app.remember_in("two", "old");
        app.remember_in("three", "existing");

        let body = serde_json::json!({ "from": "old", "to": "existing" }).to_string();
        let (status, resp) = app.call(Method::Post, "/namespaces/rename", Some(body));
        assert_eq!(status, 200, "{resp}");
        assert_eq!(resp["moved"], serde_json::json!(2));
        assert_eq!(resp["target_existed"], serde_json::json!(true));
        assert_eq!(resp["from"], serde_json::json!("old"));
        assert_eq!(resp["to"], serde_json::json!("existing"));
    }

    #[test]
    fn namespace_delete_refuse_returns_409() {
        let app = NamespaceApp::new();
        app.remember_in("keep me", "busy");

        let body = serde_json::json!({ "name": "busy" }).to_string();
        let (status, resp) = app.call(Method::Post, "/namespaces/delete", Some(body));
        assert_eq!(
            status, 409,
            "refuse on non-empty namespace must 409: {resp}"
        );
        assert!(
            resp["error"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .contains("refus")
        );
    }

    #[test]
    fn room_rename_update_move_endpoints_roundtrip() {
        let app = NamespaceApp::new();

        // Create rooms + one room memory.
        let body = serde_json::json!({ "room_id": "ws", "namespace": "default" }).to_string();
        let (status, resp) = app.call(Method::Post, "/room/create", Some(body));
        assert_eq!(status, 200, "{resp}");
        let body = serde_json::json!({ "room_id": "ws-3", "namespace": "default" }).to_string();
        let (status, resp) = app.call(Method::Post, "/room/create", Some(body));
        assert_eq!(status, 200, "{resp}");

        let body = serde_json::json!({
            "content": "room note",
            "tags": [],
            "room_id": "ws",
            "author": "tester"
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/room/remember", Some(body));
        assert_eq!(status, 200, "{resp}");
        let memory_id = resp["id"].as_str().expect("memory id").to_string();

        // Rename ws → ws-2.
        let body = serde_json::json!({ "from": "ws", "to": "ws-2" }).to_string();
        let (status, resp) = app.call(Method::Post, "/room/rename", Some(body));
        assert_eq!(status, 200, "{resp}");
        assert_eq!(resp["renamed"], serde_json::json!("ws"));
        assert_eq!(resp["room"]["id"], serde_json::json!("ws-2"));

        // Update title/description on ws-2.
        let body = serde_json::json!({
            "room_id": "ws-2",
            "title": "Workspace",
            "description": "Renamed workspace"
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/room/update", Some(body));
        assert_eq!(status, 200, "{resp}");
        assert_eq!(
            resp["room"]["description"],
            serde_json::json!("Renamed workspace")
        );

        // Move the memory ws-2 → ws-3.
        let body = serde_json::json!({
            "memory_id": memory_id,
            "from_room": "ws-2",
            "to_room": "ws-3"
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/room/memory/move", Some(body.clone()));
        assert_eq!(status, 200, "{resp}");
        assert_eq!(resp["moved"], serde_json::json!(memory_id));

        // Moving again → 404 (no link left in source).
        let (status, resp) = app.call(Method::Post, "/room/memory/move", Some(body));
        assert_eq!(status, 404, "{resp}");
        assert!(
            resp["error"]
                .as_str()
                .unwrap_or_default()
                .contains("no link")
        );
    }

    #[test]
    fn room_update_missing_room_returns_404() {
        let app = NamespaceApp::new();
        let body = serde_json::json!({ "room_id": "ghost", "title": "x" }).to_string();
        let (status, resp) = app.call(Method::Post, "/room/update", Some(body));
        assert_eq!(status, 404, "{resp}");
    }

    #[test]
    fn namespace_delete_merge_removes_namespace() {
        let app = NamespaceApp::new();
        app.remember_in("m one", "doomed");
        app.remember_in("m two", "doomed");

        let body = serde_json::json!({ "name": "doomed", "strategy": "merge", "target": "safe" })
            .to_string();
        let (status, resp) = app.call(Method::Post, "/namespaces/delete", Some(body));
        assert_eq!(status, 200, "{resp}");
        assert_eq!(resp["affected"], serde_json::json!(2));
        assert_eq!(resp["empty"], serde_json::json!(true));

        let (_, list) = app.call(Method::Get, "/namespaces", None);
        let names: Vec<&str> = list
            .as_array()
            .expect("namespaces array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(!names.contains(&"doomed"), "{list:?}");
        assert!(names.contains(&"safe"), "{list:?}");
    }

    #[test]
    fn namespace_with_counts_reports_lifecycle_split() {
        let app = NamespaceApp::new();
        let id = app.remember_in("ghost food", "ghosted");
        let _ = id;
        let body = serde_json::json!({ "name": "ghosted", "strategy": "deprecate" }).to_string();
        let (status, resp) = app.call(Method::Post, "/namespaces/delete", Some(body));
        assert_eq!(status, 200, "{resp}");
        assert_eq!(resp["affected"], serde_json::json!(1));
        assert_eq!(resp["empty"], serde_json::json!(false));

        // URL built separately so the api_registry route scanner does not
        // mistake this test call for a handler route arm.
        let counts_url = "/namespaces?with_counts=true";
        let (_, counts) = app.call(Method::Get, counts_url, None);
        let arr = counts.as_array().expect("counts array");
        let ghost = arr
            .iter()
            .find(|n| n["name"] == serde_json::json!("ghosted"))
            .expect("ghost namespace stays listed");
        assert_eq!(ghost["active"], serde_json::json!(0));
        assert_eq!(ghost["deprecated"], serde_json::json!(1));
        assert_eq!(ghost["count"], serde_json::json!(1));
    }

    // ── Provenance API (#1172) ─────────────────────────────────────────

    #[test]
    fn provenance_endpoint_reports_chain_and_hash() {
        let app = NamespaceApp::new();
        let id = app.remember_in("provenance surface probe", "audit");

        let get_url = format!("/provenance?id={id}");
        let (status, report) = app.call(Method::Get, &get_url, None);
        assert_eq!(status, 200, "{report}");
        assert_eq!(report["id"], serde_json::json!(id));
        assert_eq!(report["namespace"], serde_json::json!("audit"));
        assert_eq!(report["author_type"], serde_json::json!("agent"));
        assert_eq!(report["trust_tier"], serde_json::json!("agent"));
        assert_eq!(report["source_hash"], report["content_hash_now"]);
        let events = report["events"].as_array().expect("events array");
        assert!(
            events
                .iter()
                .any(|e| e["event_type"] == serde_json::json!("created")),
            "created event must be in the chain: {report}"
        );

        // Unknown ID → 404.
        let (status, _) = app.call(
            Method::Get,
            "/provenance?id=00000000-0000-4000-8000-000000000000",
            None,
        );
        assert_eq!(status, 404);

        // Missing param → 400.
        let (status, _) = app.call(Method::Get, "/provenance", None);
        assert_eq!(status, 400);
    }
}

// ── Contradiction ledger API (#1172 Fase 2) ─────────────────────────

#[cfg(test)]
mod contradiction_api_tests {
    use super::*;
    use tiny_http::TestRequest;

    struct ContradictionApp {
        uteke: Mutex<Uteke>,
    }

    impl ContradictionApp {
        fn new() -> Self {
            // No embedder: storage-only endpoints, CI-safe without ONNX.
            Self {
                uteke: Mutex::new(
                    Uteke::open_with_backend(":memory:", None)
                        .expect("open in-memory uteke without embedder"),
                ),
            }
        }

        fn call(
            &self,
            method: Method,
            url: &str,
            body: Option<String>,
        ) -> (u16, serde_json::Value) {
            let mut req = match body {
                Some(b) => {
                    let leaked: &'static str = Box::leak(b.into_boxed_str());
                    TestRequest::new()
                        .with_method(method)
                        .with_path(url)
                        .with_body(leaked)
                        .into()
                }
                None => TestRequest::new().with_method(method).with_path(url).into(),
            };
            let ctx = ReqCtx {
                auth_token_hash: None,
                read_only_token_hash: None,
                cors_origins: Vec::new(),
                recall_config: None,
                extraction_config: None,
            };
            let resp = route(&self.uteke, &ctx, &mut req);
            let status = resp.status_code().0;
            let bytes = resp.into_reader().into_inner();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }

        fn remember_in(&self, content: &str, namespace: &str) -> String {
            let body =
                serde_json::json!({ "content": content, "namespace": namespace }).to_string();
            let (status, resp) = self.call(Method::Post, "/remember", Some(body));
            assert_eq!(status, 200, "remember must succeed: {resp}");
            resp["id"]
                .as_str()
                .unwrap_or_else(|| panic!("remember response must carry id: {resp}"))
                .to_string()
        }

        fn supersede(&self, old: &str, new: &str) {
            // Setup via the core API — the server has no supersede route;
            // the surfaces under test are the ledger + undo endpoints.
            self.uteke
                .lock()
                .expect("poisoned mutex")
                .supersede(old, new, None)
                .expect("supersede must succeed");
        }
    }

    #[test]
    fn ledger_lists_superseded_and_undo_restores() {
        let app = ContradictionApp::new();
        let old = app.remember_in("deploy target is VM-1", "ops");
        let new = app.remember_in("deploy target is VM-2", "ops");
        app.supersede(&old, &new);

        // URL built separately so the api_registry route scanner does not
        // mistake this test call for a handler route arm.
        let list_url = "/contradictions?namespace=ops";
        let (status, ledger) = app.call(Method::Get, list_url, None);
        assert_eq!(status, 200, "{ledger}");
        let entries = ledger.as_array().expect("ledger array");
        assert_eq!(
            entries.len(),
            1,
            "one supersession must be listed: {ledger}"
        );
        assert_eq!(entries[0]["id"], serde_json::json!(old));
        let reason = entries[0]["deprecate_reason"].as_str().unwrap_or("");
        assert!(
            reason.contains("superseded"),
            "reason must mention superseded: {reason}"
        );

        // Undo via POST body.
        let undo_body = serde_json::json!({ "id": old }).to_string();
        let (status, resp) = app.call(Method::Post, "/contradictions/undo", Some(undo_body));
        assert_eq!(status, 200, "{resp}");
        assert_eq!(resp["undone"], serde_json::json!(true));
        assert_eq!(resp["restored"], serde_json::json!(old));
        assert_eq!(resp["was_superseded_by"], serde_json::json!(new));

        // Ledger is clean after the undo.
        let (status, ledger) = app.call(Method::Get, list_url, None);
        assert_eq!(status, 200);
        assert_eq!(
            ledger.as_array().expect("ledger array").len(),
            0,
            "undo must clear the ledger: {ledger}"
        );
    }

    #[test]
    fn undo_unknown_supersession_is_404() {
        let app = ContradictionApp::new();
        let id = app.remember_in("never superseded", "ops");

        let body = serde_json::json!({ "id": id }).to_string();
        let (status, resp) = app.call(Method::Post, "/contradictions/undo", Some(body));
        assert_eq!(status, 404, "undo without a supersession must 404: {resp}");

        // Malformed body → 400.
        let (status, _) = app.call(
            Method::Post,
            "/contradictions/undo",
            Some("not-json".into()),
        );
        assert_eq!(status, 400);
    }
}

// ── Explain recall API (#1160) ──────────────────────────────────────

#[cfg(test)]
mod explain_recall_api_tests {
    use super::*;
    use tiny_http::TestRequest;

    struct ExplainApp {
        uteke: Mutex<Uteke>,
    }

    impl ExplainApp {
        fn new() -> Self {
            // No embedder: tests use the fts5 strategy, which needs no
            // query embedding (CI-safe without ONNX).
            Self {
                uteke: Mutex::new(
                    Uteke::open_with_backend(":memory:", None)
                        .expect("open in-memory uteke without embedder"),
                ),
            }
        }

        fn call(
            &self,
            method: Method,
            url: &str,
            body: Option<String>,
        ) -> (u16, serde_json::Value) {
            let mut req = match body {
                Some(b) => {
                    let leaked: &'static str = Box::leak(b.into_boxed_str());
                    TestRequest::new()
                        .with_method(method)
                        .with_path(url)
                        .with_body(leaked)
                        .into()
                }
                None => TestRequest::new().with_method(method).with_path(url).into(),
            };
            let ctx = ReqCtx {
                auth_token_hash: None,
                read_only_token_hash: None,
                cors_origins: Vec::new(),
                recall_config: None,
                extraction_config: None,
            };
            let resp = route(&self.uteke, &ctx, &mut req);
            let status = resp.status_code().0;
            let bytes = resp.into_reader().into_inner();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }

        fn remember_in(&self, content: &str, namespace: &str) -> String {
            let body =
                serde_json::json!({ "content": content, "namespace": namespace }).to_string();
            let (status, resp) = self.call(Method::Post, "/remember", Some(body));
            assert_eq!(status, 200, "remember must succeed: {resp}");
            resp["id"]
                .as_str()
                .unwrap_or_else(|| panic!("remember response must carry id: {resp}"))
                .to_string()
        }
    }

    #[test]
    fn explain_fts5_returns_signals_and_guards() {
        let app = ExplainApp::new();
        let fox_id = app.remember_in("The quick brown fox jumps over the lazy dog", "explain-ns");
        app.remember_in(
            "Completely unrelated content about gardening tools",
            "explain-ns",
        );

        // Body built separately so the api_registry route scanner does not
        // mistake this literal for a handler route arm.
        let body = serde_json::json!({
            "query": "quick brown fox",
            "limit": 5,
            "namespace": "explain-ns",
            "strategy": "fts5",
            "explain": true
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/recall", Some(body));
        assert_eq!(status, 200, "{resp}");
        let arr = resp.as_array().expect("explained results array");
        assert!(!arr.is_empty(), "fts5 must find the fox");
        let first = &arr[0];
        assert_eq!(first["memory"]["id"], serde_json::json!(fox_id));
        let ex = &first["explanation"];
        assert_eq!(ex["strategy"], serde_json::json!("fts5"));
        assert_eq!(ex["fts_rank"], serde_json::json!(1));
        assert!(ex["final_score"].is_number(), "final_score must exist");

        // explain + search_type → 400 (memory-only feature).
        let bad = serde_json::json!({
            "query": "quick brown fox",
            "search_type": "all",
            "explain": true
        })
        .to_string();
        let (status, _) = app.call(Method::Post, "/recall", Some(bad));
        assert_eq!(status, 400, "explain + search_type must 400");

        // explain + at → 400.
        let bad = serde_json::json!({
            "query": "quick brown fox",
            "at": "2026-01-01T00:00:00Z",
            "explain": true
        })
        .to_string();
        let (status, _) = app.call(Method::Post, "/recall", Some(bad));
        assert_eq!(status, 400, "explain + at must 400");

        // explain + entity → 400 (filter would be silently dropped).
        let bad = serde_json::json!({
            "query": "quick brown fox",
            "entity": "staging",
            "explain": true
        })
        .to_string();
        let (status, _) = app.call(Method::Post, "/recall", Some(bad));
        assert_eq!(status, 400, "explain + entity must 400");
    }
}

// ── /list pagination metadata (#1188) ───────────────────────────────

#[cfg(test)]
mod list_pagination_tests {
    use super::*;
    use tiny_http::TestRequest;

    struct ListApp {
        uteke: Mutex<Uteke>,
    }

    impl ListApp {
        fn new() -> Self {
            Self {
                uteke: Mutex::new(
                    Uteke::open_with_backend(":memory:", None)
                        .expect("open in-memory uteke without embedder"),
                ),
            }
        }

        fn call(
            &self,
            method: Method,
            url: &str,
            body: Option<String>,
        ) -> (u16, serde_json::Value) {
            let mut req = match body {
                Some(b) => {
                    let leaked: &'static str = Box::leak(b.into_boxed_str());
                    TestRequest::new()
                        .with_method(method)
                        .with_path(url)
                        .with_body(leaked)
                        .into()
                }
                None => TestRequest::new().with_method(method).with_path(url).into(),
            };
            let ctx = ReqCtx {
                auth_token_hash: None,
                read_only_token_hash: None,
                cors_origins: Vec::new(),
                recall_config: None,
                extraction_config: None,
            };
            let resp = route(&self.uteke, &ctx, &mut req);
            let status = resp.status_code().0;
            let bytes = resp.into_reader().into_inner();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }

        fn remember_in(&self, content: &str, namespace: &str) {
            let body =
                serde_json::json!({ "content": content, "namespace": namespace }).to_string();
            let (status, resp) = self.call(Method::Post, "/remember", Some(body));
            assert_eq!(status, 200, "seed remember must succeed: {resp}");
        }
    }

    #[test]
    fn list_meta_envelope_and_bare_default() {
        let app = ListApp::new();
        for i in 0..7 {
            app.remember_in(&format!("pagination probe {i}"), "pg");
        }

        // Default (no include_meta): bare array — unchanged shape.
        let body = serde_json::json!({ "namespace": "pg", "limit": 100 }).to_string();
        let (status, resp) = app.call(Method::Post, "/list", Some(body));
        assert_eq!(status, 200);
        assert!(resp.is_array(), "default must stay a bare array: {resp}");
        assert_eq!(resp.as_array().unwrap().len(), 7);

        // include_meta: envelope with total/has_more/next_offset.
        // URL built separately so the api_registry route scanner does not
        // mistake this literal for a handler route arm.
        let body = serde_json::json!({
            "namespace": "pg",
            "limit": 3,
            "offset": 0,
            "include_meta": true
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/list", Some(body));
        assert_eq!(status, 200);
        assert!(resp.is_object(), "envelope expected: {resp}");
        assert_eq!(resp["total"], serde_json::json!(7));
        assert_eq!(resp["has_more"], serde_json::json!(true));
        assert_eq!(resp["next_offset"], serde_json::json!(3));
        assert_eq!(resp["memories"].as_array().expect("page rows").len(), 3);

        // Last page: has_more false, next_offset null.
        let body = serde_json::json!({
            "namespace": "pg",
            "limit": 10,
            "offset": 6,
            "include_meta": true
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/list", Some(body));
        assert_eq!(status, 200);
        assert_eq!(resp["has_more"], serde_json::json!(false));
        assert!(resp["next_offset"].is_null());
        assert_eq!(
            resp["memories"].as_array().expect("page rows").len(),
            1,
            "offset 6 of 7 rows returns the last row"
        );

        // include_meta + at: `at` wins — bare array (documented).
        let body = serde_json::json!({
            "namespace": "pg",
            "include_meta": true,
            "at": "2100-01-01T00:00:00Z"
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/list", Some(body));
        assert_eq!(status, 200);
        assert!(resp.is_array(), "at-mode stays a bare array: {resp}");
    }
}

#[cfg(test)]
mod payload_conformance_tests {
    use super::*;
    use std::io::Read as IoRead;

    // ── #1233: raw-payload conformance — recall responses carry the FULL
    // payload on every surface (HTTP route tested here; CLI --json prints
    // the same serde SearchResult; MCP unified path reuses these types) ──
    #[test]
    fn recall_http_payload_conformance() {
        use uteke_core::Uteke;

        let uteke = Uteke::open(":memory:").unwrap();
        uteke
            .remember(
                "Payload conformance probe memory #1233 with distinctive tokens zebraquartz",
                &[],
                None,
                Some("conf"),
            )
            .unwrap();

        let shared = std::sync::Mutex::new(uteke);
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: vec![],
            recall_config: None,
            extraction_config: None,
        };

        let mut req = tiny_http::TestRequest::new()
            .with_method(tiny_http::Method::Post)
            .with_path("/recall")
            .with_body(r#"{"query":"zebraquartz","limit":5,"min_score":0.0,"namespace":"conf"}"#)
            .into();
        let resp = route(&shared, &ctx, &mut req);
        let mut buf = String::new();
        resp.into_reader().read_to_string(&mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_str(&buf).expect("valid JSON");

        // Raw payload must be an array of FULL SearchResult objects.
        let arr = v
            .as_array()
            .expect("recall response must be a top-level JSON array");
        assert!(!arr.is_empty(), "expected at least one hit");
        let first = &arr[0];
        // Full payload = the memory object itself, not a summary stub.
        let mem = first
            .get("memory")
            .expect("each hit must carry the full memory object");
        assert!(mem.get("id").is_some(), "hit must include memory.id");
        let content = mem
            .get("content")
            .and_then(|c| c.as_str())
            .expect("hit must include memory.content");
        assert!(
            content.contains("zebraquartz"),
            "content must be the FULL memory text"
        );
        assert!(
            mem.get("created_at").is_some(),
            "hit must include memory metadata"
        );
        assert!(
            first.get("score").and_then(|s| s.as_f64()).is_some(),
            "hit must include score"
        );
        // No stub markers anywhere.
        let s = v.to_string();
        assert!(
            !s.contains("raw_hits"),
            "hit-count stub leaked into payload"
        );
    }

    #[test]
    fn recall_http_empty_result_has_no_stub() {
        use uteke_core::Uteke;

        let uteke = Uteke::open(":memory:").unwrap();
        let shared = std::sync::Mutex::new(uteke);
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: vec![],
            recall_config: None,
            extraction_config: None,
        };
        let mut req = tiny_http::TestRequest::new()
            .with_method(tiny_http::Method::Post)
            .with_path("/recall")
            .with_body(r#"{"query":"totally-unique-missing-query-xyz","limit":5,"min_score":0.0,"namespace":"conf"}"#)
            .into();
        let resp = route(&shared, &ctx, &mut req);
        let mut buf = String::new();
        resp.into_reader().read_to_string(&mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_str(&buf).expect("valid JSON");
        // Contract: an empty result set is a bare EMPTY JSON ARRAY — the same
        // full-payload shape as a populated response, just with zero hits.
        // A "N hits" summary string or a {"raw_hits": N} stub would fail this.
        let arr = v
            .as_array()
            .expect("recall response must be a top-level JSON array, never a summary string/stub");
        assert!(arr.is_empty(), "expected an empty array for a no-hit query");
    }
}
