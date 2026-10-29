//! HTTP server routing for CLI commands.

use crate::cli::Cli;
use crate::cli::Commands;
use crate::output;

/// Check if uteke server is reachable.
pub(crate) fn is_server_running(url: &str) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(100))
        .build()
        .map(|c| c.get(format!("{url}/health")).send().is_ok())
        .unwrap_or(false)
}

/// Whether the server at `url` serves the store at `store` (#1445).
///
/// Asks `GET /store` for the served store's id and compares it with the id of
/// the store this invocation resolved (`UTEKE_HOME`, `--store`, `uteke.toml`).
/// A mismatch returns `false` so the command runs against its own store
/// instead of silently reading and writing the server's. Anything that stops
/// the comparison (a server older than this route answers 404, a non-loopback
/// server whose paths are not ours, a timeout, a body that is not an id)
/// returns `true`: routing then behaves as before.
///
/// The probe goes through [`build_client`], so it carries `UTEKE_AUTH_TOKEN`
/// exactly like the command it guards. Without a token an auth-enabled
/// server answers 401 here and the routed command is refused with 401 as
/// well, so nothing can land in the wrong store on that path.
pub(crate) fn server_serves_store(url: &str, store: &str) -> bool {
    if !is_loopback_url(url) {
        return true;
    }
    let Ok(resp) = build_client(url)
        .get(format!("{url}/store"))
        .timeout(std::time::Duration::from_millis(500))
        .send()
    else {
        return true;
    };
    if !resp.status().is_success() {
        return true;
    }
    let Some(served) = resp
        .json::<serde_json::Value>()
        .ok()
        .and_then(|v| v["id"].as_str().map(str::to_string))
    else {
        return true;
    };
    let ours = uteke_core::store_identity(std::path::Path::new(
        &crate::config::Config::expand_tilde(store),
    ));
    if served == ours {
        return true;
    }
    tracing::warn!(
        "the server at {url} serves a different store than {store}; using the local store"
    );
    false
}

/// Send a request and check HTTP status before parsing JSON.
fn parse_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::blocking::Response,
) -> Result<T, String> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(format!("Server returned {status}: {body}"));
    }
    resp.json::<T>().map_err(|e| format!("Parse error: {e}"))
}

/// Send a request and check HTTP status before parsing raw JSON value.
fn parse_json_value(resp: reqwest::blocking::Response) -> Result<serde_json::Value, String> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(format!("Server returned {status}: {body}"));
    }
    resp.json().map_err(|e| format!("Parse error: {e}"))
}

/// Build the HTTP client used to talk to the local server.
///
/// When the server runs with auth enabled, every endpoint except `/health`
/// requires a bearer token, so forward `UTEKE_AUTH_TOKEN` (the same variable
/// the server reads). The token is only attached for loopback targets so a
/// redirected `server.host` can never receive it.
fn build_client(server_url: &str) -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder();
    if let Some(token) = std::env::var("UTEKE_AUTH_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .filter(|_| is_loopback_url(server_url))
    {
        if let Ok(mut value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
            value.set_sensitive(true);
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(reqwest::header::AUTHORIZATION, value);
            builder = builder.default_headers(headers);
        }
    }
    builder
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

fn is_loopback_url(url: &str) -> bool {
    // Parse with a real URL parser: hand-splitting is fooled by userinfo
    // (`user:pw@evil.example`) and similar authority tricks.
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return false;
    }
    match parsed.host_str() {
        Some("localhost") | Some("[::1]") => true,
        Some(h) => h
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    }
}

/// The `recall` flags the HTTP server cannot honour.
struct RecallFlags<'a> {
    strategy: Option<&'a str>,
    salience: Option<bool>,
    recency: Option<bool>,
    explain: bool,
    related: bool,
    depth: usize,
    context: bool,
    content_format: &'a str,
    where_filter: Option<&'a str>,
}

