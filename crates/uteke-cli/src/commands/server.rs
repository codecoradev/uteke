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
            let mut body = serde_json::json!({
                "content": content,
                "tags": tags,
                "namespace": ns
            });
            if let Some(ts) = timestamp {
                body["timestamp"] = serde_json::json!(ts);
            }
            if let Some(at) = author_type {
                body["author_type"] = serde_json::json!(at);
            }
            if !r#type.is_empty() {
                body["type"] = serde_json::json!(r#type);
            }
            if *detect_contradiction {
                body["detect_contradiction"] = serde_json::json!(true);
            }
            // Build metadata from entity/category/meta flags
            let mut meta_map = serde_json::Map::new();
            if let Some(e) = entity {
                meta_map.insert("entity".to_string(), serde_json::Value::String(e.clone()));
            }
            if let Some(c) = category {
                meta_map.insert("category".to_string(), serde_json::Value::String(c.clone()));
            }
            for pair in meta {
                if let Some((key, value)) = pair.split_once(':') {
                    meta_map.insert(
                        key.to_string(),
                        serde_json::Value::String(value.to_string()),
                    );
                }
            }
            if !meta_map.is_empty() {
                body["metadata"] = serde_json::Value::Object(meta_map);
            }
            if let Some(room_id) = room {
                body["room"] = serde_json::json!(room_id);
            }
            if let Some(author_name) = author {
                body["author"] = serde_json::json!(author_name);
            }
            if let Some(src) = source {
                body["source"] = serde_json::json!(src);
            }
            if let Some(st) = source_type {
                body["source_type"] = serde_json::json!(st);
            }
            let resp = client
                .post(format!("{server_url}/remember"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let data = parse_json_value(resp)?;
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
            tag,
            limit,
            offset,
            at,
            ..
        } => {
            let mut body = serde_json::json!({
                "tag": tag,
                "limit": limit,
                "offset": offset,
                "namespace": ns
            });
            if let Some(a) = at {
                body["at"] = serde_json::json!(a);
            }
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
