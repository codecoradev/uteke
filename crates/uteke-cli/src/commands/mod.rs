//! Command handler implementations for all CLI subcommands.

mod aging;
mod forget;
mod list;
mod maintenance;
mod namespace;
mod recall;
mod remember;
mod server;
mod tags;

use crate::resolve_namespace;
use crate::Cli;
use crate::Commands;
use crate::Config;
use uteke_core::Uteke;

pub(crate) use server::{is_server_running, run_via_server};

/// Dispatch all CLI subcommands to their handler implementations.
pub(crate) fn run_command(cli: &Cli, uteke: &Uteke, config: &Config) -> Result<(), String> {
    // Resolve effective namespace once: CLI > env > config > "default"
    let resolved_ns = resolve_namespace(cli, config);
    let ns: Option<&str> = Some(resolved_ns.as_str());

    match &cli.command {
        Commands::Remember {
            content,
            tags,
            r#type,
            detect_contradiction,
            entity,
            category,
            meta,
        } => remember::run(
            cli,
            uteke,
            ns,
            content,
            tags,
            r#type,
            *detect_contradiction,
            entity.as_deref(),
            category.as_deref(),
            meta,
        ),

        Commands::Recall {
            query,
            limit,
            tags,
            entity,
            category,
        } => recall::run_recall(cli, uteke, ns, query, *limit, tags, entity.as_deref(), category.as_deref()),

        Commands::Search { query, limit, tags } => {
            recall::run_search(cli, uteke, ns, query, *limit, tags)
        }

        Commands::List {
            tag,
            limit,
            offset,
            entity,
            category,
        } => list::run_list(cli, uteke, ns, tag, *limit, *offset, entity.as_deref(), category.as_deref()),

        Commands::Get { id } => list::run_get(cli, uteke, id),

        Commands::Forget {
            id,
            tag,
            cold,
            all,
            confirm,
        } => forget::run(
            cli,
            uteke,
            ns,
            id,
            tag,
            &forget::Flags {
                cold: *cold,
                all: *all,
                confirm: *confirm,
            },
        ),

        Commands::Stats => list::run_stats(cli, uteke, ns),

        Commands::Doctor => maintenance::run_doctor(cli, uteke),

        Commands::Verify => maintenance::run_verify(cli, uteke),

        Commands::Repair => maintenance::run_repair(cli, uteke),

        Commands::Tags { command } => tags::run(cli, uteke, ns, command),

        Commands::Prune { ttl, dry_run } => maintenance::run_prune(cli, uteke, ns, *ttl, *dry_run),

        Commands::Consolidate { threshold, dry_run } => {
            maintenance::run_consolidate(cli, uteke, ns, *threshold, *dry_run)
        }

        Commands::Export { output } => maintenance::run_export(cli, uteke, ns, output),

        Commands::Import { input } => maintenance::run_import(cli, uteke, ns, input),

        Commands::Completions { .. } => {
            // Already handled in main()
            Ok(())
        }

        Commands::Init { agent } => crate::init::run_init(agent, cli.json),

        Commands::Namespace { command } => namespace::run(cli, uteke, command),

        Commands::Aging { command } => aging::run(cli, uteke, ns, command),

        Commands::Hook { shell } => {
            use crate::SupportedShell;
            let script = match shell {
                SupportedShell::Bash => include_str!("../../../../scripts/shell/uteke-hook.bash"),
                SupportedShell::Zsh => include_str!("../../../../scripts/shell/uteke-hook.zsh"),
                SupportedShell::Fish => include_str!("../../../../scripts/shell/uteke-hook.fish"),
            };
            print!("{script}");
            Ok(())
        }

        Commands::VerifyChecksums {
            checksums_file,
            binary,
        } => maintenance::run_verify_checksums(cli, checksums_file, binary),
    }
}
