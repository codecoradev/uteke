//! `uteke update` — in-place memory edit (CLI surface for `Uteke::update_memory`,
//! issue #1202). HTTP `PUT /memory` and MCP `uteke_update` already existed;
//! this closes the CLI gap.

use crate::Cli;
use uteke_core::Uteke;

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
    // #1598: the validity is checked first and written only after the other
    // fields went through, so neither a refused interval (or a deprecated
    // memory) nor a refused field leaves a partial update behind.
    let validity = uteke_core::memory::types::Validity::parse(valid_from, valid_until)
        .map_err(|e| e.to_string())?;
    if let Some(v) = &validity {
        let found = uteke
            .check_validity(id, v)
            .map_err(|e| format!("Failed to update memory: {e}"))?;
        if !found {
            return Err(format!("Memory not found: {id}"));
        }
    }
    let other_fields = content.is_some()
        || tags.is_some()
        || importance.is_some()
        || pinned.is_some()
        || memory_type.is_some();
    let tag_list: Option<Vec<String>> = tags.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    let updated = validity.is_some() && !other_fields
        || uteke
            .update_memory(
                id,
                content.as_deref(),
                tag_list.as_deref(),
                None,
                importance,
                pinned,
                memory_type.as_deref(),
            )
            .map_err(|e| format!("Failed to update memory: {e}"))?;
    if !updated {
        return Err(format!("Memory not found: {id}"));
    }
    if let Some(v) = &validity {
        let found = uteke
            .set_validity(id, v)
            .map_err(|e| format!("Failed to update memory: {e}"))?;
        if !found {
            return Err(format!("Memory not found: {id}"));
        }
    }

    if cli.json {
        let memory = uteke
            .get(id)
            .map_err(|e| format!("Failed to reload memory: {e}"))?;
        println!("{}", serde_json::to_string_pretty(&memory).unwrap());
    } else {
        println!("✓ Memory '{id}' updated");
    }
    Ok(())
}
