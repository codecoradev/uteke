//! Route dispatcher — all HTTP endpoint handlers.
//!
//! [`route`] runs the shared preflight (API version prefix, Host guard, CORS
//! preflight, auth, read-only gate, registry 404), locks the store, packs the
//! per-request state into a [`Call`] and dispatches on method + path to one
//! named handler function per route. The handlers live in domain submodules
//! (`health`, `memory`, `namespaces`, `tags`, `rooms`, `docs`, `graph`,
//! `maintenance`, `mcp`); helpers shared by several handlers stay here.

use std::io::{Cursor, Read as IoRead};
use std::sync::Mutex;

use serde::Deserialize;
use tiny_http::{Header, Method, Request, Response, StatusCode};
use tracing::{error, warn};

use uteke_core::Uteke;

use crate::api_registry;
use crate::context::{self, ApiRole, AuthResult, ReqCtx};
use crate::types::*;

/// Current API version constant — used by health and versioned routes.
const API_LATEST: &str = "v2";
const API_VERSIONS: &[&str] = &["v1", "v2"];

mod docs;
mod graph;
mod health;
mod maintenance;
mod mcp;
mod memory;
mod namespaces;
mod rooms;
mod tags;

/// Per-request state handed to a route handler: the locked store plus what
/// the preflight in [`route`] derived from the request.
pub(crate) struct Call<'a, 'r> {
    /// The locked Uteke instance.
    pub uteke: &'a Uteke,
    pub ctx: &'a ReqCtx,
    pub req: &'r mut Request,
    /// Path with the `/api/vN` prefix stripped, query string intact.
    pub path: &'a str,
    pub api_version: Option<ApiVersion>,
    /// A tokenless `GET /health` probe (#1252).
    pub health_anonymous: bool,
    /// The caller holds a read-only token (#409).
    pub read_only: bool,
}

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

    // DNS-rebinding guard (#1326): runs before everything else, preflight and
    // health included, since a rebinding page can hit any route.
    let host_header = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| h.value.as_str().to_string());
    if !ctx.host_guard.allows(host_header.as_deref()) {
        return ctx.error_response_for(req, 403, "Host not allowed");
    }

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

    // Health endpoint — reachable WITHOUT a token (load balancers, uptime
    // probes), but anonymous callers get a minimal body: version, memory
    // counts, and update info leak the instance fingerprint to scanners (#1252).
    let is_health = matches!((&method, route_path), (Method::Get, "/health"));

    // Authenticate every request; a tokenless health probe degrades to the
    // Anonymous role instead of a 401 so uptime checks keep working.
    let mut health_anonymous = false;
    let auth_role = if !is_health {
        match context::check_auth(req, ctx) {
            Ok(role) => role,
            Err(resp) => return resp,
        }
    } else {
        match context::check_auth(req, ctx) {
            Ok(role) => role,
            Err(_) => {
                health_anonymous = true;
                AuthResult::Anonymous
            }
        }
    };

    // Enforce read-only restriction (#409, #524):
    // ReadOnly tokens can use GET endpoints + read-only POST endpoints.
    // Write operations (POST /remember, POST /forget, etc.) are blocked.
    let read_only = matches!(auth_role, AuthResult::Authenticated(ApiRole::ReadOnly));
    if read_only {
        // The gate is derived from the registry's `writes` flag (#1344): GET
        // plus the POST rows that do not write (semantic search, list,
        // stats). Exact path match avoids prefix-based bypass (/recallfoo).
        if !api_registry::read_only_allows(method.as_str(), &path) {
            return ctx.error_response_for(
                req,
                403,
                "Read-only token cannot perform write operations",
            );
        }
    }

    // The registry is the route table (#1344): a (method, path) without a row
    // is a 404 whatever arms the match below still carries, so a handler
    // cannot be reachable while unregistered (or ungated).
    if api_registry::find(method.as_str(), route_path).is_none() {
        return ctx.error_response_for(req, 404, "Not found");
    }

    // Lock the Uteke instance for the duration of this request.
    // This serializes requests but prevents data races on the SQLite connection.
    // Future: use rwlock for read-heavy workloads.
    // A handler panic poisons the mutex; recover the guard instead of failing
    // every later request. SQLite transactions roll back on unwind, so the
    // underlying state stays consistent.
    let uteke = uteke.lock().unwrap_or_else(|e| e.into_inner());

    let mut call = Call {
        uteke: &uteke,
        ctx,
        req: &mut *req,
        path: &path,
        api_version,
        health_anonymous,
        read_only,
    };

    match (method, route_path) {
        // ── Health ──────────────────────────────────────────────────────
        (Method::Get, "/health") => health::get_health(&mut call),

        // ── Memory Tools Guide (#1010) ──────────────────────────────────
        (Method::Get, "/guide") => health::get_guide(&mut call),

        // ── Route Introspection (#1289) ─────────────────────────────────
        (Method::Get, "/routes") => health::get_routes(&mut call),

        // ── Remember ───────────────────────────────────────────────────
        (Method::Post, "/remember") => memory::post_remember(&mut call),

        // ── Recall (semantic search) ────────────────────────────────────
        (Method::Post, "/recall") => memory::post_recall(&mut call),

        // ── Search (keyword) ────────────────────────────────────────────
        (Method::Post, "/search") => memory::post_search(&mut call),

        // ── List ────────────────────────────────────────────────────────
        (Method::Post, "/list") => memory::post_list(&mut call),

        // ── Forget by ID or tag (DELETE /forget?id=xxx or ?tag=xxx) ────
        (Method::Delete, "/forget") => memory::delete_forget(&mut call),

        // ── Stats (GET = all or ?namespace=<name>) ───────────────────
        (Method::Get, "/stats") => memory::get_stats(&mut call),
        (Method::Post, "/stats") => memory::post_stats(&mut call),

        // ── Lifecycle (#936): status, cycle, promote ──────────────────
        (Method::Get, "/lifecycle/status") => maintenance::get_lifecycle_status(&mut call),

        (Method::Get, "/lifecycle/deprecated") => maintenance::get_lifecycle_deprecated(&mut call),

        (Method::Post, "/lifecycle/cycle") => maintenance::post_lifecycle_cycle(&mut call),

        (Method::Post, "/lifecycle/promote") => maintenance::post_lifecycle_promote(&mut call),

        // ── Namespaces ──────────────────────────────────────────────────
        (Method::Get, "/namespaces") => namespaces::get_namespaces(&mut call),

        // ── Namespace Rename/Merge (#1181) ──────────────────────────────
        (Method::Post, "/namespaces/rename") => namespaces::post_namespaces_rename(&mut call),

        // ── Namespace Delete with explicit strategy (#1181) ─────────────
        (Method::Post, "/namespaces/delete") => namespaces::post_namespaces_delete(&mut call),

        // ── Recent (#528) ──────────────────────────────────────────────
        (Method::Get, "/recent") => memory::get_recent(&mut call),

        // ── Graph Visualization (#408) ───────────────────────────────────
        (Method::Get, "/graph") => graph::get_graph(&mut call),

        // ── Graph Mutation: Add Edge (#542) ──────────────────────────────
        (Method::Post, "/graph/edge") => graph::post_graph_edge(&mut call),

        // ── Graph Mutation: Remove Edge (#542) ────────────────────────────
        (Method::Delete, "/graph/edge") => graph::delete_graph_edge(&mut call),

        // ── Room Summary ────────────────────────────────────────────────
        (Method::Post, "/room/summary") => rooms::post_room_summary(&mut call),

        // ── Room Summary Document ────────────────────────────────────────
        (Method::Post, "/room/summary-document") => rooms::post_room_summary_document(&mut call),

        // ── Room Consolidation (#1088) ───────────────────────────────────
        (Method::Post, "/room/consolidate") => rooms::post_room_consolidate(&mut call),

        // ── DEPRECATED: POST /room/document → /room/summary-document (#735)
        (Method::Post, "/room/document") => rooms::post_room_document(&mut call),

        // ── Get memory by ID ──────────────────────────────────────────
        (Method::Get, "/memory") => memory::get_memory(&mut call),

        // ── Memory Pin/Unpin (#660) ──────────────────────────────────────
        (Method::Post, "/memory/pin") => memory::post_memory_pin(&mut call),

        // ── Memory Set Importance (#660) ──────────────────────────────────
        (Method::Post, "/memory/importance") => memory::post_memory_importance(&mut call),

        // ── Memory Feedback (Trust Scoring) (#718) ────────────────────
        (Method::Post, "/memory/feedback") => memory::post_memory_feedback(&mut call),

        // ── Update memory by ID (PUT /memory, #659) ──────────────────
        (Method::Put, "/memory") => memory::put_memory(&mut call),

        // ── Room Memories (chronological listing — GET /room/memories) ────
        (Method::Get, "/room/memories") => rooms::get_room_memories(&mut call),

        // ── Room Recall (semantic with optional fallback to chronological) ──
        (Method::Post, "/room/recall") => rooms::post_room_recall(&mut call),

        // ── Room management endpoints (#395) ────────────────────────────
        (Method::Post, "/room/create") => rooms::post_room_create(&mut call),

        (Method::Post, "/room/rename") => rooms::post_room_rename(&mut call),

        (Method::Post, "/room/update") => rooms::post_room_update(&mut call),

        (Method::Post, "/room/memory/move") => rooms::post_room_memory_move(&mut call),

        (Method::Post, "/room/remember") => rooms::post_room_remember(&mut call),

        (Method::Get, "/room/list") => rooms::get_room_list(&mut call),

        (Method::Post, "/room/stats") => rooms::post_room_stats(&mut call),

        // ── Room ↔ Document junction (#689) ──────────────────────────────
        (Method::Post, "/room/document/list") => rooms::post_room_document_list(&mut call),

        (Method::Put, "/room/document/add") => rooms::put_room_document_add(&mut call),

        (Method::Delete, "/room/document/remove") => rooms::delete_room_document_remove(&mut call),

        (Method::Post, "/doc/room/list") => rooms::post_doc_room_list(&mut call),

        (Method::Delete, "/room/delete") => rooms::delete_room_delete(&mut call),
        // ── Context Summary (#442) ───────────────────────────────────────
        (Method::Post, "/context") => maintenance::post_context(&mut call),

        // ── Dream Cycle (#442) ─────────────────────────────────────────────
        (Method::Post, "/dream") => maintenance::post_dream(&mut call),

        (Method::Post, "/mcp") => mcp::post_mcp(&mut call),

        // ── Document: Create / Upsert ────────────────────────────────────
        (Method::Post, "/doc/create") => docs::post_doc_create(&mut call),

        // ── Document: Get ───────────────────────────────────────────────
        (Method::Post, "/doc/get") => docs::post_doc_get(&mut call),

        // ── Document: Update (partial) ────────────────────────────────
        (Method::Post, "/doc/update") => docs::post_doc_update(&mut call),

        // ── Document: List ─────────────────────────────────────────────
        (Method::Post, "/doc/list") => docs::post_doc_list(&mut call),

        // ── Document: Search ────────────────────────────────────────────
        (Method::Post, "/doc/search") => docs::post_doc_search(&mut call),

        // ── Document: Move ───────────────────────────────────────────────
        (Method::Post, "/doc/move") => docs::post_doc_move(&mut call),

        // ── Document: Delete ─────────────────────────────────────────────
        (Method::Delete, "/doc/delete") => docs::delete_doc_delete(&mut call),

        // ── Cross-entity references (#689) ───────────────────────────────
        (Method::Post, "/memory/doc-refs") => memory::post_memory_doc_refs(&mut call),

        (Method::Post, "/doc/mem-refs") => docs::post_doc_mem_refs(&mut call),

        // ── Tags: List with counts ───────────────────────────────────────
        (Method::Get, "/tags") => tags::get_tags(&mut call),

        // ── Tags: Rename ─────────────────────────────────────────────────
        (Method::Post, "/tags/rename") => tags::post_tags_rename(&mut call),

        // ── Tags: Delete ─────────────────────────────────────────────────
        (Method::Post, "/tags/delete") => tags::post_tags_delete(&mut call),

        // ── Pin ───────────────────────────────────────────────────────────
        (Method::Post, "/pin") => memory::post_pin(&mut call),

        // ── Unpin ─────────────────────────────────────────────────────────
        (Method::Post, "/unpin") => memory::post_unpin(&mut call),

        // ── Provenance chain (#1172) ─────────────────────────────────────
        (Method::Get, "/provenance") => graph::get_provenance(&mut call),

        // ── Contradiction resolutions ledger (#1172) ────────────────────
        (Method::Get, "/contradictions") => graph::get_contradictions(&mut call),

        // ── Undo a contradiction resolution (#1172) ─────────────────────
        (Method::Post, "/contradictions/undo") => graph::post_contradictions_undo(&mut call),

        // ── Timeline ─────────────────────────────────────────────────────
        (Method::Get, "/timeline") => graph::get_timeline(&mut call),

        // ── Edges ────────────────────────────────────────────────────────
        (Method::Get, "/edges") => graph::get_edges(&mut call),

        // ── Extract (LLM fact extraction → store) ────────────────────────
        (Method::Post, "/extract") => maintenance::post_extract(&mut call),

        // ── Import (JSONL) ──────────────────────────────────────────────
        (Method::Post, "/import") => maintenance::post_import(&mut call),

        // ── Export (JSONL) ──────────────────────────────────────────────
        (Method::Get, "/export") => maintenance::get_export(&mut call),

        // ── Prune (maintenance) ───────────────────────────────────────────
        // ── Verify / Repair (index consistency, #1266) ───────────────────
        (Method::Post, "/verify") => maintenance::post_verify(&mut call),

        (Method::Post, "/repair") => maintenance::post_repair(&mut call),

        (Method::Post, "/prune") => maintenance::post_prune(&mut call),

        // ── Consolidate (maintenance) ────────────────────────────────────
        (Method::Post, "/consolidate") => maintenance::post_consolidate(&mut call),

        (Method::Post, "/consolidate/pair") => maintenance::post_consolidate_pair(&mut call),

        // ── Aging (maintenance) ─────────────────────────────────────────
        (Method::Post, "/aging") => maintenance::post_aging(&mut call),

        // ── Importance (monitoring) ────────────────────────────────────────
        (Method::Post, "/importance") => maintenance::post_importance(&mut call),

        // ── Orphans (monitoring — read-only) ─────────────────────────────
        (Method::Post, "/orphans") => maintenance::post_orphans(&mut call),

        // ── Rebuild Backlinks (monitoring) ────────────────────────────────
        (Method::Post, "/rebuild-backlinks") => maintenance::post_rebuild_backlinks(&mut call),

        // ── 404 ─────────────────────────────────────────────────────────
        // Unreachable while every registry row has an arm (enforced by the
        // `every_registry_row_has_a_handler_arm` test); kept for exhaustiveness.
        _ => {
            #[cfg(test)]
            UNHANDLED_ROW.with(|c| c.set(true));
            ctx.error_response_for(req, 404, "Not found")
        }
    }
}

