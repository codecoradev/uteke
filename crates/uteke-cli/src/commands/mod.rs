//! Command handler implementations for all CLI subcommands.

mod aging;
pub(crate) mod bench;
mod doc;
mod dream;
mod edges;
mod forget;
pub(crate) mod graph;
mod list;
mod maintenance;
mod namespace;
mod orphans;
mod recall;
mod remember;
mod room;
mod server;
mod tags;
mod timeline;

use crate::cli::Cli;
use crate::cli::Commands;
use crate::resolve_namespace;
use crate::Config;
use uteke_core::Uteke;

pub(crate) use server::{is_server_running, run_via_server};

/// Dispatch all CLI subcommands to their handler implementations.
pub(crate) fn run_command(cli: &Cli, uteke: &mut Uteke, config: &Config) -> Result<(), String> {
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
            room,
            author,
            source,
            source_type,
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
            room.as_deref(),
            author.as_deref(),
            source.as_deref(),
            source_type.as_deref(),
        ),

        Commands::Recall {
            query,
            limit,
            tags,
            entity,
            category,
            min,
            strict,
            strategy,
            salience,
            recency,
            related,
            depth,
            context,
            at,
            content_format,
            r#where,
        } => recall::run_recall(
            cli,
            uteke,
            ns,
            query,
            *limit,
            tags,
            entity.as_deref(),
            category.as_deref(),
            *min,
            *strict,
            strategy.as_deref(),
            config,
            *related,
            *depth,
            *context,
            at.as_deref(),
            content_format.as_str(),
            r#where.as_deref(),
            *salience,
            *recency,
        ),

        Commands::Search { query, limit, tags } => {
            recall::run_search(cli, uteke, ns, query, *limit, tags)
        }

        Commands::List {
            tag,
            limit,
            offset,
            entity,
            category,
            at,
        } => list::run_list(
            cli,
            uteke,
            ns,
            tag,
            *limit,
            *offset,
            entity.as_deref(),
            category.as_deref(),
            at.as_deref(),
        ),

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

        Commands::Import {
            input,
            tags,
            format,
            extract,
            extract_model,
            extract_api_key,
            extract_base_url,
            extract_max_facts,
            batch_dir,
            as_doc,
            as_memory,
            dry_run,
            max_size,
            recursive,
        } => {
            let opts = maintenance::ExtractOpts {
                enabled: *extract,
                model: extract_model.clone(),
                api_key: extract_api_key.clone(),
                base_url: extract_base_url.clone(),
                max_facts: *extract_max_facts,
                cfg: &config.extraction,
            };

            // Batch mode: import entire directory
            if let Some(dir) = batch_dir {
                let force_strategy = if *as_doc {
                    Some(maintenance::ImportStrategy::Document)
                } else if *as_memory {
                    Some(maintenance::ImportStrategy::MemoryExtract)
                } else {
                    None
                };
                return maintenance::run_import_batch(
                    cli,
                    uteke,
                    dir,
                    ns,
                    tags,
                    opts,
                    force_strategy,
                    *recursive,
                    *dry_run,
                    *max_size,
                );
            }

            // Single file mode (original)
            maintenance::run_import(cli, uteke, ns, input, tags, format, opts)
        }

        Commands::Completions { .. } => {
            // Already handled in main()
            Ok(())
        }

        Commands::Init {
            agent,
            memory_provider,
        } => crate::init::run_init(agent, *memory_provider, cli.json),

        Commands::Namespace { command } => namespace::run(cli, uteke, command),

        Commands::Aging { command } => aging::run(cli, uteke, ns, command),

        Commands::Hook { shell } => {
            use crate::cli::SupportedShell;
            let script = match shell {
                SupportedShell::Bash => include_str!("../../assets/shell/uteke-hook.bash"),
                SupportedShell::Zsh => include_str!("../../assets/shell/uteke-hook.zsh"),
                SupportedShell::Fish => include_str!("../../assets/shell/uteke-hook.fish"),
            };
            print!("{script}");
            Ok(())
        }

        Commands::VerifyChecksums {
            checksums_file,
            binary,
        } => maintenance::run_verify_checksums(cli, checksums_file, binary),

        Commands::Room { command } => crate::commands::room::run(cli, uteke, ns, command, config),

        Commands::Pin { id } => {
            let pinned = uteke.pin(id).map_err(|e| format!("Failed to pin: {e}"))?;
            if pinned {
                if cli.json {
                    println!("{{\"pinned\": \"{id}\"}}");
                } else {
                    println!("Pinned memory {}.", &id[..8.min(id.len())]);
                }
            } else {
                return Err(format!("Memory not found: {id}"));
            }
            Ok(())
        }

        Commands::Unpin { id } => {
            let unpinned = uteke
                .unpin(id)
                .map_err(|e| format!("Failed to unpin: {e}"))?;
            if unpinned {
                if cli.json {
                    println!("{{\"unpinned\": \"{id}\"}}");
                } else {
                    println!("Unpinned memory {}.", &id[..8.min(id.len())]);
                }
            } else {
                return Err(format!("Memory not found: {id}"));
            }
            Ok(())
        }

        Commands::Importance => {
            let updated = uteke
                .recompute_importance()
                .map_err(|e| format!("Failed to recompute: {e}"))?;
            if cli.json {
                println!("{{\"updated\": {updated}}}");
            } else {
                println!("Recalculated importance for {updated} memories.");
            }
            Ok(())
        }

        // Bench is handled early in main.rs (creates own temp stores, never
        // reaches this dispatch path).
        Commands::Bench { .. } => Ok(()),

        Commands::Graph { command } => crate::commands::graph::run(cli, uteke, command),

        Commands::Edges {
            id,
            deep,
            direction,
        } => edges::run(cli, uteke, id, *deep, direction),

        Commands::RebuildBacklinks { quiet } => edges::run_rebuild_backlinks(cli, uteke, *quiet),

        Commands::Dream {
            phases,
            skip,
            dry_run,
            quiet,
        } => dream::run(cli, uteke, ns, phases, skip, *dry_run, *quiet),

        Commands::Orphans { threshold, limit } => orphans::run(cli, uteke, ns, *threshold, *limit),

        Commands::Timeline { id, limit } => timeline::run(cli, uteke, id, *limit),

        Commands::Doc { command } => crate::commands::doc::run(cli, uteke, command, config),
    }
}
