//! Document CLI commands (#411, #438).

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
            parent,
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
            let parent_ref = parent.as_deref();
            let id = uteke
                .doc_upsert_with_parent(slug, &doc_title, &doc_content, &tag_refs, ns, parent_ref)
                .map_err(|e| format!("Failed to create document: {e}"))?;

            if cli.json {
                let parent_json = parent
                    .as_ref()
                    .map(|p| format!(r#", "parent": "{p}""#))
                    .unwrap_or_default();
                println!(r#"{{"id": "{id}", "slug": "{slug}"{parent_json}, "status": "created"}}"#);
            } else {
                println!("✓ Document '{slug}' created (id: {id})");
                println!("  Title: {doc_title}");
                println!("  Size:  {} chars", doc_content.len());
                if let Some(p) = parent {
                    println!("  Parent: {p}");
                }
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
                        println!("Title: {} (depth: {})", d.title, d.depth);
                        println!("Slug:  {}", d.slug);
                        println!("v{} | {} | {}", d.version, d.content_type, d.updated_at);
                        if let Some(ref pid) = d.parent_id {
                            println!("Parent: {pid}");
                        }
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

        DocCommands::List { limit, tree } => {
            let docs = if *tree {
                uteke
                    .doc_list_roots(ns, *limit)
                    .map_err(|e| format!("Failed to list root documents: {e}"))?
            } else {
                uteke
                    .doc_list(ns, *limit)
                    .map_err(|e| format!("Failed to list documents: {e}"))?
            };
            if cli.json {
                output::print_json(&docs);
            } else if docs.is_empty() {
                println!("No documents found.");
            } else {
                if *tree {
                    // Print tree structure.
                    let indent = "  ";
                    let mut stack: Vec<(String, usize)> = docs
                        .iter()
                        .map(|d| (d.id.clone(), 0))
                        .collect();
                    while let Some((current_id, current_depth)) = stack.pop() {
                        let children = uteke
                            .doc_list_children(&current_id, ns, 1000)
                            .unwrap_or_default();
                        let prefix: String = indent.repeat(current_depth);
                        if let Some(parent) = docs.iter().find(|d| d.id == current_id) {
                            let tree_char = if children.is_empty() { "├─" } else { "┬─" };
                            println!(
                                "{prefix}{tree_char} {:<20} {:<30} v{}",
                                &parent.slug[..parent.slug.len().min(20)],
                                &parent.title[..parent.title.len().min(30)],
                                parent.version
                            );
                        }
                        for child in children.into_iter().rev() {
                            stack.push((child.id, current_depth + 1));
                        }
                    }
                } else {
                    println!("Documents");
                    println!(
                        "─────────────────────────────────────────────────────"
                    );
                    for d in &docs {
                        let depth_indicator = if d.depth > 0 {
                            format!("  {}", "›".repeat(d.depth as usize))
                        } else {
                            String::new()
                        };
                        println!(
                            "  {:<20} {:<30} v{}  {}{}",
                            &d.slug[..d.slug.len().min(20)],
                            &d.title[..d.title.len().min(30)],
                            d.version,
                            &d.updated_at[..10],
                            depth_indicator
                        );
                    }
                }
                println!();
                println!("{} document(s)", docs.len());
            }
        }

        DocCommands::Children {
            parent,
            limit,
        } => {
            let docs = uteke
                .doc_list_children(parent, ns, *limit)
                .map_err(|e| format!("Failed to list children: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else if docs.is_empty() {
                println!("No children for '{parent}'.");
            } else {
                println!("Children of '{parent}'");
                println!(
                    "─────────────────────────────────────────────────────"
                );
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
                println!("{} child(ren)", docs.len());
            }
        }

        DocCommands::Move {
            id_or_slug,
            parent,
        } => {
            let new_parent = parent.as_deref();
            let affected = uteke
                .doc_move(id_or_slug, new_parent, ns)
                .map_err(|e| format!("Failed to move document: {e}"))?;
            if cli.json {
                println!(
                    r#"{{"moved": "{id_or_slug}", "parent": "{}", "affected": {affected}}}"#,
                    new_parent.unwrap_or("(root)")
                );
            } else {
                let dest = new_parent.unwrap_or("(root)");
                println!("✓ Moved '{id_or_slug}' → {dest}");
                println!("  {affected} document(s) updated");
            }
        }

        DocCommands::Breadcrumbs { id_or_slug } => {
            let crumbs = uteke
                .doc_breadcrumbs(id_or_slug, ns)
                .map_err(|e| format!("Failed to get breadcrumbs: {e}"))?;
            if cli.json {
                output::print_json(&crumbs);
            } else if crumbs.is_empty() {
                println!("No breadcrumbs found (root document).");
            } else {
                println!("Path to '{id_or_slug}'");
                println!(
                    "─────────────────────────────────────────────────────"
                );
                for (i, d) in crumbs.iter().enumerate() {
                    let connector = if i == crumbs.len() - 1 {
                        "└─"
                    } else {
                        "├─"
                    };
                    println!("  {connector} {} (depth: {})", d.slug, d.depth);
                }
            }
        }

        DocCommands::Descendants {
            id_or_slug,
            max_depth,
            limit,
        } => {
            let max = if *max_depth == 0 {
                None
            } else {
                Some(*max_depth as i64)
            };
            let docs = uteke
                .doc_list_descendants(id_or_slug, ns, max, *limit)
                .map_err(|e| format!("Failed to list descendants: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else if docs.is_empty() {
                println!("No descendants for '{id_or_slug}'.");
            } else {
                println!("Descendants of '{id_or_slug}'");
                println!(
                    "─────────────────────────────────────────────────────"
                );
                for d in &docs {
                    let indent = "  ".repeat(d.depth as usize);
                    println!(
                        "{indent}{:<20} {:<30} d{}",
                        &d.slug[..d.slug.len().min(20)],
                        &d.title[..d.title.len().min(30)],
                        d.depth
                    );
                }
                println!();
                println!("{} descendant(s)", docs.len());
            }
        }

        DocCommands::Delete { id } => {
            let (deleted, subtree_size) = uteke
                .doc_delete(id)
                .map_err(|e| format!("Failed to delete document: {e}"))?;
            if deleted {
                if cli.json {
                    println!(
                        r#"{{"deleted": "{id}", "subtree_size": {subtree_size}}}"#
                    );
                } else {
                    println!("✓ Document deleted: {id}");
                    if subtree_size > 1 {
                        println!(
                            "  {subtree_size} document(s) removed (cascade)"
                        );
                    }
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
