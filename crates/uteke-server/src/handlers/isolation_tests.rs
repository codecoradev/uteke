//! Scope-isolation regression gate over HTTP (#1467, part 2).
//!
//! The same two-namespace store as the core suite (`seed_two_scopes`: records
//! of `alpha` and `beta` share one vocabulary) is queried through every read
//! route that takes a scope. Each memory ends in `of <namespace>`, so a response
//! scoped to one namespace must contain its marker and never the other's,
//! whatever the JSON shape. `every_read_route_is_covered_or_exempt` fails when
//! a read route is added to the registry without being listed here.

use super::*;
use serde_json::{Value, json};
use tiny_http::TestRequest;
use uteke_core::test_support::{ISOLATION_QUERY as Q, ISOLATION_SCOPES, seed_two_scopes};

struct App {
    uteke: Mutex<Uteke>,
}

impl App {
    fn new() -> Self {
        let u = seed_two_scopes();
        // The shared vocabulary makes remember link the seeded memories, so
        // none is an orphan; one unrelated memory per scope gives `/orphans`
        // something to return.
        for scope in ISOLATION_SCOPES {
            u.remember(
                &format!("solitary zebra fjord of {scope}"),
                &[],
                None,
                Some(scope),
            )
            .expect("remember orphan");
        }
        Self {
            uteke: Mutex::new(u),
        }
    }

    fn call(&self, method: &Method, url: &str, body: Option<&Value>) -> (u16, String) {
        let mut req: Request = match body {
            Some(b) => {
                let leaked: &'static str = Box::leak(b.to_string().into_boxed_str());
                TestRequest::new()
                    .with_method(method.clone())
                    .with_path(url)
                    .with_body(leaked)
                    .into()
            }
            None => TestRequest::new()
                .with_method(method.clone())
                .with_path(url)
                .into(),
        };
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: Vec::new(),
            recall_config: None,
            extraction_config: None,
            host_guard: Default::default(),
            store_id: String::new(),
        };
        let resp = route(&self.uteke, &ctx, &mut req);
        let status = resp.status_code().0;
        let bytes = resp.into_reader().into_inner();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }
}

/// One read request scoped to `scope`.
struct Surface {
    method: Method,
    /// Path without the query string, as in the registry.
    path: &'static str,
    /// Query string appended to the path, without the `?`.
    query: String,
    body: Option<Value>,
    /// The response must contain `of <scope>` (so the check is not vacuous).
    expects_hits: bool,
}

fn get(path: &'static str, query: &str, expects_hits: bool) -> Surface {
    Surface {
        method: Method::Get,
        path,
        query: query.to_string(),
        body: None,
        expects_hits,
    }
}

fn post(path: &'static str, body: Value, expects_hits: bool) -> Surface {
    Surface {
        method: Method::Post,
        path,
        query: String::new(),
        body: Some(body),
        expects_hits,
    }
}

fn surfaces(scope: &str) -> Vec<Surface> {
    let mut v = vec![
        post(
            "/search",
            json!({"query": Q, "namespace": scope, "limit": 20}),
            true,
        ),
        post("/list", json!({"namespace": scope, "limit": 50}), true),
        post(
            "/list",
            json!({"namespace": scope, "tag": "shared", "limit": 50}),
            true,
        ),
        get("/recent", &format!("namespace={scope}&limit=50"), true),
        get("/export", &format!("namespace={scope}"), true),
        get("/graph", &format!("namespace={scope}"), true),
        get(
            "/room/memories",
            &format!("room_id=room-{scope}&namespace={scope}"),
            true,
        ),
        post(
            "/room/recall",
            json!({"room_id": format!("room-{scope}"), "query": Q}),
            true,
        ),
        post(
            "/orphans",
            json!({"namespace": scope, "limit": 50, "threshold": 2.0}),
            true,
        ),
        // Names and counts: they must not list or count the other scope.
        get("/tags", &format!("namespace={scope}"), false),
        get("/room/list", &format!("namespace={scope}"), false),
        get("/stats", &format!("namespace={scope}"), false),
        post("/stats", json!({"namespace": scope}), false),
        // Documents are global by design (#1271): no memory, hence no marker.
        post(
            "/doc/search",
            json!({"query": Q, "namespace": scope}),
            false,
        ),
        post("/doc/list", json!({"namespace": scope}), false),
    ];
    for strategy in ["hybrid", "vector", "fts5", "graph", "fusion"] {
        for search_type in ["memory", "all"] {
            v.push(post(
                "/recall",
                json!({"query": Q, "namespace": scope, "limit": 20,
                       "strategy": strategy, "search_type": search_type}),
                true,
            ));
        }
    }
    v.push(post(
        "/recall",
        json!({"query": Q, "namespace": scope, "limit": 20, "pack": true, "budget_chars": 4000}),
        true,
    ));
    v.push(post(
        "/recall",
        json!({"query": Q, "namespace": scope, "limit": 20, "explain": true}),
        true,
    ));
    v.push(post(
        "/recall",
        json!({"query": Q, "namespace": scope, "limit": 20, "tags": ["shared"]}),
        true,
    ));
    v
}

