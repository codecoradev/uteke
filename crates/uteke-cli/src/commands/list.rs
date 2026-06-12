//! List, Get, and Stats commands.

use crate::cli::Cli;
use crate::output;
use uteke_core::Uteke;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_list(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    tag: &Option<String>,
    limit: usize,
    offset: usize,
    entity: Option<&str>,
    category: Option<&str>,
) -> Result<(), String> {
    tracing::info!(
        "Listing memories (tag: {:?}, limit: {limit}, offset: {offset})",
        tag
    );
    let mut results = uteke
        .list(tag.as_deref(), limit, offset, ns)
        .map_err(|e| format!("Failed to list: {e}"))?;

    // Post-filter by entity/category metadata
    if entity.is_some() || category.is_some() {
        results.retain(|m| {
            if let Some(ent) = entity {
                let matches = m
                    .metadata
                    .get("entity")
                    .and_then(|v| v.as_str())
                    .is_some_and(|e| e == ent);
                if !matches {
                    return false;
                }
            }
            if let Some(cat) = category {
                let matches = m
                    .metadata
                    .get("category")
                    .and_then(|v| v.as_str())
                    .is_some_and(|c| c == cat);
                if !matches {
                    return false;
                }
            }
            true
        });
    }

    if cli.json {
        output::print_json(&results);
    } else {
        output::print_list_human(&results);
    }
    Ok(())
}

pub(crate) fn run_get(cli: &Cli, uteke: &Uteke, id: &str) -> Result<(), String> {
    tracing::info!("Getting memory: {id}");
    let memory = uteke
        .get(id)
        .map_err(|e| format!("Failed to get memory: {e}"))?;
    if cli.json {
        output::print_json(&memory);
    } else {
        output::print_get_human(&memory);
    }
    Ok(())
}

pub(crate) fn run_stats(cli: &Cli, uteke: &Uteke, ns: Option<&str>) -> Result<(), String> {
    tracing::info!("Getting stats");
    let stats = uteke
        .stats(ns)
        .map_err(|e| format!("Failed to get stats: {e}"))?;
    if cli.json {
        output::print_json(&stats);
    } else {
        output::print_stats_human(&stats);
    }
    Ok(())
}