// Test probe: set when a registry row reached the match fallback, i.e. it
// has no handler arm.
#[cfg(test)]
thread_local! {
    static UNHANDLED_ROW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
        max_facts: req_max_facts
            .unwrap_or(base.max_facts)
            .min(MAX_EXTRACT_FACTS),
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

/// Add the namespace deprecation `warning` (#1427) to a write response that
/// has no warning field of its own. Nothing is added for the default
/// namespace, so those responses stay byte-identical.
fn with_namespace_warning(
    mut body: serde_json::Value,
    namespace: Option<&str>,
) -> serde_json::Value {
    if let Some(w) = uteke_core::namespace_deprecation::namespace_write_warning(namespace)
        && let Some(obj) = body.as_object_mut()
    {
        obj.insert("warning".into(), serde_json::Value::String(w.to_string()));
    }
    body
}

#[cfg(test)]
mod room_memories_namespace_tests {
    use super::*;
    use tiny_http::TestRequest;

    /// Minimal harness mirroring ContradictionApp: no embedder (CI-safe),
    /// route() called directly (#1288).
    struct RoomMemoriesApp {
        uteke: Mutex<Uteke>,
    }

    impl RoomMemoriesApp {
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
                host_guard: Default::default(),
            };
            let resp = route(&self.uteke, &ctx, &mut req);
            let status = resp.status_code().0;
            let bytes = resp.into_reader().into_inner();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }
    }

    #[test]
    fn room_memories_namespace_filter_and_param_observable() {
        let app = RoomMemoriesApp::new();

        let body = serde_json::json!({ "room_id": "ns-room", "namespace": "default" }).to_string();
        let (status, resp) = app.call(Method::Post, "/room/create", Some(body));
        assert_eq!(status, 200, "{resp}");

        let mut ids: Vec<String> = Vec::new();
        for (ns, author) in [("cmo", "cmo-agent"), ("hermes", "hermes-agent")] {
            let body = serde_json::json!({
                "room_id": "ns-room",
                "content": format!("note from {ns}"),
                "namespace": ns,
                "author": author
            })
            .to_string();
            let (status, resp) = app.call(Method::Post, "/room/remember", Some(body));
            assert_eq!(status, 200, "{resp}");
            ids.push(resp["id"].as_str().expect("memory id").to_string());
        }

        // URL constants for the query-string URLs used below.
        const URL_ALL: &str = "/room/memories?room_id=ns-room";
        const URL_CMO: &str = "/room/memories?room_id=ns-room&namespace=cmo";
        const URL_CMO_OTHER_AUTHOR: &str =
            "/room/memories?room_id=ns-room&namespace=cmo&author=hermes-agent";

        // Unfiltered: cross-namespace default — both entries.
        let (status, resp) = app.call(Method::Get, URL_ALL, None);
        assert_eq!(status, 200, "{resp}");
        let arr = resp.as_array().expect("bare JSON array response");
        assert_eq!(
            arr.len(),
            2,
            "cross-namespace default must return both: {resp}"
        );

        // namespace=cmo → exactly the cmo entry (#1288 — was silently ignored).
        let (status, resp) = app.call(Method::Get, URL_CMO, None);
        assert_eq!(status, 200, "{resp}");
        let arr = resp.as_array().expect("bare JSON array response");
        assert_eq!(arr.len(), 1, "namespace filter must narrow results: {resp}");
        assert_eq!(arr[0]["namespace"], serde_json::json!("cmo"));
        assert_eq!(arr[0]["content"], serde_json::json!("note from cmo"));

        // namespace + author combined.
        let (status, resp) = app.call(Method::Get, URL_CMO_OTHER_AUTHOR, None);
        assert_eq!(status, 200, "{resp}");
        let arr = resp.as_array().expect("bare JSON array response");
        assert!(
            arr.is_empty(),
            "conflicting filters must yield empty: {resp}"
        );
    }
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
            recall_count: 0,
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
                host_guard: Default::default(),
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

        // POST still reports the graph-node ids it created (#1180)...
        assert!(resp["source_node"].as_str().is_some());
        assert!(resp["target_node"].as_str().is_some());

        // ...but GET /graph keys nodes by MEMORY id (#1366) so a viewer can
        // open them, and keeps `memory_id` for clients that map back.
        let (_, graph) = app.call(Method::Get, "/graph", None);
        let nodes = graph["nodes"].as_array().expect("nodes array");
        for (id, text) in [(&id1, "alpha memory"), (&id2, "beta memory")] {
            assert!(
                nodes.iter().any(|n| n["id"] == serde_json::json!(id)
                    && n["memory_id"] == serde_json::json!(id)
                    && n["label"].as_str().is_some_and(|l| l.starts_with(text))),
                "node for memory {id} missing: {graph}"
            );
        }

        // The edge is visible with both long and short endpoint names.
        let edges = graph["edges"].as_array().expect("edges array");
        assert!(
            edges
                .iter()
                .any(|e| e["source_id"] == serde_json::json!(id1)
                    && e["target_id"] == serde_json::json!(id2)
                    && e["source"] == serde_json::json!(id1)
                    && e["target"] == serde_json::json!(id2)
                    && e["relation"] == serde_json::json!("related")),
            "edge must appear in GET /graph: {graph}"
        );
        assert_eq!(graph["truncated"], serde_json::json!(false));
    }

    #[test]
    fn graph_node_id_and_limit_query_params() {
        let app = GraphEdgeApp::new();
        let a = app.remember("hub memory");
        let b = app.remember("spoke one");
        let c = app.remember("spoke two");
        let d = app.remember("elsewhere one");
        let e = app.remember("elsewhere two");
        for (s, t) in [(&a, &b), (&a, &c), (&d, &e)] {
            let (status, resp) = app.add_edge(s, t);
            assert_eq!(status, 200, "{resp}");
        }

        let hood_url = format!("/graph?node_id={a}");
        let (_, hood) = app.call(Method::Get, &hood_url, None);
        let touching: Vec<&serde_json::Value> = hood["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["relation"] == serde_json::json!("related"))
            .collect();
        assert!(
            touching
                .iter()
                .all(|e| e["source"] == serde_json::json!(a) || e["target"] == serde_json::json!(a)),
            "node_id must restrict to that memory's edges: {hood}"
        );
        assert!(touching.len() >= 2, "{hood}");

        let capped_url = "/graph?limit=1";
        let (_, capped) = app.call(Method::Get, capped_url, None);
        assert_eq!(capped["edges"].as_array().unwrap().len(), 1, "{capped}");
        assert_eq!(capped["truncated"], serde_json::json!(true));
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
                host_guard: Default::default(),
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
                host_guard: Default::default(),
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
mod pack_recall_api_tests {
    //! #1281 Phase 1: `pack` recall returns a budgeted ContextPack envelope.
    use super::*;
    use tiny_http::TestRequest;

    struct PackApp {
        uteke: Mutex<Uteke>,
    }

    impl PackApp {
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
                host_guard: Default::default(),
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
    fn pack_returns_envelope_honours_budget_and_excludes() {
        let app = PackApp::new();
        let fox_id = app.remember_in("The quick brown fox jumps over the lazy dog", "pack-ns");
        app.remember_in(
            "Completely unrelated content about gardening tools",
            "pack-ns",
        );

        // Normal pack: envelope shape with a generous budget.
        let body = serde_json::json!({
            "query": "quick brown fox",
            "limit": 5,
            "namespace": "pack-ns",
            "strategy": "fts5",
            "pack": true,
            "budget_chars": 10000
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/recall", Some(body));
        assert_eq!(status, 200, "{resp}");
        assert!(
            resp["selected"].is_array(),
            "pack must carry selected: {resp}"
        );
        assert!(
            !resp["selected"].as_array().unwrap().is_empty(),
            "fts5 must find the fox"
        );
        assert_eq!(resp["budget_chars"], serde_json::json!(10000));
        assert!(resp["budget_used"].as_u64().unwrap() > 0, "{resp}");
        assert!(resp["skipped"].is_array(), "{resp}");

        // exclude_ids: the fox is already injected → skipped, reason excluded.
        let body2 = serde_json::json!({
            "query": "quick brown fox",
            "limit": 5,
            "namespace": "pack-ns",
            "strategy": "fts5",
            "pack": true,
            "budget_chars": 10000,
            "exclude_ids": [fox_id]
        })
        .to_string();
        let (status, resp2) = app.call(Method::Post, "/recall", Some(body2));
        assert_eq!(status, 200, "{resp2}");
        let skipped = resp2["skipped"].as_array().unwrap();
        assert!(
            skipped
                .iter()
                .any(|s| s["memory_id"] == serde_json::json!(fox_id)
                    && s["reason"] == serde_json::json!("excluded")),
            "excluded fox must be reported: {resp2}"
        );

        // Impossible budget → nothing selected, everything skipped as budget.
        let body3 = serde_json::json!({
            "query": "quick brown fox",
            "limit": 5,
            "namespace": "pack-ns",
            "strategy": "fts5",
            "pack": true,
            "budget_chars": 1
        })
        .to_string();
        let (status, resp3) = app.call(Method::Post, "/recall", Some(body3));
        assert_eq!(status, 200, "{resp3}");
        assert!(resp3["selected"].as_array().unwrap().is_empty());
        assert_eq!(resp3["budget_used"], serde_json::json!(0));
        assert!(
            !resp3["skipped"].as_array().unwrap().is_empty(),
            "oversized items must be reported: {resp3}"
        );

        // pack + time filters → 400 (loud, same contract as the CLI).
        let body4 = serde_json::json!({
            "query": "quick brown fox",
            "namespace": "pack-ns",
            "strategy": "fts5",
            "pack": true,
            "budget_chars": 10000,
            "at": "2026-01-01T00:00:00Z"
        })
        .to_string();
        let (status, resp4) = app.call(Method::Post, "/recall", Some(body4));
        assert_eq!(status, 400, "{resp4}");
        assert!(
            resp4["error"]
                .as_str()
                .unwrap_or_default()
                .contains("pack is not supported"),
            "{resp4}"
        );
    }
}

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
                host_guard: Default::default(),
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
                host_guard: Default::default(),
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

        // No embedder: these tests are payload-shape conformance and must run
        // in CI builds without the ONNX runtime lib (same pattern as the
        // graph-edge tests). Keyword (fts5) recall needs no vectors.
        let uteke = Uteke::open_with_backend(":memory:", None)
            .expect("open in-memory uteke without embedder");
        let shared = std::sync::Mutex::new(uteke);
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: vec![],
            recall_config: None,
            extraction_config: None,
            host_guard: Default::default(),
        };

        let remember_body: &'static str = Box::leak(r#"{"content":"Payload conformance probe memory #1233 with distinctive tokens zebraquartz","namespace":"conf"}"#.to_string().into_boxed_str());
        let mut remember_req = tiny_http::TestRequest::new()
            .with_method(tiny_http::Method::Post)
            .with_path("/remember")
            .with_body(remember_body)
            .into();
        let remember_resp = route(&shared, &ctx, &mut remember_req);
        let mut rbuf = String::new();
        remember_resp
            .into_reader()
            .read_to_string(&mut rbuf)
            .unwrap();
        assert!(rbuf.contains("\"id\""), "remember must succeed: {rbuf}");

        let mut req = tiny_http::TestRequest::new()
            .with_method(tiny_http::Method::Post)
            .with_path("/recall")
            .with_body(r#"{"query":"zebraquartz","limit":5,"min_score":0.0,"namespace":"conf","strategy":"fts5"}"#)
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

        // No embedder: these tests are payload-shape conformance and must run
        // in CI builds without the ONNX runtime lib (same pattern as the
        // graph-edge tests). Keyword (fts5) recall needs no vectors.
        let uteke = Uteke::open_with_backend(":memory:", None)
            .expect("open in-memory uteke without embedder");
        let shared = std::sync::Mutex::new(uteke);
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: vec![],
            recall_config: None,
            extraction_config: None,
            host_guard: Default::default(),
        };
        let mut req = tiny_http::TestRequest::new()
            .with_method(tiny_http::Method::Post)
            .with_path("/recall")
            .with_body(r#"{"query":"totally-unique-missing-query-xyz","limit":5,"min_score":0.0,"namespace":"conf","strategy":"fts5"}"#)
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
    /// #1252: an unauthenticated GET /health gets liveness only — no version,
    /// no counts — while an authenticated request (or auth-disabled server)
    /// still receives the full payload.
    #[test]
    fn test_health_anonymous_gets_minimal_payload() {
        struct HealthAuthApp {
            uteke: Mutex<Uteke>,
            admin_token: Option<&'static str>,
        }
        impl HealthAuthApp {
            fn with_auth(token: Option<&'static str>) -> Self {
                Self {
                    uteke: Mutex::new(
                        Uteke::open_with_backend(":memory:", None)
                            .expect("open in-memory uteke without embedder"),
                    ),
                    admin_token: token,
                }
            }

            fn call(&self, auth_header: Option<&str>) -> (u16, serde_json::Value) {
                let mut builder = tiny_http::TestRequest::new()
                    .with_method(tiny_http::Method::Get)
                    .with_path("/health");
                if let Some(h) = auth_header {
                    builder = builder
                        .with_header(tiny_http::Header::from_bytes("Authorization", h).unwrap());
                }
                let mut req: tiny_http::Request = builder.into();
                let ctx = ReqCtx {
                    auth_token_hash: self.admin_token.map(|t| {
                        use sha2::{Digest, Sha256};
                        Sha256::digest(t.as_bytes()).into()
                    }),
                    read_only_token_hash: None,
                    cors_origins: Vec::new(),
                    recall_config: None,
                    extraction_config: None,
                    host_guard: Default::default(),
                };
                let resp = route(&self.uteke, &ctx, &mut req);
                let status = resp.status_code().0;
                let bytes = resp.into_reader().into_inner();
                let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
                (status, json)
            }
        }

        // Auth ENABLED, no token -> minimal body only.
        let app = HealthAuthApp::with_auth(Some("secret-token"));
        let (status, json) = app.call(None);
        assert_eq!(status, 200);
        assert_eq!(json["status"], "ok", "liveness must stay available");
        assert!(
            json.get("version").is_none()
                && json.get("memories").is_none()
                && json.get("namespaces").is_none()
                && json.get("update_available").is_none(),
            "anonymous health must not leak instance details: {json}"
        );

        // Auth ENABLED, valid token -> full payload.
        let (status, json) = app.call(Some("Bearer secret-token"));
        assert_eq!(status, 200);
        assert_eq!(json["status"], "ok");
        assert!(
            json.get("version").is_some(),
            "authenticated health carries version: {json}"
        );
        assert!(
            json.get("memories").is_some(),
            "authenticated health carries counts: {json}"
        );

        // Auth ENABLED, wrong token -> same minimal body as anonymous:
        // an invalid credential must not see more than a scanner would,
        // and uptime probes keep a 200 while tokens are being rotated.
        let (status, json) = app.call(Some("Bearer wrong-token"));
        assert_eq!(status, 200);
        assert!(
            json.get("version").is_none() && json.get("memories").is_none(),
            "invalid credentials must get the minimal body: {json}"
        );

        // Auth DISABLED, no token -> full payload (single-operator local mode).
        let app = HealthAuthApp::with_auth(None);
        let (status, json) = app.call(None);
        assert_eq!(status, 200);
        assert!(
            json.get("memories").is_some(),
            "auth-disabled health stays full: {json}"
        );
    }
}

// ── /routes machine-readable registry (#1289) ──────────────────────────

#[cfg(test)]
mod routes_introspection_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    struct RoutesApp {
        uteke: Mutex<Uteke>,
        read_only_token_hash: Option<[u8; 32]>,
    }

    impl RoutesApp {
        fn new() -> Self {
            // No embedder: storage-only server tests must run in CI builds
            // without the ONNX runtime lib.
            Self {
                uteke: Mutex::new(
                    Uteke::open_with_backend(":memory:", None)
                        .expect("open in-memory uteke without embedder"),
                ),
                read_only_token_hash: None,
            }
        }

        fn with_read_only_token(token: &str) -> Self {
            let mut app = Self::new();
            app.read_only_token_hash = Some(Sha256::digest(token.as_bytes()).into());
            app
        }

        fn call(
            &self,
            method: Method,
            url: &str,
            auth_header: Option<&str>,
        ) -> (u16, serde_json::Value) {
            let mut builder = tiny_http::TestRequest::new()
                .with_method(method)
                .with_path(url);
            if let Some(h) = auth_header {
                builder =
                    builder.with_header(tiny_http::Header::from_bytes("Authorization", h).unwrap());
            }
            let mut req: tiny_http::Request = builder.into();
            let ctx = ReqCtx {
                auth_token_hash: None,
                read_only_token_hash: self.read_only_token_hash,
                cors_origins: Vec::new(),
                recall_config: None,
                extraction_config: None,
                host_guard: Default::default(),
            };
            let resp = route(&self.uteke, &ctx, &mut req);
            let status = resp.status_code().0;
            let bytes = resp.into_reader().into_inner();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }
    }

    #[test]
    fn routes_returns_full_registry() {
        let app = RoutesApp::new();
        let (status, json) = app.call(Method::Get, "/routes", None);
        assert_eq!(status, 200, "{json}");

        let entries = json.as_array().expect("routes payload is a JSON array");
        assert_eq!(
            entries.len(),
            crate::api_registry::ENDPOINTS.len(),
            "one entry per registered endpoint"
        );

        // Self-describing: /routes lists itself.
        let self_entry = entries
            .iter()
            .find(|e| e["path"] == "/routes" && e["method"] == "GET")
            .expect("/routes must list itself");
        assert!(self_entry["tier"].is_string(), "{self_entry}");
        assert!(self_entry["description"].is_string(), "{self_entry}");

        // Entry shape matches the registry struct fields.
        let first = &entries[0];
        for field in ["method", "path", "tier", "description", "issues"] {
            assert!(
                first.get(field).is_some(),
                "registry entry missing field '{field}': {first}"
            );
        }
    }

    #[test]
    fn routes_respects_auth_roles() {
        // RO-only server: no token -> 401.
        let app = RoutesApp::with_read_only_token("ro-secret");
        let (status, _) = app.call(Method::Get, "/routes", None);
        assert_eq!(status, 401, "anonymous must not read the registry");

        // Valid RO token -> allowed (GET is inherently read-only).
        let (status, json) = app.call(Method::Get, "/routes", Some("Bearer ro-secret"));
        assert_eq!(status, 200, "{json}");
        assert!(json.as_array().is_some(), "RO token gets the registry");
    }

    fn method_of(name: &str) -> Method {
        match name {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "DELETE" => Method::Delete,
            other => panic!("unexpected registry method {other}"),
        }
    }

    /// Every registry row must have a handler arm: the router consults the
    /// table first, so a row whose arm is missing would silently 404 through
    /// the fallback. Built from the table, not from source text (#1344).
    #[test]
    fn every_registry_row_has_a_handler_arm() {
        let app = RoutesApp::new();
        for e in crate::api_registry::ENDPOINTS {
            UNHANDLED_ROW.with(|c| c.set(false));
            let _ = app.call(method_of(e.method), e.path, None);
            assert!(
                !UNHANDLED_ROW.with(|c| c.get()),
                "{} {} is registered but has no handler arm",
                e.method,
                e.path
            );
        }
    }

    /// A route that is not in the table is a 404 that never reaches a
    /// handler, even when the path exists under another verb.
    #[test]
    fn unregistered_method_path_pairs_are_404_before_dispatch() {
        let app = RoutesApp::new();
        for (method, path) in [
            (Method::Put, "/list"),
            (Method::Delete, "/recall"),
            (Method::Get, "/remember"),
            (Method::Post, "/health"),
            (Method::Get, "/no-such-route"),
        ] {
            UNHANDLED_ROW.with(|c| c.set(false));
            let (status, json) = app.call(method.clone(), path, None);
            assert_eq!(status, 404, "{method} {path}: {json}");
            assert!(
                !UNHANDLED_ROW.with(|c| c.get()),
                "{method} {path} must be refused by the table, not the match fallback"
            );
        }
    }

    /// The read-only token is refused (403, gate message) on exactly the
    /// rows the table marks as writes, and never on the others.
    #[test]
    fn read_only_token_gets_403_on_exactly_the_writing_rows() {
        const GATE_MSG: &str = "Read-only token cannot perform write operations";
        let app = RoutesApp::with_read_only_token("ro-secret");
        let mut gated = 0;
        for e in crate::api_registry::ENDPOINTS {
            let (status, json) = app.call(method_of(e.method), e.path, Some("Bearer ro-secret"));
            let refused = status == 403 && json["error"] == GATE_MSG;
            let expect_refused = e.method != "GET" && !(e.method == "POST" && !e.writes);
            assert_eq!(
                refused, expect_refused,
                "{} {}: {status} {json}",
                e.method, e.path
            );
            if e.writes {
                assert!(refused, "{} {} writes but was not gated", e.method, e.path);
                gated += 1;
            }
        }
        assert!(gated > 0, "table must contain writing rows");
    }

    #[test]
    fn routes_post_is_not_routed() {
        let app = RoutesApp::new();
        let (status, _) = app.call(Method::Post, "/routes", None);
        assert_eq!(status, 404, "only GET /routes exists (catch-all 404)");
    }
}

// ── #1302: plain /remember must honor (not silently normalize) `type` ──────

#[cfg(test)]
mod plain_remember_type_tests {
    use super::*;
    use tiny_http::TestRequest;

    /// Minimal harness mirroring the other in-memory route tests:
    /// no embedder (CI-safe), route() called directly (#1288).
    struct PlainRememberApp {
        uteke: Mutex<Uteke>,
    }

    impl PlainRememberApp {
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
                host_guard: Default::default(),
            };
            let resp = route(&self.uteke, &ctx, &mut req);
            let status = resp.status_code().0;
            let bytes = resp.into_reader().into_inner();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }
    }

    #[test]
    fn plain_remember_honors_explicit_type() {
        let app = PlainRememberApp::new();

        // Issue repro: explicit `decision` on the plain path must survive —
        // silently downgrading it to `fact` broke typed writers (tole #143).
        let body = serde_json::json!({
            "content": "ISOLATION-TEST decision typing contract",
            "namespace": "repo-mem-e2e",
            "type": "decision",
            "tags": ["test", "isolation"],
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/remember", Some(body));
        assert_eq!(status, 200, "{resp}");
        let id = resp["id"].as_str().expect("memory id").to_string();

        let (status, mem) = app.call(Method::Get, &format!("/memory?id={id}"), None);
        assert_eq!(status, 200, "{mem}");
        assert_eq!(
            mem["memory_type"].as_str(),
            Some("decision"),
            "explicit type must be stored verbatim, not normalized to fact: {mem}"
        );
        // Metadata copy retained (back-compat for metadata-based readers).
        assert_eq!(mem["metadata"]["type"].as_str(), Some("decision"));
    }

    #[test]
    fn plain_remember_rejects_unknown_type_loudly() {
        let app = PlainRememberApp::new();

        let body = serde_json::json!({
            "content": "ISOLATION-TEST unknown type must 400",
            "type": "journal",
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/remember", Some(body));
        assert_eq!(status, 400, "unknown type must fail loudly: {resp}");
        let err = resp["error"].as_str().unwrap_or_default();
        assert!(err.contains("Unknown memory type"), "{err}");
        assert!(
            err.contains("decision"),
            "error must list the supported vocabulary: {err}"
        );
    }

    #[test]
    fn plain_remember_rejects_unknown_type_even_with_contradiction() {
        let app = PlainRememberApp::new();

        // #1302 review follow-up: type validation runs BEFORE the branch, so
        // the contradiction arm is equally covered — no path may store an
        // invalid type. (Honoring a valid type on the contradiction arm is
        // verified by live E2E: it forwards the type to the core, which has
        // no embedder here — CI-safe validation-only assertion.)
        let body = serde_json::json!({
            "content": "ISOLATION-TEST unknown type contradiction arm",
            "type": "journal",
            "detect_contradiction": true,
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/remember", Some(body));
        assert_eq!(
            status, 400,
            "type validation must precede the contradiction arm: {resp}"
        );
        assert!(
            resp["error"]
                .as_str()
                .unwrap_or_default()
                .contains("Unknown memory type")
        );
    }

    #[test]
    fn plain_remember_without_type_still_auto_infers() {
        let app = PlainRememberApp::new();

        // No explicit type → legacy auto-inference path unchanged.
        let body = serde_json::json!({
            "content": "ISOLATION-TEST auto infer: we decided to keep the contract",
        })
        .to_string();
        let (status, resp) = app.call(Method::Post, "/remember", Some(body));
        assert_eq!(status, 200, "{resp}");
        let id = resp["id"].as_str().expect("memory id").to_string();

        let (status, mem) = app.call(Method::Get, &format!("/memory?id={id}"), None);
        assert_eq!(status, 200, "{mem}");
        assert_eq!(
            mem["memory_type"].as_str(),
            Some("decision"),
            "content contains 'we decided' → auto-inference must still run: {mem}"
        );
    }
}

#[cfg(test)]
mod host_guard_route_tests {
    use super::*;
    use crate::context::HostGuard;
    use tiny_http::{Header, TestRequest};

    fn status(guard: HostGuard, host: Option<&str>, path: &str) -> u16 {
        let uteke = Mutex::new(
            Uteke::open_with_backend(":memory:", None)
                .expect("open in-memory uteke without embedder"),
        );
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: Vec::new(),
            recall_config: None,
            extraction_config: None,
            host_guard: guard,
        };
        let mut t = TestRequest::new().with_method(Method::Get).with_path(path);
        if let Some(h) = host {
            t = t.with_header(Header::from_bytes(&b"Host"[..], h.as_bytes()).unwrap());
        }
        let mut req = t.into();
        route(&uteke, &ctx, &mut req).status_code().0
    }

    // #1326 acceptance: a loopback bind rejects `Host: evil.example`.
    #[test]
    fn loopback_bind_rejects_foreign_host() {
        let g = HostGuard::new("127.0.0.1", &[]);
        assert_eq!(status(g.clone(), Some("evil.example:8767"), "/health"), 403);
        assert_eq!(status(g.clone(), Some("evil.example"), "/stats"), 403);
        assert_eq!(status(g.clone(), Some("127.0.0.1:8767"), "/health"), 200);
        assert_eq!(status(g, Some("localhost:8767"), "/health"), 200);
    }

    // #1326 acceptance: a Docker-style bind works, and allowed_hosts is honoured.
    #[test]
    fn docker_style_bind_and_allowed_hosts() {
        let open = HostGuard::new("0.0.0.0", &[]);
        assert_eq!(status(open, Some("uteke:8767"), "/health"), 200);

        let listed = HostGuard::new("0.0.0.0", &["uteke".to_string()]);
        assert_eq!(status(listed.clone(), Some("uteke:8767"), "/health"), 200);
        assert_eq!(status(listed, Some("evil.example"), "/health"), 403);
    }
}

#[cfg(test)]
mod mcp_http_hardening_tests {
    use super::*;
    use tiny_http::{Header, TestRequest};

    fn app() -> Mutex<Uteke> {
        Mutex::new(
            Uteke::open_with_backend(":memory:", None)
                .expect("open in-memory uteke without embedder"),
        )
    }

    fn ctx(origins: Vec<String>) -> ReqCtx {
        ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: origins,
            recall_config: None,
            extraction_config: None,
            host_guard: Default::default(),
        }
    }

    // #1328: an oversized /mcp body is a 413, not a truncated JSON parse error.
    #[test]
    fn mcp_oversized_body_is_413() {
        let big: &'static str = Box::leak("x".repeat(1024 * 1024 + 8).into_boxed_str());
        let mut req = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/mcp")
            .with_body(big)
            .into();
        let resp = route(&app(), &ctx(Vec::new()), &mut req);
        assert_eq!(resp.status_code().0, 413);
    }

    // #1328: /mcp responses carry CORS headers like every other endpoint.
    #[test]
    fn mcp_response_has_cors_headers() {
        let origin = "https://app.example";
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let mut req = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/mcp")
            .with_header(Header::from_bytes(&b"Origin"[..], origin.as_bytes()).unwrap())
            .with_body(body)
            .into();
        let resp = route(&app(), &ctx(vec![origin.to_string()]), &mut req);
        assert!(
            resp.headers()
                .iter()
                .any(|h| h.field.equiv("Access-Control-Allow-Origin")
                    && h.value.as_str() == origin),
            "missing ACAO on /mcp: {:?}",
            resp.headers()
        );
    }
}

// #1439: /mcp is open to read-only tokens; the per-tool gate inside uteke-mcp
// refuses the write tools, write tokens are unchanged.
#[cfg(test)]
mod mcp_read_only_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tiny_http::TestRequest;

    fn ro_ctx() -> ReqCtx {
        ReqCtx {
            auth_token_hash: Some(Sha256::digest(b"rw-secret").into()),
            read_only_token_hash: Some(Sha256::digest(b"ro-secret").into()),
            cors_origins: Vec::new(),
            recall_config: None,
            extraction_config: None,
            host_guard: Default::default(),
        }
    }

    fn mcp(uteke: &Mutex<Uteke>, token: &str, tool: &str, args: &str) -> (u16, serde_json::Value) {
        let body: &'static str = Box::leak(
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
            )
            .into_boxed_str(),
        );
        let auth: &'static str = Box::leak(format!("Bearer {token}").into_boxed_str());
        let mut req = TestRequest::new()
            .with_method(Method::Post)
            .with_path("/mcp")
            .with_header(tiny_http::Header::from_bytes("Authorization", auth).unwrap())
            .with_body(body)
            .into();
        let resp = route(uteke, &ro_ctx(), &mut req);
        let status = resp.status_code().0;
        let bytes = resp.into_reader().into_inner();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn store() -> Mutex<Uteke> {
        Mutex::new(Uteke::open_with_backend(":memory:", None).unwrap())
    }

    #[test]
    fn read_only_token_reaches_read_tools() {
        let u = store();
        let (status, json) = mcp(&u, "ro-secret", "uteke_stats", "{}");
        assert_eq!(status, 200, "{json}");
        assert!(json.get("error").is_none(), "{json}");
        assert_ne!(json["result"]["isError"], true, "{json}");
    }

    #[test]
    fn read_only_token_is_refused_write_tools() {
        let u = store();
        let (status, json) = mcp(&u, "ro-secret", "uteke_remember", r#"{"content":"x"}"#);
        assert_eq!(status, 200, "{json}");
        assert_eq!(json["result"]["isError"], true, "{json}");
        assert_eq!(
            json["result"]["content"][0]["text"],
            "Read-only token cannot use uteke_remember"
        );
        assert_eq!(u.lock().unwrap().count(None).unwrap(), 0);
    }

    #[test]
    fn write_token_is_unchanged() {
        let u = store();
        let (status, json) = mcp(&u, "rw-secret", "uteke_remember", r#"{"content":"x"}"#);
        assert_eq!(status, 200, "{json}");
        assert!(json.get("error").is_none(), "{json}");
        assert_ne!(json["result"]["isError"], true, "{json}");
        assert_eq!(u.lock().unwrap().count(None).unwrap(), 1);
    }
}

#[cfg(test)]
mod namespace_deprecation_tests {
    use super::*;
    use tiny_http::TestRequest;

    fn call(
        uteke: &Mutex<Uteke>,
        method: Method,
        url: &str,
        body: serde_json::Value,
    ) -> (u16, String) {
        let leaked: &'static str = Box::leak(body.to_string().into_boxed_str());
        let mut req: Request = TestRequest::new()
            .with_method(method)
            .with_path(url)
            .with_body(leaked)
            .into();
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: Vec::new(),
            recall_config: None,
            extraction_config: None,
            host_guard: Default::default(),
        };
        let resp = route(uteke, &ctx, &mut req);
        let status = resp.status_code().0;
        let bytes = resp.into_reader().into_inner();
        (status, String::from_utf8(bytes).unwrap())
    }

    fn app() -> Mutex<Uteke> {
        Mutex::new(Uteke::open_with_backend(":memory:", None).expect("open without embedder"))
    }

    fn warning(raw: &str) -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(raw).unwrap()["warning"].clone()
    }

    #[test]
    fn remember_non_default_namespace_carries_warning() {
        let u = app();
        let (st, raw) = call(
            &u,
            Method::Post,
            "/remember",
            serde_json::json!({"content": "x", "namespace": "ns-http-remember"}),
        );
        assert_eq!(st, 200, "{raw}");
        let w = warning(&raw);
        let w = w.as_str().expect("warning is a string");
        assert!(w.contains(uteke_core::namespace_deprecation::NAMESPACE_DEPRECATION_WARNING));
    }

    #[test]
    fn remember_default_or_omitted_namespace_has_no_deprecation() {
        let u = app();
        for body in [
            serde_json::json!({"content": "a"}),
            serde_json::json!({"content": "b", "namespace": "default"}),
        ] {
            let (st, raw) = call(&u, Method::Post, "/remember", body);
            assert_eq!(st, 200, "{raw}");
            let w = warning(&raw);
            assert!(
                w.is_null() || !w.as_str().unwrap().contains("Namespaces are deprecated"),
                "{raw}"
            );
            // Same keys as before the change: warning stays a key, no new ones.
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
            let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            assert_eq!(keys, ["author_type", "embedding_written", "id", "warning"]);
        }
    }

    #[test]
    fn room_create_warning_only_for_non_default() {
        let u = app();
        let (_, raw) = call(
            &u,
            Method::Post,
            "/room/create",
            serde_json::json!({"room_id": "r-default"}),
        );
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(v.get("warning").is_none(), "{raw}");
        assert_eq!(v["namespace"], "default");

        let (_, raw) = call(
            &u,
            Method::Post,
            "/room/create",
            serde_json::json!({"room_id": "r-ns", "namespace": "ns-http-room"}),
        );
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            v["warning"],
            uteke_core::namespace_deprecation::NAMESPACE_DEPRECATION_WARNING
        );
        assert_eq!(v["created"], "r-ns");
    }

    #[test]
    fn with_namespace_warning_is_identity_for_default() {
        let body = serde_json::json!({"imported": 1, "skipped": 0});
        assert_eq!(with_namespace_warning(body.clone(), None), body);
        assert_eq!(with_namespace_warning(body.clone(), Some("default")), body);
        let w = with_namespace_warning(body, Some("other"));
        assert!(w["warning"].is_string());
        assert_eq!(w["imported"], 1);
    }
}

// ── #1343: `DELETE /forget` goes through the typed forget request ──────────

#[cfg(test)]
mod forget_route_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use uteke_core::memory::types::Memory;

    const ID_A: &str = "abcd0000-0000-4000-8000-00000000000a";
    const ID_B: &str = "abcd0000-0000-4000-8000-00000000000b";
    const ID_C: &str = "cafe0000-0000-4000-8000-00000000000c";

    fn memory(id: &str, ns: &str, tag: &str) -> Memory {
        let now = chrono::Utc::now();
        Memory {
            id: id.to_string(),
            content: format!("forget route probe {id}"),
            embedding: vec![0.21; 768],
            tags: vec![tag.to_string()],
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
            namespace: ns.to_string(),
            access_count: 0,
            recall_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
            author_type: "agent".to_string(),
        }
    }

    fn app() -> Mutex<Uteke> {
        let uteke = Uteke::open_with_backend(":memory:", None)
            .expect("open in-memory uteke without embedder");
        uteke.store().insert(&memory(ID_A, "ns-a", "t1")).unwrap();
        uteke.store().insert(&memory(ID_B, "ns-a", "t1")).unwrap();
        uteke.store().insert(&memory(ID_C, "ns-b", "t2")).unwrap();
        Mutex::new(uteke)
    }

    fn call(
        uteke: &Mutex<Uteke>,
        method: Method,
        url: &str,
        bearer: Option<&str>,
        tokens: bool,
    ) -> (u16, serde_json::Value) {
        let mut builder = tiny_http::TestRequest::new()
            .with_method(method)
            .with_path(url);
        if let Some(h) = bearer {
            builder =
                builder.with_header(tiny_http::Header::from_bytes("Authorization", h).unwrap());
        }
        let mut req: tiny_http::Request = builder.into();
        let ctx = ReqCtx {
            auth_token_hash: tokens.then(|| Sha256::digest("rw").into()),
            read_only_token_hash: tokens.then(|| Sha256::digest("ro").into()),
            cors_origins: Vec::new(),
            recall_config: None,
            extraction_config: None,
            host_guard: Default::default(),
        };
        let resp = route(uteke, &ctx, &mut req);
        let status = resp.status_code().0;
        let bytes = resp.into_reader().into_inner();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    fn del(uteke: &Mutex<Uteke>, url: &str) -> (u16, serde_json::Value) {
        call(uteke, Method::Delete, url, None, false)
    }

    #[test]
    fn forget_by_full_id() {
        let u = app();
        let (s, body) = del(&u, &format!("/forget?id={ID_A}"));
        assert_eq!((s, &body), (200, &serde_json::json!({"forgotten": ID_A})));
    }

    #[test]
    fn forget_by_unambiguous_short_prefix() {
        let u = app();
        let (s, body) = del(&u, "/forget?id=cafe");
        assert_eq!((s, &body), (200, &serde_json::json!({"forgotten": ID_C})));
    }

    #[test]
    fn error_statuses_and_texts_are_unchanged() {
        let u = app();
        let (s, body) = del(&u, "/forget?id=ffff");
        assert_eq!(s, 404);
        assert_eq!(body["error"], "Memory not found: ffff");
        let absent = "11111111-1111-4111-8111-111111111111";
        let (s, body) = del(&u, &format!("/forget?id={absent}"));
        assert_eq!(s, 404);
        assert_eq!(body["error"], format!("Memory not found: {absent}"));
        let (s, body) = del(&u, "/forget?id=abcd");
        assert_eq!(s, 400);
        assert_eq!(
            body["error"],
            "Validation error: Ambiguous ID prefix 'abcd' — matches 2 memories. Use a longer prefix."
        );
        for url in ["/forget", "/forget?namespace=ns-a", "/forget?flag"] {
            let (s, body) = del(&u, url);
            assert_eq!(s, 400, "{url}");
            assert_eq!(body["error"], "Provide ?id= or ?tag= parameter", "{url}");
        }
    }

    #[test]
    fn id_wins_over_tag() {
        let u = app();
        let (s, body) = del(&u, &format!("/forget?tag=t1&id={ID_C}"));
        assert_eq!((s, &body), (200, &serde_json::json!({"forgotten": ID_C})));
        // The tag was not applied: the `t1` memories are untouched.
        assert!(
            !u.lock()
                .unwrap()
                .get_by_id(ID_A)
                .unwrap()
                .unwrap()
                .deprecated
        );
    }

    #[test]
    fn tag_forget_honours_namespace_and_none_means_default() {
        let u = app();
        // Wrong namespace: nothing matches.
        let (s, body) = del(&u, "/forget?tag=t1&namespace=ns-b");
        assert_eq!(s, 200);
        assert_eq!(body["deleted"], 0);
        let (s, body) = del(&u, "/forget?tag=t1&namespace=ns-a");
        assert_eq!(s, 200);
        assert_eq!(body["deleted"], 2);
        // No namespace means the default namespace, NOT every namespace:
        // `t2` lives in `ns-b`, so it is untouched until ns-b is named.
        let (s, body) = del(&u, "/forget?tag=t2");
        assert_eq!(s, 200);
        assert_eq!(body["deleted"], 0);
        let (s, body) = del(&u, "/forget?tag=t2&namespace=ns-b");
        assert_eq!(s, 200);
        assert_eq!(body["deleted"], 1);
    }

    #[test]
    fn read_only_token_cannot_forget() {
        let u = app();
        for url in [format!("/forget?id={ID_C}"), "/forget?tag=t2".to_string()] {
            let (s, body) = call(&u, Method::Delete, &url, Some("Bearer ro"), true);
            assert_eq!(s, 403, "{url}");
            assert_eq!(
                body["error"],
                "Read-only token cannot perform write operations"
            );
        }
        // Nothing was deleted by the blocked requests.
        assert!(
            !u.lock()
                .unwrap()
                .get_by_id(ID_C)
                .unwrap()
                .unwrap()
                .deprecated
        );
        // The write token still can.
        let (s, _) = call(
            &u,
            Method::Delete,
            &format!("/forget?id={ID_C}"),
            Some("Bearer rw"),
            true,
        );
        assert_eq!(s, 200);
        // Recorded disagreement: the gate is HTTP-only.
        let may = |p: uteke_core::ForgetPolicy| p.read_only_token_may_forget;
        assert_eq!(
            (
                may(uteke_core::ForgetPolicy::HTTP),
                may(uteke_core::ForgetPolicy::MCP)
            ),
            (false, true)
        );
    }

    #[test]
    fn http_and_mcp_resolve_the_same_ids_to_the_same_memory() {
        let u = app();
        let guard = u.lock().unwrap();
        let resolve = |input: &str, p| uteke_core::resolve_memory_id(&guard, input, p);
        for input in [ID_A, "cafe", ID_B] {
            let http = resolve(input, uteke_core::ForgetPolicy::HTTP);
            let mcp = resolve(input, uteke_core::ForgetPolicy::MCP);
            assert_eq!(http.unwrap(), mcp.unwrap(), "{input}");
        }
        // Where they disagree, each keeps its own text (pinned).
        let http = resolve("ffff", uteke_core::ForgetPolicy::HTTP);
        let mcp = resolve("ffff", uteke_core::ForgetPolicy::MCP);
        assert_eq!(http.unwrap_err().message(), "Memory not found: ffff");
        assert_eq!(
            mcp.unwrap_err().message(),
            "No memory matches id prefix 'ffff'"
        );
    }
}