#[test]
fn every_scoped_read_route_returns_only_its_own_namespace() {
    let app = App::new();
    for scope in ISOLATION_SCOPES {
        let foreign = if scope == "alpha" { "beta" } else { "alpha" };
        for s in surfaces(scope) {
            let url = if s.query.is_empty() {
                s.path.to_string()
            } else {
                format!("{}?{}", s.path, s.query)
            };
            let (status, text) = app.call(&s.method, &url, s.body.as_ref());
            let what = format!("{} {url} {:?}", s.method, s.body);
            let shown: String = text.chars().take(300).collect();
            assert_eq!(status, 200, "{what}: {text}");
            assert!(
                !text.contains(&format!("of {foreign}")),
                "{what} scoped to {scope} returned {foreign} records: {shown}"
            );
            assert!(
                !text.contains(&format!("project:{foreign}")),
                "{what} scoped to {scope} exposed the {foreign} project tag: {shown}"
            );
            if s.expects_hits {
                assert!(
                    text.contains(&format!("of {scope}")),
                    "{what} found no {scope} record, the check would be vacuous: {shown}"
                );
            }
        }
    }
}

#[test]
fn counts_are_per_namespace() {
    let app = App::new();
    for scope in ISOLATION_SCOPES {
        let (_, text) = app.call(&Method::Post, "/stats", Some(&json!({"namespace": scope})));
        let v: Value = serde_json::from_str(&text).unwrap();
        let count = v["total_memories"]
            .as_u64()
            .or_else(|| v["count"].as_u64())
            .or_else(|| v["memories"].as_u64())
            .unwrap_or_else(|| panic!("no count in {text}"));
        assert_eq!(count, 5, "{scope}: {text}");
    }
}

#[test]
fn a_foreign_room_asked_for_under_a_scope_yields_nothing() {
    let app = App::new();
    for scope in ISOLATION_SCOPES {
        let foreign = if scope == "alpha" { "beta" } else { "alpha" };
        let url = format!("/room/memories?room_id=room-{foreign}&namespace={scope}");
        let (status, text) = app.call(&Method::Get, &url, None);
        assert_eq!(status, 200, "{text}");
        assert!(
            !text.contains(&format!("of {foreign}")),
            "room-{foreign} leaked into {scope}: {text}"
        );
    }
}

/// Read routes the suite does not scope-check, each with the reason. Every
/// other read route of the registry must appear in `surfaces`.
const EXEMPT: &[(&str, &str, &str)] = &[
    ("GET", "/health", "liveness and counts, no records"),
    ("GET", "/guide", "static agent guide"),
    ("GET", "/routes", "the route table"),
    ("GET", "/store", "store identity hash (#1445)"),
    (
        "POST",
        "/mcp",
        "the MCP transport, covered by the MCP isolation suite",
    ),
    ("GET", "/namespaces", "lists namespace names by definition"),
    ("GET", "/memory", "fetch by ID: IDs are global handles"),
    ("POST", "/memory/doc-refs", "by memory ID"),
    ("GET", "/edges", "by memory ID"),
    ("GET", "/provenance", "by memory ID"),
    ("GET", "/timeline", "by memory ID"),
    ("POST", "/room/summary", "by room ID; the room is the scope"),
    (
        "POST",
        "/room/summary-document",
        "by room ID; the room is the scope",
    ),
    ("POST", "/room/stats", "by room ID; the room is the scope"),
    (
        "POST",
        "/room/document/list",
        "by room ID; documents are global",
    ),
    (
        "POST",
        "/doc/room/list",
        "by document slug; documents are global",
    ),
    ("POST", "/doc/get", "by document slug; documents are global"),
    (
        "POST",
        "/doc/mem-refs",
        "by document slug; documents are global",
    ),
    (
        "GET",
        "/contradictions",
        "audit ledger; filtered by `?namespace=` but empty in this fixture",
    ),
    (
        "GET",
        "/lifecycle/status",
        "counts per namespace; no records",
    ),
    (
        "GET",
        "/lifecycle/deprecated",
        "deprecated memories; none in this fixture",
    ),
];

/// A read route (GET, or a POST that does not write) added to the registry
/// must be listed in `surfaces` or in `EXEMPT` with a reason, so a new
/// retrieval surface cannot skip the isolation suite unnoticed.
#[test]
fn every_read_route_is_covered_or_exempt() {
    let covered: std::collections::BTreeSet<(String, String)> = surfaces("alpha")
        .iter()
        .map(|s| {
            (
                s.method.to_string().to_ascii_uppercase(),
                s.path.to_string(),
            )
        })
        .collect();
    let exempt: std::collections::BTreeSet<(String, String)> = EXEMPT
        .iter()
        .map(|(m, p, _)| (m.to_string(), p.to_string()))
        .collect();
    let reads: Vec<(String, String)> = crate::api_registry::ENDPOINTS
        .iter()
        .filter(|e| e.method == "GET" || !e.writes)
        .map(|e| (e.method.to_string(), e.path.to_string()))
        .collect();
    for row in &reads {
        assert!(
            covered.contains(row) || exempt.contains(row),
            "{} {} is a read route with no isolation check: add it to `surfaces` or to `EXEMPT` with a reason (#1467)",
            row.0,
            row.1
        );
    }
    for row in covered.iter().chain(exempt.iter()) {
        assert!(
            reads.contains(row),
            "{} {} is listed in the isolation suite but is not a read route of the registry",
            row.0,
            row.1
        );
    }
    assert!(
        covered.is_disjoint(&exempt),
        "a route is both covered and exempt"
    );
}
