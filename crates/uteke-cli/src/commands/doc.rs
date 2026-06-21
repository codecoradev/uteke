//! Document CLI commands (#411).

use crate::cli::{Cli, DocCommands};
use crate::output;
use crate::Config;
use uteke_core::Uteke;

/// Run document subcommands.
pub(crate) fn run(
    cli: &Cli,
    uteke: &mut Uteke,
    command: &DocCommands,
    config: &Config,
) -> Result<(), String> {
    let ns = crate::resolve_namespace(cli, config);
    let ns: Option<&str> = Some(ns.as_str());

    match command {
        DocCommands::Create {
            slug,
            title,
            file,
            content,
            tags,
        } => {
            // Get content from --content, --file, or stdin.
            let doc_content = if let Some(c) = content {
                c.clone()
            } else if let Some(f) = file {
                if f == "-" {
                    // Read from stdin.
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map_err(|e| format!("Failed to read stdin: {e}"))?;
                    buf
                } else {
                    std::fs::read_to_string(f).map_err(|e| format!("Failed to read file: {e}"))?
                }
            } else {
                return Err("Provide --content <text> or --file <path>".into());
            };

            let doc_title = title.clone().unwrap_or_else(|| {
                // Derive title from first heading or slug.
                doc_content
                    .lines()
                    .find(|l| l.starts_with("# "))
                    .map(|l| l.trim_start_matches("# ").to_string())
                    .unwrap_or_else(|| slug.replace('-', " "))
            });

            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let id = uteke
                .doc_upsert(slug, &doc_title, &doc_content, &tag_refs, ns)
                .map_err(|e| format!("Failed to create document: {e}"))?;

            if cli.json {
                println!("{{\"id\": \"{id}\", \"slug\": \"{slug}\", \"status\": \"created\"}}");
            } else {
                println!("\u{2713} Document '{slug}' created (id: {id})");
                println!("  Title: {doc_title}");
                println!("  Size:  {} chars", doc_content.len());
            }
        }

        DocCommands::Get { id_or_slug } => {
            let doc = uteke
                .doc_get(id_or_slug, ns)
                .map_err(|e| format!("Failed to get document: {e}"))?;
            match doc {
                Some(d) => {
                    if cli.json {
                        output::print_json(&d);
                    } else {
                        println!("Title: {}", d.title);
                        println!("Slug:  {}", d.slug);
                        println!("v{} | {} | {}", d.version, d.content_type, d.updated_at);
                        if !d.tags.is_empty() {
                            println!("Tags:  {}", d.tags.join(", "));
                        }
                        println!();
                        println!("{}", d.content);
                    }
                }
                None => {
                    return Err(format!("Document '{id_or_slug}' not found"));
                }
            }
        }

        DocCommands::List { limit } => {
            let docs = uteke
                .doc_list(ns, *limit)
                .map_err(|e| format!("Failed to list documents: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else if docs.is_empty() {
                println!("No documents found.");
            } else {
                println!("Documents");
                println!("─────────────────────────────────────────────────────");
                for d in &docs {
                    println!(
                        "  {:<20} {:<30} v{}  {}",
                        &d.slug[..d.slug.len().min(20)],
                        &d.title[..d.title.len().min(30)],
                        d.version,
                        &d.updated_at[..10]
                    );
                }
                println!();
                println!("{} document(s)", docs.len());
            }
        }

        DocCommands::Delete { id } => {
            let deleted = uteke
                .doc_delete(id)
                .map_err(|e| format!("Failed to delete document: {e}"))?;
            if deleted {
                if cli.json {
                    println!("{{\"deleted\": \"{id}\"}}");
                } else {
                    println!("\u{2713} Document deleted: {id}");
                }
            } else {
                return Err(format!("Document not found: {id}"));
            }
        }

        DocCommands::Export { output: _ } => {
            let docs = uteke
                .doc_list(ns, 1000)
                .map_err(|e| format!("Failed to list documents for export: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else {
                for d in &docs {
                    println!("--- {} ---", d.slug);
                    if let Some(doc) = uteke.doc_get(&d.id, ns).ok().flatten() {
                        println!("{}", doc.content);
                        println!();
                    }
                }
            }
        }
    }

    Ok(())
}
