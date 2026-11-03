//! `uteke-serve --viewer`: the read-only browser viewer behind `uteke view` (#1470 b).
//!
//! A separate loopback listener, not a route of the API server. It serves one
//! static page plus the few read routes the page needs, and nothing else:
//!
//! - every request needs the per-launch token (cookie, `X-Uteke-Viewer-Token`
//!   or bearer; the launch URL carries it once as `?token=` on `/`), else 401;
//! - the `Host` header goes through the same [`HostGuard`] the API uses, plus a
//!   port check, so a DNS-rebinding page gets 403;
//! - only `GET` is served (anything else is 405), and a `GET` is handed to the
//!   API router only when the registry row exists and has `writes = false`;
//! - no CORS headers are ever sent, whatever `uteke.toml` configures.
//!
//! The viewer paths (`/`, `/tokens.css`) are not API routes and are not in the
//! registry, so CORE/LAB are untouched.

use std::io::Cursor;
use std::sync::Mutex;

use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use tracing::{error, info, warn};
use uteke_core::Uteke;

use crate::api_registry;
use crate::context::{self, HostGuard, ReqCtx};
use crate::handlers;

const PAGE: &str = include_str!("viewer/index.html");
const TOKENS_CSS: &str = include_str!("viewer/tokens.css");

const COOKIE: &str = "uteke_viewer";
const TOKEN_HEADER: &str = "X-Uteke-Viewer-Token";

/// API routes the page may read. Each must also be a registry GET row with
/// `writes = false`, checked again at request time.
const READ_ROUTES: &[&str] = &[
    "/health",
    "/namespaces",
    "/stats",
    "/room/list",
    "/recent",
    "/memory",
];

/// Per-launch credentials and the port the listener is on.
pub struct Gate {
    token_hash: [u8; 32],
    port: u16,
}

impl Gate {
    pub fn new(token: &str, port: u16) -> Self {
        Self {
            token_hash: Sha256::digest(token.as_bytes()).into(),
            port,
        }
    }

    fn accepts(&self, token: &str) -> bool {
        let given: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        context::constant_time_eq_digest(&given, &self.token_hash)
    }
}

/// A fresh random token: two v4 UUIDs, 64 hex digits.
pub fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn header_value(req: &Request, name: &str) -> Option<String> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

/// The token a request presents and whether it came from the URL query.
fn presented_token(req: &Request, route_path: &str) -> Option<(String, bool)> {
    if let Some(t) = header_value(req, TOKEN_HEADER) {
        return Some((t.trim().to_string(), false));
    }
    if let Some(v) = header_value(req, "Authorization") {
        let mut parts = v.split_whitespace();
        if let (Some(scheme), Some(t), None) = (parts.next(), parts.next(), parts.next())
            && scheme.eq_ignore_ascii_case("Bearer")
        {
            return Some((t.to_string(), false));
        }
    }
    if let Some(cookies) = header_value(req, "Cookie") {
        for c in cookies.split(';') {
            if let Some(t) = c.trim().strip_prefix(&format!("{COOKIE}=")) {
                return Some((t.to_string(), false));
            }
        }
    }
    // Only the page itself may take the token from the URL: the launch link.
    if route_path == "/"
        && let Some(q) = req.url().split_once('?').map(|(_, q)| q)
    {
        for pair in q.split('&') {
            if let Some(t) = pair.strip_prefix("token=") {
                return Some((t.to_string(), true));
            }
        }
    }
    None
}

/// The port of a `Host` value, if it has one.
fn host_port(host: &str) -> Option<u16> {
    let rest = match host.strip_prefix('[') {
        Some(r) => r.split_once("]:")?.1,
        None => match host.split_once(':') {
            Some((_, p)) if !p.contains(':') => p,
            _ => return None,
        },
    };
    rest.parse().ok()
}