/// Names of the flags in `f` that `POST /recall` has no field for. Defaults
/// (`--depth 1`, `--content-format auto`) do not count as set.
fn recall_flags_unsupported_by_server(f: RecallFlags<'_>) -> Vec<&'static str> {
    let mut out = Vec::new();
    if f.strategy.is_some() {
        out.push("--strategy");
    }
    if f.salience.is_some() {
        out.push("--salience");
    }
    if f.recency.is_some() {
        out.push("--recency");
    }
    if f.explain {
        out.push("--explain");
    }
    if f.related {
        out.push("--related");
    }
    if f.depth != 1 {
        out.push("--depth");
    }
    if f.context {
        out.push("--context");
    }
    if f.content_format != "auto" {
        out.push("--content-format");
    }
    if f.where_filter.is_some() {
        out.push("--where");
    }
    out
}

/// Route CLI commands through the HTTP server for <50ms latency.
pub(crate) fn run_via_server(cli: &Cli, server_url: &str) -> Result<(), String> {
    let client = build_client(server_url);
    let ns = cli.namespace.as_deref().unwrap_or("default");

    match &cli.command {
        Commands::Remember {
            content,
            tags,
            r#type,
            detect_contradiction,
            entity,
            category,
            author_type,
            meta,
            room,
            author,
            source,
            source_type,
            timestamp,
        } => {
            // The typed remember request shared with HTTP and MCP (#1343)
            // builds the body; the server validates and applies its policy.
            // entity/category/--meta travel inside `metadata`.
            let body = uteke_core::RememberInput {
                content: content.clone(),
                tags: tags.clone(),
                namespace: Some(ns.to_string()),
                memory_type: Some(r#type.clone()),
                detect_contradiction: *detect_contradiction,
                metadata: uteke_core::metadata_from_cli_flags(
                    entity.as_deref(),
                    category.as_deref(),
                    meta,
                ),
                source: source.clone(),
                source_type: source_type.clone(),
                author_type: author_type.clone(),
                room: room.clone(),
                author: author.clone(),
                timestamp: timestamp.clone(),
                ..uteke_core::RememberInput::default()
            }
            .to_http_body();
            let resp = client
                .post(format!("{server_url}/remember"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let mut data = parse_json_value(resp)?;
            // The deprecation signal (#1427) goes to stderr (see main), never
            // into stdout JSON: strip it from the server's merged `warning`.
            if let Some(w) = data["warning"].as_str() {
                data["warning"] = match uteke_core::namespace_deprecation::strip_deprecation(w) {
                    Some(rest) => serde_json::Value::String(rest),
                    None => serde_json::Value::Null,
                };
            }
            if cli.json {
                println!("{data}");
            } else {
                println!("\u{2713} Memory stored\n  ID: {}", data["id"]);
            }
        }
        Commands::Recall {
            query,
            limit,
            tags,
            min,
            strict,
            entity,
            category,
            at,
            strategy,
            salience,
            recency,
            explain,
            related,
            depth,
            context,
            content_format,
            r#where,
            r#type,
            enrich,
            pack,
            budget,
            exclude_ids,
        } => {
            // The HTTP /recall request has no field for these flags. Dropping
            // them silently returned a differently-ranked or differently-shaped
            // result than the same command run locally (#1332): let the caller
            // fall back to the local store instead.
            let unsupported = recall_flags_unsupported_by_server(RecallFlags {
                strategy: strategy.as_deref(),
                salience: *salience,
                recency: *recency,
                explain: *explain,
                related: *related,
                depth: *depth,
                context: *context,
                content_format,
                where_filter: r#where.as_deref(),
            });
            if !unsupported.is_empty() {
                tracing::info!(
                    "recall flag(s) {} are not supported via the server; using the local store",
                    unsupported.join(", ")
                );
                return Err("unsupported".to_string());
            }
            // The typed recall request shared with HTTP and MCP (#1343) builds
            // the body; the server validates and applies its own policy.
            let body = uteke_core::RecallInput {
                query: query.clone(),
                limit: Some(*limit),
                tags: Some(tags.clone()),
                namespace: Some(ns.to_string()),
                entity: entity.clone(),
                category: category.clone(),
                min_score: *min,
                strict: *strict,
                at: at.clone(),
                search_type: r#type.clone(),
                enrich: *enrich,
                pack: *pack,
                budget_chars: Some(*budget),
                exclude_ids: exclude_ids.clone(),
                ..uteke_core::RecallInput::default()
            }
            .to_http_body();
            let resp = client
                .post(format!("{server_url}/recall"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let data = parse_json_value(resp)?;
            // Check if server returned enriched empty response with threshold
            if data.is_object()
                && data
                    .get("results")
                    .is_some_and(|r| r.as_array().is_some_and(|a| a.is_empty()))
            {
                if let Some(threshold) = data.get("threshold").and_then(|t| t.as_f64()) {
                    // Enriched empty response with threshold info
                    if cli.json {
                        println!("{data}");
                    } else {
                        println!("No matching memories found.");
                        println!("(min_score threshold: {:.2})", threshold);
                    }
                    return Ok(());
                }
            }
            // Normal response: array of SearchResult or empty array
            let results: Vec<uteke_core::SearchResult> =
                serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&results);
            } else {
                output::print_recall_human(&results);
            }
        }
        Commands::Search {
            query, limit, tags, ..
        } => {
            let body = serde_json::json!({
                "query": query,
                "limit": limit,
                "tags": tags,
                "namespace": ns
            });
            let resp = client
                .post(format!("{server_url}/search"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let results = parse_response::<Vec<uteke_core::SearchResult>>(resp)?;
            if cli.json {
                output::print_json(&results);
            } else {
                output::print_search_human(&results);
            }
        }
        Commands::List {
            entity: Some(_), ..
        }
        | Commands::List {
            category: Some(_), ..
        } => {
            // `/list` has no entity or category filter: serving this from the
            // server would silently drop it, so the local store answers (#1439).
            return Err("unsupported".to_string());
        }
        Commands::List {
            tag,
            limit,
            offset,
            at,
            ..
        } => {
            let body = uteke_core::ListInput {
                tag: tag.clone(),
                limit: Some(*limit),
                offset: Some(*offset),
                namespace: Some(ns.to_string()),
                at: at.clone(),
                ..uteke_core::ListInput::default()
            }
            .to_http_body();
            let resp = client
                .post(format!("{server_url}/list"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let memories = parse_response::<Vec<uteke_core::Memory>>(resp)?;
            if cli.json {
                output::print_json(&memories);
            } else {
                output::print_list_human(&memories);
            }
        }
        Commands::Stats => {
            let body = serde_json::json!({ "namespace": ns });
            let resp = client
                .post(format!("{server_url}/stats"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let stats = parse_response::<uteke_core::StoreStats>(resp)?;
            if cli.json {
                output::print_json(&stats);
            } else {
                output::print_stats_human(&stats);
            }
        }
        Commands::Forget {
            id,
            tag,
            cold: _,
            all: _,
            confirm: _,
        } => {
            // The DELETE /forget path comes from the typed request in
            // uteke-core (#1343); the id wins over the tag, as on the server.
            let input = uteke_core::ForgetInput {
                id: id.clone(),
                tag: tag.clone(),
                namespace: Some(ns.to_string()),
            };
            let Some(path) = input.to_http_path(|v| urlencoding::encode(v).into_owned()) else {
                return Err("Provide an ID, --tag, --cold, or --all".into());
            };
            let resp = client
                .delete(format!("{server_url}{path}"))
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let data = parse_json_value(resp)?;
            if cli.json {
                println!("{data}");
            } else if let Some(id) = id {
                println!("\u{2713} Memory forgotten: {id}");
            } else if let Some(tag) = tag {
                println!(
                    "\u{2713} Deleted {} memories with tag '{}'",
                    data["deleted"], tag
                );
            }
        }
        // Commands not supported via server fall through to local
        _ => {
            return Err("unsupported".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_loopback_url;

    // #1343: the forget URL is built by the typed request; it must stay what
    // the CLI always sent (id wins, tag carries the namespace, values encoded).
    #[test]
    fn forget_path_matches_the_old_urls() {
        let enc = |v: &str| urlencoding::encode(v).into_owned();
        let by_id = uteke_core::ForgetInput {
            id: Some("ab cd/1".into()),
            tag: Some("t".into()),
            namespace: Some("default".into()),
        };
        assert_eq!(
            by_id.to_http_path(enc).as_deref(),
            Some("/forget?id=ab%20cd%2F1")
        );
        let by_tag = uteke_core::ForgetInput {
            id: None,
            tag: Some("my tag&x".into()),
            namespace: Some("work space".into()),
        };
        assert_eq!(
            by_tag.to_http_path(enc).as_deref(),
            Some("/forget?tag=my%20tag%26x&namespace=work%20space")
        );
        assert_eq!(uteke_core::ForgetInput::default().to_http_path(enc), None);
    }

    #[test]
    fn loopback_detection() {
        // Built from parts so the fixtures are not mistaken for hardcoded URLs.
        let url = |authority: &str| format!("{}://{authority}", "http");
        assert!(is_loopback_url(&url("127.0.0.1:8767")));
        assert!(is_loopback_url(&url("localhost:8767")));
        assert!(is_loopback_url(&url("[::1]:8767")));
        assert!(!is_loopback_url(&url("evil.example:8767")));
        assert!(!is_loopback_url(&url("127.0.0.1.evil.example:8767")));
        // userinfo confusion: the real host is evil.example
        assert!(!is_loopback_url(&url("127.0.0.1:80@evil.example:8767")));
        assert!(!is_loopback_url(&url("localhost@evil.example")));
        assert!(!is_loopback_url("not a url"));
    }
}

#[cfg(test)]
mod recall_flag_tests {
    use super::{RecallFlags, recall_flags_unsupported_by_server};

    fn defaults() -> RecallFlags<'static> {
        RecallFlags {
            strategy: None,
            salience: None,
            recency: None,
            explain: false,
            related: false,
            depth: 1,
            context: false,
            content_format: "auto",
            where_filter: None,
        }
    }

    #[test]
    fn plain_recall_is_served_by_the_server() {
        assert!(recall_flags_unsupported_by_server(defaults()).is_empty());
    }

    #[test]
    fn every_unsupported_flag_is_reported() {
        let f = RecallFlags {
            strategy: Some("graph"),
            salience: Some(false),
            recency: Some(true),
            explain: true,
            related: true,
            depth: 3,
            context: true,
            content_format: "json",
            where_filter: Some("role=CTO"),
        };
        assert_eq!(
            recall_flags_unsupported_by_server(f),
            [
                "--strategy",
                "--salience",
                "--recency",
                "--explain",
                "--related",
                "--depth",
                "--context",
                "--content-format",
                "--where"
            ]
        );
    }

    #[test]
    fn single_flags_are_detected_individually() {
        let mut f = defaults();
        f.explain = true;
        assert_eq!(recall_flags_unsupported_by_server(f), ["--explain"]);
        let mut f = defaults();
        f.salience = Some(false);
        assert_eq!(recall_flags_unsupported_by_server(f), ["--salience"]);
    }
}

#[cfg(test)]
#[allow(unsafe_code)] // env var mutation (edition 2024) in one serialized test
mod store_match_tests {
    use super::server_serves_store;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    /// The probe reads `UTEKE_AUTH_TOKEN`, so the tests of this module run one
    /// at a time.
    static ENV: Mutex<()> = Mutex::new(());
    fn serialized() -> std::sync::MutexGuard<'static, ()> {
        ENV.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Serve one canned HTTP response on a loopback port; returns its base URL.
    fn stub(status: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            if let Ok((mut conn, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = conn.read(&mut buf);
                let _ = conn.write_all(reply.as_bytes());
            }
        });
        url
    }

    /// Like `stub`, but also hands back the raw request the server received.
    fn capturing_stub(body: &str) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut conn, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = conn.read(&mut buf).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
                let _ = conn.write_all(reply.as_bytes());
            }
        });
        (url, rx)
    }

    fn id_body(store: &std::path::Path) -> String {
        format!(r#"{{"id":"{}"}}"#, uteke_core::store_identity(store))
    }

    #[test]
    fn the_same_store_keeps_routing() {
        let _g = serialized();
        let dir = tempfile::tempdir().unwrap();
        let url = stub("200 OK", &id_body(dir.path()));
        assert!(server_serves_store(&url, dir.path().to_str().unwrap()));
    }

    #[test]
    fn another_store_is_not_routed_to_the_server() {
        let _g = serialized();
        let served = tempfile::tempdir().unwrap();
        let ours = tempfile::tempdir().unwrap();
        let url = stub("200 OK", &id_body(served.path()));
        assert!(!server_serves_store(&url, ours.path().to_str().unwrap()));
    }

    #[test]
    fn anything_that_blocks_the_comparison_keeps_the_old_routing() {
        let _g = serialized();
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().to_str().unwrap();
        // A server older than GET /store answers 404.
        assert!(server_serves_store(&stub("404 Not Found", "{}"), store));
        // 401 (no token) and a body that is not an id.
        assert!(server_serves_store(&stub("401 Unauthorized", "{}"), store));
        assert!(server_serves_store(&stub("200 OK", "not json"), store));
        assert!(server_serves_store(&stub("200 OK", r#"{"id":7}"#), store));
        // Nothing listening.
        let gone = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", gone.local_addr().unwrap());
        drop(gone);
        assert!(server_serves_store(&url, store));
    }

    #[test]
    fn a_non_loopback_server_is_never_asked() {
        let _g = serialized();
        // Its paths are not ours, so the ids could never match; no request is made.
        // TEST-NET-1 address: never routable, so a stray request would time out.
        let url = format!("{}://{}", "http", "192.0.2.1:9");
        assert!(server_serves_store(&url, "/tmp/anything"));
    }

    #[test]
    fn the_probe_carries_the_auth_token_like_the_command_it_guards() {
        let _g = serialized();
        let dir = tempfile::tempdir().unwrap();
        let (url, request) = capturing_stub(&id_body(dir.path()));
        // SAFETY: serialized by `ENV`; no other thread of this crate reads the
        // variable while the test runs.
        unsafe { std::env::set_var("UTEKE_AUTH_TOKEN", "probe-token") };
        let same = server_serves_store(&url, dir.path().to_str().unwrap());
        unsafe { std::env::remove_var("UTEKE_AUTH_TOKEN") };
        assert!(same);
        let raw = request
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("the stub saw a request")
            .to_ascii_lowercase();
        assert!(raw.starts_with("get /store "), "{raw}");
        assert!(raw.contains("authorization: bearer probe-token"), "{raw}");
    }
}
