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
            let resp = client
                .post(format!("{server_url}/remember"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let data: serde_json::Value = resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                println!("{data}");
            } else {
                println!("\u{2713} Memory stored\n  ID: {}", data["id"]);
            }
        }
        Commands::Recall {
            query, limit, tags, ..
        } => {
            let body = serde_json::json!({
                "query": query,
                "limit": limit,
                "tags": tags,
                "namespace": ns
            });
            let resp = client
                .post(format!("{server_url}/recall"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let results: Vec<uteke_core::SearchResult> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
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
            let results: Vec<uteke_core::SearchResult> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
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
            let memories: Vec<uteke_core::Memory> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
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
            let stats: uteke_core::StoreStats =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
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
                let data: serde_json::Value =
                    resp.json().map_err(|e| format!("Parse error: {e}"))?;
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
                let data: serde_json::Value =
                    resp.json().map_err(|e| format!("Parse error: {e}"))?;
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
