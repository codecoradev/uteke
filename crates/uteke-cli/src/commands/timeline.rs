//! `uteke timeline <id>` — show timeline events for a memory (#347).

use crate::cli::Cli;
use uteke_core::Uteke;

pub(crate) fn run(cli: &Cli, uteke: &Uteke, id: &str, limit: usize) -> Result<(), String> {
    let events = uteke
        .timeline(id, limit)
        .map_err(|e| format!("Failed to list timeline: {e}"))?;

    if cli.json {
        println!("{}", serde_json::to_string(&events).unwrap());
        return Ok(());
    }

    if events.is_empty() {
        println!("No timeline events for memory {}.", &id[..8.min(id.len())]);
        return Ok(());
    }

    println!(
        "Timeline for memory {} ({} event{}):",
        &id[..8.min(id.len())],
        events.len(),
        if events.len() == 1 { "" } else { "s" }
    );
    for e in &events {
        let data = e
            .event_data
            .as_deref()
            .map(|d| format!("  {d}"))
            .unwrap_or_default();
        println!("  {}  [{}]{}", &e.created_at[..19], e.event_type, data);
    }
    Ok(())
}
