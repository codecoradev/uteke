//! List, Get, and Stats commands.

use crate::output;
use crate::Cli;
use uteke_core::Uteke;

pub(crate) fn run_list(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    tag: &Option<String>,
    limit: usize,
    offset: usize,
) -> Result<(), String> {
    tracing::info!(
        "Listing memories (tag: {:?}, limit: {limit}, offset: {offset})",
        tag
    );
    let results = uteke
        .list(tag.as_deref(), limit, offset, ns)
        .map_err(|e| format!("Failed to list: {e}"))?;
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
