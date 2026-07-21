#![allow(clippy::too_many_arguments)]
//! Recall and Search commands — semantic and keyword search.

use crate::cli::Cli;
use crate::config::Config;
use crate::output;
use uteke_core::{RecallStrategy, SearchType, Uteke};

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_recall(
    cli: &Cli,
    uteke: &mut Uteke,
    ns: Option<&str>,
    query: &str,
    limit: usize,
    tags: &[String],
    entity: Option<&str>,
    category: Option<&str>,
    min: Option<f32>,
    strict: bool,
    strategy: Option<&str>,
    config: &Config,
    related: bool,
    depth: usize,
    context: bool,
    at: Option<&str>,
    content_format: &str,
    where_filter: Option<&str>,
    salience: Option<bool>,
    recency: Option<bool>,
    search_type: Option<&str>,
    enrich: bool,
) -> Result<(), String> {
    // Resolve search type: --type flag > default (All = unified)
    let resolved_search_type = match search_type {
        Some("memory") => SearchType::Memory,
        Some("doc") => SearchType::Document,
        Some("all") | None => SearchType::All,
        Some(other) => {
            return Err(format!(
                "Invalid --type: '{other}'. Use 'all', 'memory', or 'doc'."
            ));
        }
    };

    // When --type is explicitly set (not default unified), route to recall_unified.
    // When unified (default), use existing recall path for backward compat with
    // --strategy, --at, --related, --entity, --category, --where flags
    // which only apply to memory recall.
    // --salience/--recency/--no-salience/--no-recency work on both paths (#721).
    let use_unified = match resolved_search_type {
        SearchType::All => {
            // Use unified only when no memory-only flags are active.
            // Memory-only flags: --at, --related, --entity, --category, --where
            at.is_none()
                && !related
                && entity.is_none()
                && category.is_none()
                && where_filter.is_none()
        }
        SearchType::Memory | SearchType::Document => true,
    };

    // Resolve threshold: --min > --strict (→ config min_score_strict) > config min_score > 0.0
    let min_score = match min {
        Some(m) => m,
        None if strict => config.recall.min_score_strict as f32,
        None => config.recall.min_score as f32,
    };

    tracing::info!(
        "Recalling: {query} (limit: {limit}, min_score: {min_score}, type: {:?})",
        resolved_search_type
    );
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    let tags_filter = if tag_refs.is_empty() {
        None
    } else {
        Some(tag_refs.as_slice())
    };

    // Resolve strategy: --strategy flag > config [recall].default_strategy
    // > built-in default ("vector"). Unknown values fall back to vector with
    // a warning so a typo never silently changes recall semantics.
    let strategy_name = strategy.unwrap_or(&config.recall.default_strategy);
    let resolved_strategy = match RecallStrategy::from_str_opt(strategy_name) {
        Some(s) => s,
        None => {
            tracing::warn!("Unknown recall strategy '{strategy_name}', falling back to vector");
            RecallStrategy::Vector
        }
    };

    // #352/#721: dual-axis salience/recency boost.
    // Tri-state via Option<bool>: None (absent) = use default (0.1),
    // Some(true) = use config weight (0.15), Some(false) / --no-* = 0.0.
    uteke.set_salience_recency_config(uteke_core::SalienceRecencyConfig {
        salience_weight: match salience {
            Some(true) => config.recall.salience_weight, // explicit --salience
            Some(false) => 0.0,                          // explicit --no-salience
            None => uteke_core::SalienceRecencyConfig::default().salience_weight, // default 0.1
        },
        recency_weight: match recency {
            Some(true) => config.recall.recency_weight, // explicit --recency
            Some(false) => 0.0,                         // explicit --no-recency
            None => uteke_core::SalienceRecencyConfig::default().recency_weight, // default 0.1
        },
    });

    if use_unified {
        // Unified search path (#531)
        let unified_results = uteke
            .recall_unified(
                query,
                limit,
                tags_filter,
                ns,
                min_score,
                resolved_search_type,
                None,
                None,
                enrich,
            )
            .map_err(|e| format!("Failed to recall: {e}"))?;

        uteke.reset_salience_recency_config();

        if unified_results.is_empty() {
            if cli.json {
                output::print_json(&unified_results);
            } else if min_score > 0.0 {
                println!("No matching results found.");
                println!("(min_score threshold: {:.2})", min_score);
            } else {
                println!("No matching results found.");
            }
            return Ok(());
        }

        if cli.json {
            output::print_json(&unified_results);
        } else {
            output::print_unified_human(&unified_results);
        }
        return Ok(());
    }

    // Existing memory-only recall path (backward compatible)
    // Wrap recall in a closure so reset always runs, even on error paths
    // (CodeCora #387: boost config must not leak on early return).
    let recall_result: Result<_, String> = (|| {
        // Time-travel mode: parse --at as RFC3339 and use recall_at_time
        let results = if let Some(at_str) = at {
            let point_in_time = chrono::DateTime::parse_from_rfc3339(at_str)
                .map_err(|e| {
                    format!(
                        "Invalid --at timestamp: {e}. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
                    )
                })?
                .with_timezone(&chrono::Utc);
            if related {
                return Err("--at and --related cannot be used together".into());
            }
            uteke
                .recall_at_time(
                    query,
                    limit,
                    tags_filter,
                    ns,
                    point_in_time,
                    min_score,
                    None,
                    None,
                )
                .map_err(|e| format!("Failed to recall at time: {e}"))?
        } else if related {
            uteke
                .recall_related(query, limit, tags_filter, ns, min_score, depth)
                .map_err(|e| format!("Failed to recall: {e}"))?
        } else {
            uteke
                .recall_hybrid(query, limit, tags_filter, ns, resolved_strategy, min_score)
                .map_err(|e| format!("Failed to recall: {e}"))?
        };
        Ok(results)
    })();

    // Reset per-query boost config so later recalls on the same Uteke instance
    // aren't affected — runs on EVERY path (CodeCora #387).
    uteke.reset_salience_recency_config();

    let results = recall_result?;

    // Post-filter by entity/category metadata
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|sr| {
            if let Some(ent) = entity {
                let matches = sr
                    .memory
                    .metadata
                    .get("entity")
                    .and_then(|v| v.as_str())
                    .is_some_and(|e| e == ent);
                if !matches {
                    return false;
                }
            }
            if let Some(cat) = category {
                let matches = sr
                    .memory
                    .metadata
                    .get("category")
                    .and_then(|v| v.as_str())
                    .is_some_and(|c| c == cat);
                if !matches {
                    return false;
                }
            }
            true
        })
        .collect();

    // JSON field-level filtering: --where key=value
    let filtered: Vec<_> = if let Some(where_clause) = where_filter {
        let (key, value) = parse_where(where_clause)?;
        filtered
            .into_iter()
            .filter(|sr| {
                if sr.memory.content_type != "json" {
                    return false;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&sr.memory.content) {
                    if let Some(obj) = v.as_object() {
                        let field = match obj.get(&key) {
                            Some(f) => f,
                            None => return false,
                        };
                        // Match string, number, or bool values
                        let actual = match field {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Null => "null".to_string(),
                            _ => return false, // arrays/objects not supported in key=value filter
                        };
                        return actual == value;
                    }
                }
                false
            })
            .collect()
    } else {
        filtered
    };

    if filtered.is_empty() {
        if cli.json {
            // Always output a JSON array for machine consumers (cora-cli,
            // scripts, MCP). A bare [] is easier to parse than {"results":[]}.
            output::print_json(&filtered);
        } else if min_score > 0.0 {
            println!("No matching memories found.");
            println!("(min_score threshold: {:.2})", min_score);
        } else if context {
            println!("[No relevant memories found for: {query}]");
        } else {
            println!("No matching memories found.");
        }
        return Ok(());
    }

    if context {
        // Context mode: formatted for AI prompt injection
        let avg_score: f32 = filtered.iter().map(|r| r.score).sum::<f32>() / filtered.len() as f32;
        println!(
            "[Relevant Memories ({} results, {:.2} avg score)]",
            filtered.len(),
            avg_score
        );
        for (i, sr) in filtered.iter().enumerate() {
            let tags = if sr.memory.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", sr.memory.tags.join(", "))
            };
            let importance = if sr.memory.pinned {
                " \u{2605}".to_string() // ★
            } else if sr.memory.importance > 0.7 {
                " \u{2191}".to_string() // ↑
            } else {
                String::new()
            };
            println!(
                "{}. [{:.2}] {}{}{}",
                i + 1,
                sr.score,
                sr.memory.content,
                tags,
                importance
            );
        }
    } else if cli.json {
        // Apply content_format: pretty-print JSON content when requested
        if content_format == "json" {
            let mut formatted: Vec<_> = filtered.into_iter().collect();
            for sr in &mut formatted {
                if sr.memory.content_type == "json" {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&sr.memory.content) {
                        sr.memory.content = serde_json::to_string_pretty(&v)
                            .unwrap_or_else(|_| sr.memory.content.clone());
                    }
                }
            }
            output::print_json(&formatted);
        } else {
            output::print_json(&filtered);
        }
    } else {
        output::print_recall_human(&filtered);
    }
    Ok(())
}

pub(crate) fn run_search(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    query: &str,
    limit: usize,
    tags: &[String],
) -> Result<(), String> {
    tracing::info!("Searching: {query} (limit: {limit})");
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    let tags_filter = if tag_refs.is_empty() {
        None
    } else {
        Some(tag_refs.as_slice())
    };
    let results = uteke
        .search(query, limit, tags_filter, ns)
        .map_err(|e| format!("Failed to search: {e}"))?;
    if cli.json {
        output::print_json(&results);
    } else {
        output::print_search_human(&results);
    }
    Ok(())
}

/// Parse a `--where` expression in `key=value` format.
fn parse_where(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid --where expression: '{s}'. Use key=value format."
        ));
    }
    Ok((parts[0].trim().to_string(), parts[1].trim().to_string()))
}
