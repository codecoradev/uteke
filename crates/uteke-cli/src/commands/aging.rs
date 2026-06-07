//! Aging subcommands — status, preview, cleanup.

use crate::output;
use crate::AgingCommands;
use crate::Cli;
use uteke_core::Uteke;

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    command: &AgingCommands,
) -> Result<(), String> {
    match command {
        AgingCommands::Status => {
            tracing::info!("Aging status");
            let status = uteke
                .aging_status(ns)
                .map_err(|e| format!("Failed to get aging status: {e}"))?;
            if cli.json {
                output::print_json(&status);
            } else {
                output::print_aging_status_human(&status);
            }
            Ok(())
        }
        AgingCommands::Preview {
            older_than_days,
            max_access_count,
        } => {
            tracing::info!(
                "Aging preview (older_than: {}d, max_access: {})",
                older_than_days,
                max_access_count
            );
            let memories = uteke
                .aging_preview(*older_than_days, *max_access_count, ns)
                .map_err(|e| format!("Failed to preview aged memories: {e}"))?;
            if cli.json {
                output::print_json(&memories);
            } else {
                output::print_aging_preview_human(&memories);
            }
            Ok(())
        }
        AgingCommands::Cleanup {
            older_than_days,
            max_access_count,
            yes,
        } => {
            // Preview first
            let preview = uteke
                .aging_preview(*older_than_days, *max_access_count, ns)
                .map_err(|e| format!("Failed to preview aged memories: {e}"))?;

            if preview.is_empty() {
                if cli.json {
                    output::print_json(&uteke_core::CleanupResult { deleted: 0 });
                } else {
                    println!("No aged memories to clean up.");
                }
                return Ok(());
            }

            if !yes {
                if !cli.json {
                    output::print_aging_preview_human(&preview);
                    println!();
                    println!(
                        "\u{26a0} About to delete {} memory(ies). Use --yes to confirm.",
                        preview.len()
                    );
                }
                return Err("Cleanup not confirmed. Use --yes flag to proceed.".to_string());
            }

            let _count = preview.len();
            let result = uteke
                .aging_cleanup(*older_than_days, *max_access_count, ns)
                .map_err(|e| format!("Failed to cleanup aged memories: {e}"))?;
            if cli.json {
                output::print_json(&result);
            } else {
                println!("\u{2713} Cleaned up {} aged memory(ies)", result.deleted);
            }
            Ok(())
        }
    }
}
