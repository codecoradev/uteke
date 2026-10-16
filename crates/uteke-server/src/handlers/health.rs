//! Health, guide, route-introspection and store-identity handlers.

use super::*;

pub(super) fn get_health(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let health_anonymous = c.health_anonymous;
    // #1252: anonymous probe gets liveness only — no version, no
    // counts, no update info. Authenticated (or auth-disabled)
    // callers get the full payload, unchanged.
    if health_anonymous {
        #[derive(serde::Serialize)]
        struct MinimalHealthResponse {
            status: &'static str,
        }
        return ctx.ok_response_for(req, &MinimalHealthResponse { status: "ok" });
    }
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

pub(super) fn get_guide(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let ctx = c.ctx;
    let req = &mut *c.req;
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

// Machine-readable API spec: serialize the api_registry ENDPOINTS
// table — the same single source of truth docgen renders into
// docs/api-reference.md — so consumers can validate against the
// real contract instead of stale notes. GET is inherently
// read-only, so the ReadOnly token gate needs no extra entry.
pub(super) fn get_routes(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let ctx = c.ctx;
    let req = &mut *c.req;
    ctx.ok_response_for(req, &api_registry::ENDPOINTS)
}

/// `GET /store` (#1445): which store this server serves, as an id, never a path.
pub(super) fn get_store(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(serde::Serialize)]
    struct StoreResponse<'a> {
        id: &'a str,
    }
    ctx.ok_response_for(req, &StoreResponse { id: &ctx.store_id })
}
