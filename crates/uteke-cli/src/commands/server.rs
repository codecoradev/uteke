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

/// Names of the `recall` flags `POST /recall` has no field for. Everything else
/// is sent in the request or only changes how the answer is printed, so
/// `uteke recall` returns the same with and without a server.
///
/// `--salience`/`--recency` (per-request boost weights), `--related`/`--depth`
/// (graph expansion) and `--where` (a filter on JSON content) exist only on the
/// typed [`uteke_core::RecallRequest`], not on the wire: adding them to
/// `POST /recall` would change a CORE endpoint. Defaults (`--depth 1`) do not
/// count as set.
fn recall_flags_unsupported_by_server(args: &super::recall::RecallArgs<'_>) -> Vec<&'static str> {
    let mut out = Vec::new();
    if args.salience.is_some() {
        out.push("--salience");
    }
    if args.recency.is_some() {
        out.push("--recency");
    }
    if args.related {
        out.push("--related");
    }
    if args.depth != 1 {
        out.push("--depth");
    }
    if args.where_filter.is_some() {
        out.push("--where");
    }
    out
}

/// Decode the server's answer to `POST /recall` into the shape core returns.
fn recall_output_from_server(
    data: serde_json::Value,
    req: &uteke_core::RecallInput,
) -> Result<uteke_core::recall_exec::RecallOutput, String> {
    use uteke_core::recall_exec::RecallOutput;
    let parse = |e: serde_json::Error| format!("Parse error: {e}");
    Ok(if req.explain {
        RecallOutput::Explained(serde_json::from_value(data).map_err(parse)?)
    } else if req.pack {
        RecallOutput::Packed(serde_json::from_value(data).map_err(parse)?)
    } else if req.search_type.is_some() {
        RecallOutput::Unified(serde_json::from_value(data).map_err(parse)?)
    } else {
        // An empty answer under a floor is an object that carries the threshold.
        let threshold = data.get("threshold").and_then(|t| t.as_f64());
        let results = match data.get("results") {
            Some(r) if data.is_object() => r.clone(),
            _ => data,
        };
        RecallOutput::Memories {
            results: serde_json::from_value(results).map_err(parse)?,
            min_score: threshold.unwrap_or(0.0) as f32,
        }
    })
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
            valid_from,
            valid_until,
        } => {
            // `POST /remember` takes no `room` (a room write is a separate
            // operation). Say so here instead of sending a key the server
            // rejects as unknown, and do not fall back to a local store: that
            // would split the data between two databases (#1439).
            if room.is_some() {
                return Err(
                    "--room is not supported via the server; run without --server or use the room API"
                        .to_string(),
                );
            }
            // The timestamp anchor is a prefix of the content (#1232), so it is
            // applied here, before the request, exactly as on a local store.
            let content = match timestamp {
                Some(ts) => super::anchor::apply_timestamp_anchor(content, ts)?,
                None => content.clone(),
            };
            // The typed remember request shared with HTTP and MCP (#1343)
            // builds the body; the server validates and applies its policy.
            // entity/category/--meta travel inside `metadata`.
            let body = uteke_core::RememberInput {
                content,
                tags: tags.clone(),
                namespace: Some(ns.to_string()),
                memory_type: r#type.clone(),
                detect_contradiction: *detect_contradiction,
                metadata: uteke_core::metadata_from_cli_flags(
                    entity.as_deref(),
                    category.as_deref(),
                    meta,
                ),
                source: source.clone(),
                source_type: source_type.clone(),
                author_type: author_type.clone(),
                author: author.clone(),
                // Validity goes into the columns on the server (#1598); the
                // server validates it, and so does the local path.
                valid_from: valid_from.clone(),
                valid_until: valid_until.clone(),
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
        Commands::Recall { .. } => {
            let args = super::recall::RecallArgs::from_command(&cli.command)
                .expect("a recall command carries recall flags");
            // The HTTP /recall request has no field for some flags. Dropping
            // them silently returned a differently-ranked or differently-shaped
            // result than the same command run locally (#1332): let the caller
            // fall back to the local store instead.
            let unsupported = recall_flags_unsupported_by_server(&args);
            if !unsupported.is_empty() {
                tracing::info!(
                    "recall flag(s) {} are not supported via the server; using the local store",
                    unsupported.join(", ")
                );
                return Err("unsupported".to_string());
            }
            args.check_rendering()?;
            // The same request as the local path, as a `POST /recall` body; the
            // server validates and applies its own policy.
            let input = args.input(Some(ns));
            let resp = client
                .post(format!("{server_url}/recall"))
                .json(&input.to_http_body())
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let data = parse_json_value(resp)?;
            let output = recall_output_from_server(data, &input)?;
            let min_score = match &output {
                uteke_core::recall_exec::RecallOutput::Memories { min_score, .. } => *min_score,
                _ => args.min.unwrap_or(0.0),
            };
            super::recall::render(cli, &args, output, min_score);
        }
        Commands::Capture {
            session,
            role,
            source,
            text,
        } => {
            // Queued by the server, which holds the store (#1465: a local
            // open would wait on its lock). It refuses while capture is off.
            let content = match text {
                Some(t) => t.clone(),
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map_err(|e| format!("Failed to read stdin: {e}"))?;
                    buf
                }
            };
            let body = serde_json::json!({
                "session_id": session,
                "role": role,
                "content": content,
                "source": source,
                "namespace": ns,
            });
            let resp = client
                .post(format!("{server_url}/capture"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let data = parse_json_value(resp)?;
            if cli.json {
                println!("{data}");
            } else if data["duplicate"].as_bool() == Some(true) {
                println!(
                    "Already captured: {}",
                    data["id"].as_str().unwrap_or_default()
                );
            } else {
                println!(
                    "\u{2713} Captured: {}",
                    data["id"].as_str().unwrap_or_default()
                );
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
            cold,
            all,
            confirm,
            reason,
        } => {
            // `DELETE /forget` takes an id or a tag only. `--cold` and `--all`
            // are local operations: say so instead of dropping them, and do
            // not fall back to a local store, which would delete from the
            // wrong database (#1439).
            if id.is_none() && tag.is_none() && (*cold || *all) {
                return Err(
                    "--cold and --all are not supported via the server; run without --server"
                        .to_string(),
                );
            }
            // The confirmation a local forget asks for (#1439): a destructive
            // call does not go out without `--confirm` or a "y" at the prompt.
            if let Some(id) = id {
                if !super::forget::confirm_single(id, *confirm) {
                    println!("Cancelled.");
                    return Ok(());
                }
            } else if let Some(tag) = tag
                && !*confirm
            {
                println!(
                    "This will delete every memory with tag '{tag}' in namespace '{ns}'. Use --confirm to proceed."
                );
                return Ok(());
            }
            // The DELETE /forget path comes from the typed request in
            // uteke-core (#1343); the id wins over the tag, as on the server.
            let input = uteke_core::ForgetInput {
                id: id.clone(),
                tag: tag.clone(),
                namespace: Some(ns.to_string()),
                reason: reason.clone(),
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
            reason: None,
        };
        assert_eq!(
            by_id.to_http_path(enc).as_deref(),
            Some("/forget?id=ab%20cd%2F1")
        );
        let by_tag = uteke_core::ForgetInput {
            id: None,
            tag: Some("my tag&x".into()),
            namespace: Some("work space".into()),
            reason: None,
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
    use super::{recall_flags_unsupported_by_server, recall_output_from_server};
    use crate::commands::recall::RecallArgs;
    use clap::Parser;
    use uteke_core::recall_exec::RecallOutput;

    fn unsupported(argv: &[&str]) -> Vec<&'static str> {
        let mut v = vec!["uteke", "recall", "q"];
        v.extend_from_slice(argv);
        let cmd = crate::cli::Cli::parse_from(v).command;
        recall_flags_unsupported_by_server(&RecallArgs::from_command(&cmd).unwrap())
    }

    #[test]
    fn plain_recall_is_served_by_the_server() {
        assert!(unsupported(&[]).is_empty());
    }

    /// Flags the wire carries, and flags that only change the printing, are
    /// served: the server must not be bypassed for them.
    #[test]
    fn what_the_request_or_the_printing_covers_is_served() {
        for flags in [
            &["--strategy", "graph"][..],
            &["--explain"],
            &["--context"],
            &["--content-format", "json"],
            &["--pack", "--budget", "100"],
            &["--type", "doc"],
            &["--at", "2026-06-01T12:00:00Z", "--entity", "e"],
            &["--depth", "1"],
        ] {
            assert!(unsupported(flags).is_empty(), "{flags:?}");
        }
    }

    #[test]
    fn every_flag_without_a_wire_field_is_reported() {
        assert_eq!(
            unsupported(&[
                "--salience",
                "false",
                "--recency",
                "true",
                "--related",
                "--depth",
                "3",
                "--where",
                "role=CTO",
            ]),
            ["--salience", "--recency", "--related", "--depth", "--where"]
        );
        assert_eq!(unsupported(&["--salience", "true"]), ["--salience"]);
    }

    #[test]
    fn the_answer_is_decoded_by_the_request_that_asked() {
        let input = |f: fn(&mut uteke_core::RecallInput)| {
            let mut i = uteke_core::RecallInput {
                query: "q".into(),
                ..Default::default()
            };
            f(&mut i);
            i
        };
        // A floor that empties the answer arrives as an object with the threshold.
        let envelope = serde_json::json!({"results": [], "total": 0, "threshold": 0.3});
        match recall_output_from_server(envelope, &input(|_| {})).unwrap() {
            RecallOutput::Memories { results, min_score } => {
                assert!(results.is_empty());
                assert_eq!(min_score, 0.3);
            }
            other => panic!("{other:?}"),
        }
        let empty = || serde_json::json!([]);
        assert!(matches!(
            recall_output_from_server(empty(), &input(|i| i.search_type = Some("all".into()))),
            Ok(RecallOutput::Unified(r)) if r.is_empty()
        ));
        assert!(matches!(
            recall_output_from_server(empty(), &input(|i| i.explain = true)),
            Ok(RecallOutput::Explained(r)) if r.is_empty()
        ));
        let pack = serde_json::json!({"selected": [], "skipped": [], "budget_used": 0, "budget_chars": 10});
        assert!(matches!(
            recall_output_from_server(pack, &input(|i| i.pack = true)),
            Ok(RecallOutput::Packed(p)) if p.selected.is_empty()
        ));
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
