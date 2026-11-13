//! `uteke update` — in-place memory edit (CLI surface for `Uteke::update_memory`,
//! issue #1202). HTTP `PUT /memory` and MCP `uteke_update` already existed;
//! this closes the CLI gap.

use crate::Cli;
use uteke_core::{Error, UpdateInput, UpdateRequest, Uteke};

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    id: &str,
    content: Option<String>,
    tags: Option<String>,
    importance: Option<f64>,
    pinned: Option<bool>,
    memory_type: Option<String>,
    valid_from: Option<&str>,
    valid_until: Option<&str>,
) -> Result<(), String> {
    tracing::info!("Updating memory: {id}");
    // The order (validity checked first, written last, #1598) is core's
    // `run_update`; this command only decodes the flags and renders.
    let tag_list: Option<Vec<String>> = tags.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let req = UpdateRequest::decode(UpdateInput {
        id: id.to_string(),
        content,
        tags: tag_list,
        importance,
        pinned,
        memory_type,
        valid_from: valid_from.map(str::to_string),
        valid_until: valid_until.map(str::to_string),
        ..Default::default()
    })
    .map_err(|e| e.to_string())?;
    let run = uteke.run_update(&req).map_err(|e| match e {
        Error::NotFound(m) => m,
        other => format!("Failed to update memory: {other}"),
    })?;

    if cli.json {
        let memory = uteke
            .get(id)
            .map_err(|e| format!("Failed to reload memory: {e}"))?;
        println!("{}", serde_json::to_string_pretty(&memory).unwrap());
    } else {
        println!("✓ Memory '{id}' updated");
        if let Some(summary) = run.redaction_summary() {
            println!("  \u{26a0} Secrets masked in the stored text: {summary}");
        }
    }
    Ok(())
}
