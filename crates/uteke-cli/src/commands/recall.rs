//! Recall and Search commands — semantic and keyword search.

use crate::cli::Cli;
use crate::config::Config;
use crate::output;
use uteke_core::Uteke;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_recall(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    query: &str,
    limit: usize,
    tags: &[String],
    entity: Option<&str>,
    category: Option<&str>,
    min: Option<f32>,
    strict: bool,
    config: &Config,
    related: bool,
    depth: usize,
    context: bool,
    at: Option<&str>,
) -> Result<(), String> {
    // Resolve threshold: --min > --strict (→ config min_score_strict) > config min_score > 0.0
    let min_score = match min {
        Some(m) => m,
        None if strict => config.recall.min_score_strict as f32,
        None => config.recall.min_score as f32,
    };

    tracing::info!("Recalling: {query} (limit: {limit}, min_score: {min_score})");
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    let tags_filter = if tag_refs.is_empty() {
        None
    } else {
        Some(tag_refs.as_slice())
    };

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
            .recall_at_time(query, limit, tags_filter, ns, point_in_time, min_score)
            .map_err(|e| format!("Failed to recall at time: {e}"))?
    } else if related {
        uteke
            .recall_related(query, limit, tags_filter, ns, min_score, depth)
            .map_err(|e| format!("Failed to recall: {e}"))?
    } else {
        uteke
            .recall(query, limit, tags_filter, ns, min_score)
            .map_err(|e| format!("Failed to recall: {e}"))?
    };

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
        output::print_json(&filtered);
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
