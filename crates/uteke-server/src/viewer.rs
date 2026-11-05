//! `uteke-serve --viewer`: the read-only browser viewer behind `uteke view` (#1470 b).
//!
//! A separate loopback listener, not a route of the API server. It serves one
//! static page plus the few read routes the page needs, and nothing else:
//!
//! - every request needs the per-launch token (cookie, `X-Uteke-Viewer-Token`
//!   or bearer; the launch URL carries it once as `?token=` on `/`), else 401;
//!   on a non-loopback bind the bearer slot is the server auth token instead,
//!   also required on every request;
//! - the store is opened read-only (no migration, no writes);
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
use crate::types::parse_query_param;

const PAGE: &str = include_str!("viewer/index.html");
const TOKENS_CSS: &str = include_str!("viewer/tokens.css");

/// Most turns one session view returns, and most sessions the list returns.
const SESSION_TURN_LIMIT: usize = 1000;
const SESSION_LIST_LIMIT: usize = 200;

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
    /// The server auth token, required on every request as `Authorization:
    /// Bearer` in addition to the launch token. Set for a non-loopback bind only.
    auth_hash: Option<[u8; 32]>,
}

impl Gate {
    pub fn new(token: &str, port: u16) -> Self {
        Self {
            token_hash: Sha256::digest(token.as_bytes()).into(),
            port,
            auth_hash: None,
        }
    }

    /// Also require the server auth token (bearer) on every request.
    pub fn with_auth_token(mut self, auth_token: &str) -> Self {
        self.auth_hash = Some(Sha256::digest(auth_token.as_bytes()).into());
        self
    }

    /// Whether the request carries the server auth token as a bearer. Always
    /// true when none is required.
    fn auth_ok(&self, req: &Request) -> bool {
        let Some(want) = &self.auth_hash else {
            return true;
        };
        let given = bearer(req).map(|t| Sha256::digest(t.as_bytes()));
        given.is_some_and(|g| {
            let g: [u8; 32] = g.into();
            context::constant_time_eq_digest(&g, want)
        })
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

/// The token of an `Authorization: Bearer <token>` header.
fn bearer(req: &Request) -> Option<String> {
    let v = header_value(req, "Authorization")?;
    let mut parts = v.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(scheme), Some(t), None) if scheme.eq_ignore_ascii_case("Bearer") => {
            Some(t.to_string())
        }
        _ => None,
    }
}

