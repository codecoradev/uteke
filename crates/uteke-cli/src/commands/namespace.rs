//! Namespace subcommands — list, stats, switch.

use crate::cli::Cli;
use crate::cli::NamespaceCommands;
use crate::output;
use crate::Config;
use uteke_core::Uteke;

pub(crate) fn run(cli: &Cli, uteke: &Uteke, command: &NamespaceCommands) -> Result<(), String> {
    match command {
        NamespaceCommands::List => {
            tracing::info!("Listing namespaces");
            let namespaces = uteke
                .list_namespaces()
                .map_err(|e| format!("Failed to list namespaces: {e}"))?;
            if cli.json {
                output::print_json(&namespaces);
            } else if namespaces.is_empty() {
                println!("No namespaces found.");
            } else {
                println!("Namespaces ({} total):\n", namespaces.len());
                for ns_name in &namespaces {
                    let count = uteke.count(Some(ns_name.as_str())).unwrap_or(0);
                    println!("  {} ({} memories)", ns_name, count);
                }
            }
            Ok(())
        }
        NamespaceCommands::Stats { name } => {
            tracing::info!("Namespace stats: {name}");
            let stats = uteke
                .stats(Some(name.as_str()))
                .map_err(|e| format!("Failed to get namespace stats: {e}"))?;
            if cli.json {
                output::print_json(&stats);
            } else {
                println!("Namespace: {name}");
                output::print_stats_human(&stats);
            }
            Ok(())
        }
        NamespaceCommands::Switch { name } => {
            tracing::info!("Switching default namespace to: {name}");
            Config::set_default_namespace(name)
                .map_err(|e| format!("Failed to switch namespace: {e}"))?;
            if cli.json {
                output::print_json(&serde_json::json!({"default_namespace": name}));
            } else {
                println!("\u{2713} Default namespace set to '{name}'");
            }
            Ok(())
        }
    }
}
