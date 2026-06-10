//! Recall and Search commands — semantic and keyword search.

use crate::output;
use crate::Cli;
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
) -> Result<(), String> {
    tracing::info!("Recalling: {query} (limit: {limit})");
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    let tags_filter = if tag_refs.is_empty() {
        None
    } else {
        Some(tag_refs.as_slice())
    };
    let results = uteke
        .recall(query, limit, tags_filter, ns)
        .map_err(|e| format!("Failed to recall: {e}"))?;

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
