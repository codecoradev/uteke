//! HTTP server routing for CLI commands.

use crate::output;
use crate::Cli;
use crate::Commands;

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

/// Route CLI commands through the HTTP server for <50ms latency.
pub(crate) fn run_via_server(cli: &Cli, server_url: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::new();
    let ns = cli.namespace.as_deref().unwrap_or("default");

    match &cli.command {
        Commands::Remember {
            content,
            tags,
            r#type,
            detect_contradiction,
            entity,
            category,
            meta,
        } => {
            let mut body = serde_json::json!({
                "content": content,
                "tags": tags,
                "namespace": ns
            });
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
            ..
        } => {
            let mut body = serde_json::json!({
                "query": query,
                "limit": limit,
                "tags": tags,
                "namespace": ns
            });
            if let Some(e) = entity {
                body["entity"] = serde_json::json!(e);
            }
            if let Some(c) = category {
                body["category"] = serde_json::json!(c);
            }
            if let Some(m) = min {
                body["min_score"] = serde_json::json!(m);
            }
            if *strict {
                body["strict"] = serde_json::json!(true);
            }
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
            tag, limit, offset, ..
        } => {
            let body = serde_json::json!({
                "tag": tag,
                "limit": limit,
                "offset": offset,
                "namespace": ns
            });
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
            if let Some(id) = id {
                let resp = client
                    .delete(format!(
                        "{server_url}/forget?id={}",
                        urlencoding::encode(id)
                    ))
                    .send()
                    .map_err(|e| format!("Server error: {e}"))?;
                let data = parse_json_value(resp)?;
                if cli.json {
                    println!("{data}");
                } else {
                    println!("\u{2713} Memory forgotten: {id}");
                }
            } else if let Some(tag) = tag {
                let resp = client
                    .delete(format!(
                        "{server_url}/forget?tag={}&namespace={}",
                        urlencoding::encode(tag),
                        urlencoding::encode(ns)
                    ))
                    .send()
                    .map_err(|e| format!("Server error: {e}"))?;
                let data = parse_json_value(resp)?;
                if cli.json {
                    println!("{data}");
                } else {
                    println!(
                        "\u{2713} Deleted {} memories with tag '{}'",
                        data["deleted"], tag
                    );
                }
            } else {
                return Err("Provide an ID, --tag, --cold, or --all".into());
            }
        }
        // Commands not supported via server fall through to local
        _ => {
            return Err("unsupported".into());
        }
    }
    Ok(())
}
