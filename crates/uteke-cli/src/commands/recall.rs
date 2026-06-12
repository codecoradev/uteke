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
    let results = if related {
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

    if filtered.is_empty() && min_score > 0.0 {
        if cli.json {
            let response = serde_json::json!({
                "results": [],
                "total": 0,
                "threshold": min_score,
                "message": "No memories above similarity threshold"
            });
            println!("{}", serde_json::to_string(&response).unwrap());
        } else {
            println!("No matching memories found.");
            println!("(min_score threshold: {:.2})", min_score);
        }
        return Ok(());
    }

    if filtered.is_empty() {
        if cli.json {
            output::print_json(&filtered);
        } else {
            println!("No matching memories found.");
        }
        return Ok(());
    }

    if cli.json {
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