/// The launch token a request presents and whether it came from the URL query.
/// `bearer_is_launch` is false when the bearer slot carries the server auth
/// token instead (non-loopback bind).
fn presented_token(
    req: &Request,
    route_path: &str,
    bearer_is_launch: bool,
) -> Option<(String, bool)> {
    // Only the page itself may take the token from the URL: the launch link. It is
    // read first, so a stale cookie from an earlier launch (cookies ignore
    // ports) cannot shadow the fresh token.
    if route_path == "/"
        && let Some(q) = req.url().split_once('?').map(|(_, q)| q)
    {
        for pair in q.split('&') {
            if let Some(t) = pair.strip_prefix("token=") {
                return Some((t.to_string(), true));
            }
        }
    }
    if let Some(t) = header_value(req, TOKEN_HEADER) {
        return Some((t.trim().to_string(), false));
    }
    if bearer_is_launch && let Some(t) = bearer(req) {
        return Some((t, false));
    }
    if let Some(cookies) = header_value(req, "Cookie") {
        for c in cookies.split(';') {
            if let Some(t) = c.trim().strip_prefix(&format!("{COOKIE}=")) {
                return Some((t.to_string(), false));
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

    let Some((token, from_query)) = presented_token(req, &route_path, gate.auth_hash.is_none())
    else {
        return error(401, "Viewer token required");
    };
    if !gate.accepts(&token) {
        return error(401, "Invalid viewer token");
    }
    if !gate.auth_ok(req) {
        return error(401, "Server auth token required (Authorization: Bearer)");
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
        "/viewer/sessions" => session_list(uteke),
        "/viewer/session" => {
            let id = url
                .split_once('?')
                .and_then(|(_, q)| parse_query_param(q, "id"))
                .unwrap_or_default();
            session_detail(uteke, &id)
        }
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

fn json<T: serde::Serialize>(value: &T) -> Response<Cursor<Vec<u8>>> {
    match serde_json::to_vec(value) {
        Ok(body) => reply(200, "application/json", CSP_NONE, body),
        Err(_) => error(500, "Internal server error"),
    }
}

/// `GET /viewer/sessions`: captured sessions, most recently active first.
fn session_list(uteke: &Mutex<Uteke>) -> Response<Cursor<Vec<u8>>> {
    let u = uteke.lock().unwrap_or_else(|e| e.into_inner());
    match u.capture_sessions(SESSION_LIST_LIMIT) {
        Ok(rows) => json(&rows),
        Err(e) => {
            error!("viewer sessions: {e}");
            error(500, "Internal server error")
        }
    }
}

/// `GET /viewer/session?id=`: one session's turns in `seq` order and the
/// memories whose source points back at it.
fn session_detail(uteke: &Mutex<Uteke>, id: &str) -> Response<Cursor<Vec<u8>>> {
    if id.is_empty() {
        return error(400, "id is required");
    }
    let u = uteke.lock().unwrap_or_else(|e| e.into_inner());
    let turns = u.session_timeline(id, SESSION_TURN_LIMIT);
    let memories = u.session_memories(id);
    match (turns, memories) {
        (Ok(turns), Ok(memories)) => {
            json(&serde_json::json!({ "session_id": id, "turns": turns, "memories": memories }))
        }
        (Err(e), _) | (_, Err(e)) => {
            error!("viewer session: {e}");
            error(500, "Internal server error")
        }
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

/// Why a bind is refused, if it is: a non-loopback host needs the server auth token.
fn check_bind(host: &str, auth_token: Option<&str>) -> Result<(), String> {
    if !context::is_loopback_host(host) && auth_token.is_none_or(str::is_empty) {
        return Err(format!(
            "Refusing to bind the viewer to non-loopback address {host}: set the server auth \
             token (--auth-token or UTEKE_AUTH_TOKEN) first."
        ));
    }
    Ok(())
}

/// The host to print in the launch URL: the bind address, except that a
/// wildcard (`0.0.0.0`, `::`), which no browser can open, becomes loopback, and
/// an IPv6 literal gets its brackets.
fn launch_host(bind: &str) -> String {
    let bare = bind.trim().trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<std::net::IpAddr>() {
        Ok(ip) if ip.is_unspecified() && ip.is_ipv6() => "[::1]".to_string(),
        Ok(ip) if ip.is_unspecified() => "127.0.0.1".to_string(),
        Ok(ip) if ip.is_ipv6() => format!("[{bare}]"),
        _ => bare.to_string(),
    }
}

/// Run the viewer until interrupted. Returns the process exit code.
pub fn run(opts: Options) -> i32 {
    if let Err(msg) = check_bind(&opts.host, opts.auth_token.as_deref()) {
        eprintln!("{msg}");
        return 1;
    }
    let remote = !context::is_loopback_host(&opts.host);
    let home = match uteke_core::uteke_home() {
        Ok(h) => h,
        Err(e) => {
            error!("Failed to determine home directory: {e}");
            return 1;
        }
    };
    let db_path = home.join("uteke.db").to_string_lossy().to_string();
    // Read-only and without an embedder: the viewer only reads rows, so it
    // never writes the store, migrates it, or loads the model (#1667). An older
    // schema is refused with a message instead of being upgraded from here.
    let uteke = match Uteke::open_read_only(&db_path) {
        Ok(u) => Mutex::new(u),
        Err(e) => {
            eprintln!("Cannot open the store for viewing: {e}");
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
    let mut gate = Gate::new(&token, port);
    if remote && let Some(t) = opts.auth_token.as_deref() {
        gate = gate.with_auth_token(t);
    }
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
    println!("http://{}:{port}/?token={token}", launch_host(&opts.host));
    if remote {
        eprintln!(
            "Warning: the viewer speaks plain HTTP on {}:{port}. Every request also needs the \
             server auth token as `Authorization: Bearer`; terminate TLS in front (a reverse \
             proxy that adds the header) before using it beyond a trusted network.",
            opts.host
        );
    }
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
    use uteke_core::capture::Role;

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
    fn a_stale_cookie_does_not_shadow_the_launch_token() {
        let (uteke, ctx, gate) = app();
        let mut req = TestRequest::new()
            .with_method(Method::Get)
            .with_path("/?token=t0ken")
            .with_header(hdr("Host", "127.0.0.1:8768"))
            .with_header(hdr("Cookie", "uteke_viewer=left-over-from-another-launch"))
            .into();
        assert_eq!(route(&uteke, &ctx, &gate, &mut req).status_code().0, 200);
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

    fn json_of(r: Response<Cursor<Vec<u8>>>) -> serde_json::Value {
        serde_json::from_str(&body(r)).expect("json body")
    }

    fn send(
        uteke: &Mutex<Uteke>,
        method: Method,
        path: &str,
        token: Option<&str>,
    ) -> Response<Cursor<Vec<u8>>> {
        let (_, ctx, gate) = app();
        let mut t = TestRequest::new()
            .with_method(method)
            .with_path(path)
            .with_header(hdr("Host", "127.0.0.1:8768"));
        if let Some(tok) = token {
            t = t.with_header(hdr(TOKEN_HEADER, tok));
        }
        let mut req = t.into();
        route(uteke, &ctx, &gate, &mut req)
    }

    #[test]
    fn session_routes_need_the_token_and_refuse_writes() {
        let (uteke, _, _) = app();
        for path in ["/viewer/sessions", "/viewer/session?id=s1"] {
            assert_eq!(send(&uteke, Method::Get, path, None).status_code().0, 401);
            assert_eq!(
                send(&uteke, Method::Post, path, Some(TOKEN))
                    .status_code()
                    .0,
                405
            );
            assert_eq!(
                send(&uteke, Method::Delete, path, Some(TOKEN))
                    .status_code()
                    .0,
                405
            );
        }
        let bad_host = {
            let (_, ctx, gate) = app();
            let mut req = TestRequest::new()
                .with_method(Method::Get)
                .with_path("/viewer/sessions")
                .with_header(hdr("Host", "evil.example:8768"))
                .with_header(hdr(TOKEN_HEADER, TOKEN))
                .into();
            route(&uteke, &ctx, &gate, &mut req)
        };
        assert_eq!(bad_host.status_code().0, 403);
        assert_eq!(
            send(&uteke, Method::Get, "/viewer/session", Some(TOKEN))
                .status_code()
                .0,
            400
        );
    }

    #[test]
    fn the_timeline_lists_turns_in_seq_order_and_links_memories_back() {
        let (uteke, _, _) = app();
        {
            let u = uteke.lock().unwrap();
            u.capture(
                "sess one",
                Role::User,
                "<script>alert(1)</script>",
                None,
                None,
            )
            .unwrap();
            u.capture("sess one", Role::Assistant, "done", None, None)
                .unwrap();
            u.capture("other", Role::User, "elsewhere", None, None)
                .unwrap();
            let id = u
                .remember("a fact from the session", &[], None, None)
                .unwrap();
            u.store_for_tests()
                .conn
                .execute(
                    "UPDATE memories SET source = 'capture:sess one:batch0001' WHERE id = ?1",
                    [&id],
                )
                .unwrap();
        }
        let list = json_of(send(&uteke, Method::Get, "/viewer/sessions", Some(TOKEN)));
        assert_eq!(list.as_array().unwrap().len(), 2);

        // the id travels percent-encoded, as the page sends it
        let detail = json_of(send(
            &uteke,
            Method::Get,
            "/viewer/session?id=sess%20one",
            Some(TOKEN),
        ));
        let turns = detail["turns"].as_array().unwrap();
        let seqs: Vec<i64> = turns.iter().map(|t| t["seq"].as_i64().unwrap()).collect();
        assert_eq!(seqs, [1, 2]);
        // stored markup comes back verbatim as a JSON string, for textContent
        assert_eq!(turns[0]["content"], "<script>alert(1)</script>");
        let mems = detail["memories"].as_array().unwrap();
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0]["content"], "a fact from the session");
    }

    #[test]
    fn tokens_are_long_and_distinct() {
        let a = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, new_token());
    }

    // --- #1667: bind guard and read-only store ---

    fn call_with_auth(auth: Option<&str>, token: Option<&str>) -> u16 {
        let (uteke, mut ctx, _) = app();
        ctx.host_guard = HostGuard::new("0.0.0.0", &[]);
        let gate = Gate::new(TOKEN, PORT).with_auth_token("srv-secret");
        let mut t = TestRequest::new()
            .with_method(Method::Get)
            .with_path("/health")
            .with_header(hdr("Host", "10.0.0.5:8768"));
        if let Some(tok) = token {
            t = t.with_header(hdr(TOKEN_HEADER, tok));
        }
        if let Some(a) = auth {
            t = t.with_header(hdr("Authorization", &format!("Bearer {a}")));
        }
        let mut req = t.into();
        route(&uteke, &ctx, &gate, &mut req).status_code().0
    }

    #[test]
    fn a_non_loopback_bind_without_the_auth_token_is_refused() {
        assert!(check_bind("0.0.0.0", None).is_err());
        assert!(check_bind("192.168.1.9", Some("")).is_err());
        assert!(check_bind("0.0.0.0", Some("x")).is_ok());
        assert!(check_bind("127.0.0.1", None).is_ok());
        let code = run(Options {
            host: "0.0.0.0".to_string(),
            port: 0,
            auth_token: None,
            allowed_hosts: Vec::new(),
        });
        assert_eq!(code, 1);
    }

    #[test]
    fn with_the_auth_token_configured_every_request_needs_it() {
        assert_eq!(call_with_auth(Some("srv-secret"), Some(TOKEN)), 200);
        // Launch token alone is not enough.
        assert_eq!(call_with_auth(None, Some(TOKEN)), 401);
        assert_eq!(call_with_auth(Some("wrong"), Some(TOKEN)), 401);
        // The auth token alone is not enough either, and is not a launch token.
        assert_eq!(call_with_auth(Some("srv-secret"), None), 401);
    }

    #[test]
    fn the_launch_url_host_is_one_a_browser_can_open() {
        assert_eq!(launch_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(launch_host("::"), "[::1]");
        assert_eq!(launch_host("[::]"), "[::1]");
        assert_eq!(launch_host("192.168.1.9"), "192.168.1.9");
        assert_eq!(launch_host("2001:db8::1"), "[2001:db8::1]");
        assert_eq!(launch_host("viewer.lan"), "viewer.lan");
        assert_eq!(launch_host("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn the_viewer_handle_cannot_write() {
        let dir = std::env::temp_dir().join(format!("uteke-viewer-{}", uuid::Uuid::new_v4()));
        drop(Uteke::open_with_backend(&dir, None).unwrap());
        let ro = Uteke::open_read_only(&dir).expect("current store opens");
        assert!(ro.stats(None).is_ok());
        // A write through the handle fails: pinning needs a write transaction.
        assert!(ro.store_for_tests().pin("nope").is_err());
        // Every route the page reads still answers on the read-only handle.
        let (_, ctx, gate) = app();
        let uteke = Mutex::new(ro);
        for path in [
            "/health",
            "/namespaces",
            "/stats",
            "/room/list",
            "/recent",
            "/memory?id=x",
            "/viewer/sessions",
        ] {
            let mut req = TestRequest::new()
                .with_method(Method::Get)
                .with_path(path)
                .with_header(hdr("Host", "127.0.0.1:8768"))
                .with_header(hdr(TOKEN_HEADER, TOKEN))
                .into();
            let status = route(&uteke, &ctx, &gate, &mut req).status_code().0;
            assert!(matches!(status, 200 | 400 | 404), "{path}: {status}");
        }
        drop(uteke);
        std::fs::remove_dir_all(&dir).ok();
    }
}
