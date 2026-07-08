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
                .doc_upsert_with_parent(slug, &doc_title, &doc_content, &tag_refs, None, parent_ref)
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
                .doc_get(id_or_slug)
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
                    .doc_list_roots(*limit)
                    .map_err(|e| format!("Failed to list root documents: {e}"))?
            } else {
                uteke
                    .doc_list(*limit)
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
                    let mut stack: Vec<(String, usize)> =
                        docs.iter().map(|d| (d.id.clone(), 0)).collect();
                    while let Some((current_id, current_depth)) = stack.pop() {
                        let children = uteke
                            .doc_list_children(&current_id, 1000)
                            .unwrap_or_default();
                        let prefix: String = indent.repeat(current_depth);
                        if let Some(parent) = docs.iter().find(|d| d.id == current_id) {
                            let tree_char = if children.is_empty() {
                                "├─"
                            } else {
                                "┬─"
                            };
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
                    println!("─────────────────────────────────────────────────────");
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

        DocCommands::Children { parent, limit } => {
            let docs = uteke
                .doc_list_children(parent, *limit)
                .map_err(|e| format!("Failed to list children: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else if docs.is_empty() {
                println!("No children for '{parent}'.");
            } else {
                println!("Children of '{parent}'");
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
                println!("{} child(ren)", docs.len());
            }
        }

        DocCommands::Move { id_or_slug, parent } => {
            let new_parent = parent.as_deref();
            let affected = uteke
                .doc_move(id_or_slug, new_parent)
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
                .doc_breadcrumbs(id_or_slug)
                .map_err(|e| format!("Failed to get breadcrumbs: {e}"))?;
            if cli.json {
                output::print_json(&crumbs);
            } else if crumbs.is_empty() {
                println!("No breadcrumbs found (root document).");
            } else {
                println!("Path to '{id_or_slug}'");
                println!("─────────────────────────────────────────────────────");
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
                .doc_list_descendants(id_or_slug, max, *limit)
                .map_err(|e| format!("Failed to list descendants: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else if docs.is_empty() {
                println!("No descendants for '{id_or_slug}'.");
            } else {
                println!("Descendants of '{id_or_slug}'");
                println!("─────────────────────────────────────────────────────");
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

        DocCommands::Search { query, limit, mode } => {
            let results = uteke
                .doc_search(query, *limit, mode)
                .map_err(|e| format!("Failed to search documents: {e}"))?;
            if cli.json {
                output::print_json(&results);
            } else if results.is_empty() {
                println!("No documents found for '{query}'.");
            } else {
                println!("Search results for '{query}' (mode: {mode})");
                println!("─────────────────────────────────────────────────────");
                for r in &results {
                    let depth_indicator = if r.document.depth > 0 {
                        format!("  {}", "›".repeat(r.document.depth as usize))
                    } else {
                        String::new()
                    };
                    println!(
                        "  {:<20} {:<30} {:.3} {}",
                        &r.document.slug[..r.document.slug.len().min(20)],
                        &r.document.title[..r.document.title.len().min(30)],
                        r.score,
                        depth_indicator
                    );
                    if !r.chunk_heading.is_empty() {
                        println!(
                            "    ↳ {}",
                            &r.chunk_heading[..r.chunk_heading.len().min(60)]
                        );
                    }
                    if !r.chunk_snippet.is_empty() {
                        let snippet = &r.chunk_snippet[..r.chunk_snippet.len().min(80)];
                        println!("    \"{}\"", snippet);
                    }
                }
                println!();
                println!("{} result(s)", results.len());
            }
        }

        DocCommands::Update {
            id_or_slug,
            title,
            content,
            file,
            tags,
            metadata,
        } => {
            // At least one field must be provided.
            if title.is_none()
                && content.is_none()
                && file.is_none()
                && tags.is_empty()
                && metadata.is_none()
            {
                return Err(
                    "Provide at least one field to update: --title, --content, --file, --tags, --metadata"
                        .into(),
                );
            }

            // Resolve content: --content, --file, or stdin.
            let doc_content = if let Some(c) = content {
                Some(c.clone())
            } else if let Some(f) = file {
                let text = if f == "-" {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map_err(|e| format!("Failed to read stdin: {e}"))?;
                    buf
                } else {
                    std::fs::read_to_string(f).map_err(|e| format!("Failed to read file: {e}"))?
                };
                Some(text)
            } else {
                None
            };

            // Parse metadata JSON if provided.
            let meta_value: Option<serde_json::Value> = match metadata {
                Some(ref json_str) => Some(
                    serde_json::from_str(json_str)
                        .map_err(|e| format!("Invalid metadata JSON: {e}"))?,
                ),
                None => None,
            };

            let title_ref = title.as_deref();
            let content_ref = doc_content.as_deref();
            let tag_refs: Option<&[String]> = if tags.is_empty() {
                None
            } else {
                Some(tags.as_slice())
            };
            let meta_ref = meta_value.as_ref();

            let updated = uteke
                .doc_update(id_or_slug, title_ref, content_ref, tag_refs, meta_ref)
                .map_err(|e| format!("Failed to update document: {e}"))?;

            match updated {
                Some(d) => {
                    let mut changed = Vec::new();
                    if title.is_some() {
                        changed.push("title");
                    }
                    if content.is_some() || file.is_some() {
                        changed.push("content");
                    }
                    if !tags.is_empty() {
                        changed.push("tags");
                    }
                    if metadata.is_some() {
                        changed.push("metadata");
                    }

                    if cli.json {
                        println!(
                            r#"{{"slug": "{}", "title": "{}", "version": {}, "updated_fields": {}}}"#,
                            d.slug,
                            d.title,
                            d.version,
                            serde_json::to_string(&changed).unwrap_or_default()
                        );
                    } else {
                        println!("✓ Document '{id_or_slug}' updated (v{})", d.version);
                        println!("  Title: {}", d.title);
                        println!("  Fields: {}", changed.join(", "));
                    }
                }
                None => {
                    return Err(format!("Document '{id_or_slug}' not found"));
                }
            }
        }

        DocCommands::Delete { id } => {
            let (deleted, subtree_size) = uteke
                .doc_delete(id)
                .map_err(|e| format!("Failed to delete document: {e}"))?;
            if deleted {
                if cli.json {
                    println!(r#"{{"deleted": "{id}", "subtree_size": {subtree_size}}}"#);
                } else {
                    println!("✓ Document deleted: {id}");
                    if subtree_size > 1 {
                        println!("  {subtree_size} document(s) removed (cascade)");
                    }
                }
            } else {
                return Err(format!("Document not found: {id}"));
            }
        }

        DocCommands::Export { output: _ } => {
            let docs = uteke
                .doc_list(1000)
                .map_err(|e| format!("Failed to list documents for export: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else {
                for d in &docs {
                    if let Some(doc) = uteke.doc_get(&d.id).ok().flatten() {
                        println!("{}", doc.content);
                        println!();
                    }
                }
            }
        }
    }

    Ok(())
}