fn hdr(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

/// A response with the headers every viewer answer carries. No CORS headers.
fn reply(status: u16, content_type: &str, csp: &str, body: Vec<u8>) -> Response<Cursor<Vec<u8>>> {
    let mut r = Response::new(
        StatusCode::from(status),
        vec![
            hdr("Content-Type", content_type),
            hdr("Content-Security-Policy", csp),
            hdr("Cache-Control", "no-store"),
            hdr("X-Content-Type-Options", "nosniff"),
            hdr("Referrer-Policy", "no-referrer"),
            hdr("Cross-Origin-Resource-Policy", "same-origin"),
            hdr("X-Frame-Options", "DENY"),
        ],
        Cursor::new(body),
        None,
        None,
    );
    if status == 405 {
        r.add_header(hdr("Allow", "GET"));
    }
    r
}

const CSP_NONE: &str = "default-src 'none'";

fn error(status: u16, msg: &str) -> Response<Cursor<Vec<u8>>> {
    let body = serde_json::json!({ "error": msg }).to_string();
    reply(status, "application/json", CSP_NONE, body.into_bytes())
}

/// Handle one viewer request. Order matters: Host (403), token (401), method
/// (405), then the path.
pub fn route(
    uteke: &Mutex<Uteke>,
    ctx: &ReqCtx,
    gate: &Gate,
    req: &mut Request,
) -> Response<Cursor<Vec<u8>>> {
    let url = req.url().to_string();
    let route_path = url.split('?').next().unwrap_or(&url).to_string();

    let host = header_value(req, "Host");
    if !ctx.host_guard.allows(host.as_deref()) {
        return error(403, "Host not allowed");
    }
    if let Some(h) = &host
        && host_port(&h.trim().to_ascii_lowercase()) != Some(gate.port)
    {
        return error(403, "Host not allowed");
    }

    let Some((token, from_query)) = presented_token(req, &route_path) else {
        return error(401, "Viewer token required");
    };
    if !gate.accepts(&token) {
        return error(401, "Invalid viewer token");
    }

    if *req.method() != Method::Get {
        return error(405, "The viewer is read-only");
    }

    match route_path.as_str() {
        "/" => {
            let nonce = uuid::Uuid::new_v4().simple().to_string();
            let csp = format!(
                "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'self' 'nonce-{nonce}'; \
                 connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
            );
            let mut r = reply(
                200,
                "text/html; charset=utf-8",
                &csp,
                PAGE.replace("__NONCE__", &nonce).into_bytes(),
            );
            if from_query {
                r.add_header(hdr(
                    "Set-Cookie",
                    &format!("{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/"),
                ));
            }
            r
        }
        "/tokens.css" => reply(
            200,
            "text/css; charset=utf-8",
            CSP_NONE,
            TOKENS_CSS.as_bytes().to_vec(),
        ),
        p if READ_ROUTES.contains(&p)
            && api_registry::find("GET", p).is_some_and(|e| !e.writes) =>
        {
            // Authenticated here already; the API router sees an auth-less
            // context and, as a second line, no CORS origins.
            let inner = ReqCtx {
                auth_token_hash: None,
                read_only_token_hash: None,
                cors_origins: Vec::new(),
                ..ctx.clone()
            };
            let mut resp = handlers::route(uteke, &inner, req);
            for (k, v) in [
                ("Content-Security-Policy", CSP_NONE),
                ("Cache-Control", "no-store"),
                ("X-Content-Type-Options", "nosniff"),
                ("Referrer-Policy", "no-referrer"),
                ("Cross-Origin-Resource-Policy", "same-origin"),
            ] {
                resp.add_header(hdr(k, v));
            }
            resp
        }
        _ => error(404, "Not found"),
    }
}

/// What `uteke-serve --viewer` was asked for.
pub struct Options {
    pub host: String,
    pub port: u16,
    /// The existing server auth token, if any. A non-loopback bind needs it.
    pub auth_token: Option<String>,
    pub allowed_hosts: Vec<String>,
}

/// Run the viewer until interrupted. Returns the process exit code.
pub fn run(opts: Options) -> i32 {
    if !context::is_loopback_host(&opts.host) && opts.auth_token.is_none() {
        eprintln!(
            "Refusing to bind the viewer to non-loopback address {}: set the server auth token \
             (--auth-token or UTEKE_AUTH_TOKEN) first.",
            opts.host
        );
        return 1;
    }
    let home = match uteke_core::uteke_home() {
        Ok(h) => h,
        Err(e) => {
            error!("Failed to determine home directory: {e}");
            return 1;
        }
    };
    let db_path = home.join("uteke.db").to_string_lossy().to_string();
    // No embedder: the viewer only reads rows, so it skips the model load.
    let uteke = match Uteke::open_with_backend(&db_path, None) {
        Ok(u) => Mutex::new(u),
        Err(e) => {
            error!("Failed to open store: {e}");
            return 1;
        }
    };
    let server = match Server::http(format!("{}:{}", opts.host, opts.port)) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to bind {}:{}: {e}", opts.host, opts.port);
            return 1;
        }
    };
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(opts.port);
    let token = new_token();
    let gate = Gate::new(&token, port);
    let ctx = ReqCtx {
        auth_token_hash: None,
        read_only_token_hash: None,
        cors_origins: Vec::new(),
        recall_config: None,
        extraction_config: None,
        host_guard: HostGuard::new(&opts.host, &opts.allowed_hosts),
        store_id: uteke_core::store_identity(&home),
        embed_warmer: Default::default(),
    };

    // Printed on stdout so `uteke view` can relay it; never logged.
    println!("http://{}:{port}/?token={token}", opts.host);
    info!("Viewer listening on {}:{port} (read-only)", opts.host);

    ctrlc::set_handler(|| std::process::exit(0)).ok();
    for mut req in server.incoming_requests() {
        let routed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            route(&uteke, &ctx, &gate, &mut req)
        }));
        let response = routed.unwrap_or_else(|_| {
            warn!("viewer handler panicked");
            error(500, "Internal server error")
        });
        if let Err(e) = req.respond(response) {
            warn!("Response error: {e}");
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_http::TestRequest;

    const TOKEN: &str = "t0ken";
    const PORT: u16 = 8768;

    fn app() -> (Mutex<Uteke>, ReqCtx, Gate) {
        let uteke =
            Mutex::new(Uteke::open_with_backend(":memory:", None).expect("in-memory store"));
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            // A hostile config must not leak into the viewer.
            cors_origins: vec!["*".to_string()],
            recall_config: None,
            extraction_config: None,
            host_guard: HostGuard::new("127.0.0.1", &[]),
            store_id: String::new(),
            embed_warmer: Default::default(),
        };
        (uteke, ctx, Gate::new(TOKEN, PORT))
    }

    struct Call {
        method: Method,
        path: &'static str,
        host: Option<&'static str>,
        token: Option<&'static str>,
    }

    fn call(c: Call) -> Response<Cursor<Vec<u8>>> {
        let (uteke, ctx, gate) = app();
        let mut t = TestRequest::new().with_method(c.method).with_path(c.path);
        if let Some(h) = c.host {
            t = t.with_header(hdr("Host", h));
        }
        if let Some(tok) = c.token {
            t = t.with_header(hdr(TOKEN_HEADER, tok));
        }
        let mut req = t.into();
        route(&uteke, &ctx, &gate, &mut req)
    }

    fn get(
        path: &'static str,
        host: Option<&'static str>,
        token: Option<&'static str>,
    ) -> Response<Cursor<Vec<u8>>> {
        call(Call {
            method: Method::Get,
            path,
            host,
            token,
        })
    }

    fn body(r: Response<Cursor<Vec<u8>>>) -> String {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut r.into_reader(), &mut s).unwrap();
        s
    }

    const LOCAL: Option<&str> = Some("127.0.0.1:8768");

    #[test]
    fn no_token_is_401() {
        assert_eq!(get("/", LOCAL, None).status_code().0, 401);
        assert_eq!(get("/health", LOCAL, None).status_code().0, 401);
        assert_eq!(get("/recent", LOCAL, None).status_code().0, 401);
    }

    #[test]
    fn wrong_token_is_401() {
        assert_eq!(get("/health", LOCAL, Some("nope")).status_code().0, 401);
    }

    #[test]
    fn the_token_in_the_query_only_opens_the_page() {
        assert_eq!(get("/?token=t0ken", LOCAL, None).status_code().0, 200);
        assert_eq!(get("/health?token=t0ken", LOCAL, None).status_code().0, 401);
    }

    #[test]
    fn page_sets_a_strict_cookie_for_the_launch_link() {
        let r = get("/?token=t0ken", LOCAL, None);
        let cookie = r
            .headers()
            .iter()
            .find(|h| h.field.equiv("Set-Cookie"))
            .map(|h| h.value.as_str().to_string())
            .expect("cookie");
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    }

    #[test]
    fn foreign_host_is_403_even_with_the_token() {
        for host in [
            "evil.example:8768",
            "evil.example",
            "127.0.0.1:9999",
            "localhost:1",
        ] {
            assert_eq!(
                get("/health", Some(host), Some(TOKEN)).status_code().0,
                403,
                "{host}"
            );
        }
        // Host is checked before the token, so a rebinding page learns nothing.
        assert_eq!(
            get("/health", Some("evil.example:8768"), None)
                .status_code()
                .0,
            403
        );
    }

    #[test]
    fn loopback_hosts_with_the_port_are_accepted() {
        for host in ["127.0.0.1:8768", "localhost:8768"] {
            assert_eq!(
                get("/health", Some(host), Some(TOKEN)).status_code().0,
                200,
                "{host}"
            );
        }
    }

    #[test]
    fn every_non_get_method_is_refused() {
        for method in [Method::Post, Method::Put, Method::Delete, Method::Options] {
            for path in ["/", "/remember", "/forget", "/list", "/recall", "/health"] {
                let r = call(Call {
                    method: method.clone(),
                    path,
                    host: LOCAL,
                    token: Some(TOKEN),
                });
                assert_eq!(r.status_code().0, 405, "{method} {path}");
            }
        }
    }

    #[test]
    fn write_and_unlisted_routes_are_not_reachable() {
        for path in [
            "/remember",
            "/forget",
            "/export",
            "/doc/list",
            "/capture/status",
            "/mcp",
        ] {
            assert_eq!(get(path, LOCAL, Some(TOKEN)).status_code().0, 404, "{path}");
        }
    }

    #[test]
    fn read_routes_work_and_carry_no_cors_headers() {
        for path in ["/health", "/namespaces", "/stats", "/room/list", "/recent"] {
            let r = get(path, LOCAL, Some(TOKEN));
            assert_eq!(r.status_code().0, 200, "{path}");
            assert!(
                !r.headers().iter().any(|h| h
                    .field
                    .as_str()
                    .as_str()
                    .to_ascii_lowercase()
                    .starts_with("access-control")),
                "{path} sent CORS headers"
            );
        }
    }

    #[test]
    fn every_read_route_is_a_registry_read() {
        for p in READ_ROUTES {
            let e = api_registry::find("GET", p).unwrap_or_else(|| panic!("{p} not registered"));
            assert!(!e.writes, "{p} writes");
        }
    }

    #[test]
    fn page_is_nonce_locked_and_never_uses_inner_html() {
        let r = get("/", LOCAL, Some(TOKEN));
        let csp = r
            .headers()
            .iter()
            .find(|h| h.field.equiv("Content-Security-Policy"))
            .map(|h| h.value.as_str().to_string())
            .unwrap();
        assert!(csp.contains("default-src 'none'") && csp.contains("script-src 'nonce-"));
        assert!(!csp.contains("unsafe-inline"));
        let html = body(r);
        assert!(!html.contains("__NONCE__"));
        assert!(!html.contains("innerHTML") && !html.contains("insertAdjacentHTML"));
        assert!(!html.contains("document.write"));
    }

    #[test]
    fn stored_markup_comes_back_as_data_for_the_page_to_render_as_text() {
        let (uteke, ctx, gate) = app();
        let evil = "<script>alert(1)</script>";
        uteke
            .lock()
            .unwrap()
            .remember(evil, &[], None, None)
            .expect("store the memory");
        let mut req = TestRequest::new()
            .with_method(Method::Get)
            .with_path("/recent")
            .with_header(hdr("Host", "127.0.0.1:8768"))
            .with_header(hdr(TOKEN_HEADER, TOKEN))
            .into();
        let r = route(&uteke, &ctx, &gate, &mut req);
        let ct = r
            .headers()
            .iter()
            .find(|h| h.field.equiv("Content-Type"))
            .map(|h| h.value.as_str().to_string())
            .unwrap();
        assert_eq!(ct, "application/json");
        let nosniff = r
            .headers()
            .iter()
            .any(|h| h.field.equiv("X-Content-Type-Options") && h.value.as_str() == "nosniff");
        assert!(nosniff, "JSON must not be sniffed into HTML");
        // The markup travels verbatim as a JSON string; the page sets it with
        // textContent (no innerHTML, see the page test above), so it shows as text.
        assert!(body(r).contains(evil));
    }

    #[test]
    fn tokens_are_long_and_distinct() {
        let a = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, new_token());
    }
}
