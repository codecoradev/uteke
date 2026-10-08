//! Document CLI commands (#411, #438).

use crate::Config;
use crate::cli::{Cli, DocCommands};
use crate::output;
use uteke_core::Uteke;

/// Documents fetched per round trip by `doc export`.
const EXPORT_PAGE_SIZE: usize = 500;

/// JSON mode: one array of full documents. Text mode: each document body
/// followed by a blank line (the historical format).
fn render_export(docs: &[uteke_core::Document], json: bool) -> String {
    if json {
        let mut out = serde_json::to_string_pretty(docs).unwrap_or_else(|_| "[]".to_string());
        out.push('\n');
        out
    } else {
        docs.iter()
            .map(|d| format!("{}\n\n", d.content))
            .collect::<String>()
    }
}

/// Render the document forest as indented lines: one line per document, roots
/// first, children under their parent in the order `children_of` returns them.
/// A visited-set guards against cyclic parent links in damaged data.
fn render_doc_tree(
    roots: &[uteke_core::DocumentSummary],
    mut children_of: impl FnMut(&str) -> Vec<uteke_core::DocumentSummary>,
) -> Vec<String> {
    let indent = "  ";
    let mut lines = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // (document, depth); reversed so popping yields document order.
    let mut stack: Vec<(uteke_core::DocumentSummary, usize)> =
        roots.iter().rev().cloned().map(|d| (d, 0)).collect();
    while let Some((doc, depth)) = stack.pop() {
        if !seen.insert(doc.id.clone()) {
            continue;
        }
        let children = children_of(&doc.id);
        let tree_char = if children.is_empty() {
            "├─"
        } else {
            "┬─"
        };
        lines.push(format!(
            "{}{tree_char} {:<20} {:<30} v{}",
            indent.repeat(depth),
            output::char_prefix(&doc.slug, 20),
            output::char_prefix(&doc.title, 30),
            doc.version
        ));
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    lines
}

/// Run document subcommands.
pub(crate) fn run(
    cli: &Cli,
    uteke: &mut Uteke,
    command: &DocCommands,
    _config: &Config,
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
                let json = serde_json::json!({
                    "id": id,
                    "slug": slug,
                    "parent": parent,
                    "status": "created",
                });
                println!("{json}");
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

        DocCommands::List {
            limit,
            tree,
            namespace,
        } => {
            let ns = namespace.as_deref().or(cli.namespace.as_deref());
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
                    // Roots come from `doc_list_roots`; every descendant is
                    // fetched on the way down. The tree walk used to look each
                    // node up among the roots only, so children never printed
                    // (#1332).
                    let lines = render_doc_tree(&docs, |id| {
                        uteke.doc_list_children(id, 1000).unwrap_or_default()
                    });
                    for line in &lines {
                        println!("{line}");
                    }
                    println!();
                    println!("{} document(s)", lines.len());
                    return Ok(());
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
                            output::char_prefix(&d.slug, 20),
                            output::char_prefix(&d.title, 30),
                            d.version,
                            output::char_prefix(&d.updated_at, 10),
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
                        output::char_prefix(&d.slug, 20),
                        output::char_prefix(&d.title, 30),
                        d.version,
                        output::char_prefix(&d.updated_at, 10)
                    );
                }
                println!();
                println!("{} child(ren)", docs.len());
            }
        }

        DocCommands::Move { id_or_slug, parent } => {
            let new_parent = parent.as_deref();
            let affected = uteke
                .doc_move(id_or_slug, new_parent, None)
                .map_err(|e| format!("Failed to move document: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "moved": id_or_slug,
                        "parent": new_parent.unwrap_or("(root)"),
                        "affected": affected,
                    })
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
                        output::char_prefix(&d.slug, 20),
                        output::char_prefix(&d.title, 30),
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
                        output::char_prefix(&r.document.slug, 20),
                        output::char_prefix(&r.document.title, 30),
                        r.score,
                        depth_indicator
                    );
                    if !r.chunk_heading.is_empty() {
                        println!("    ↳ {}", output::char_prefix(&r.chunk_heading, 60));
                    }
                    if !r.chunk_snippet.is_empty() {
                        let snippet = output::char_prefix(&r.chunk_snippet, 80);
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
                Some(json_str) => Some(
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
                            "{}",
                            serde_json::json!({
                                "slug": d.slug,
                                "title": d.title,
                                "version": d.version,
                                "updated_fields": changed,
                            })
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
                    let json = serde_json::json!({
                        "deleted": id,
                        "subtree_size": subtree_size,
                    });
                    println!("{json}");
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

        DocCommands::Export { output } => {
            // Every document of the namespace (the old export stopped at 1000
            // rows, ignored the namespace and ignored --output; #1332).
            let ns = cli.namespace.as_deref();
            let mut docs = Vec::new();
            let mut offset = 0;
            loop {
                let page = uteke
                    .doc_list_page(ns, EXPORT_PAGE_SIZE, offset)
                    .map_err(|e| format!("Failed to list documents for export: {e}"))?;
                if page.is_empty() {
                    break;
                }
                offset += page.len();
                for summary in &page {
                    if let Some(doc) = uteke.doc_get(&summary.id).ok().flatten() {
                        docs.push(doc);
                    }
                }
                if page.len() < EXPORT_PAGE_SIZE {
                    break;
                }
            }
            let rendered = render_export(&docs, cli.json);
            match output.as_deref() {
                Some(path) if path != "-" => {
                    std::fs::write(path, &rendered)
                        .map_err(|e| format!("Failed to write {path}: {e}"))?;
                    eprintln!("Exported {} document(s) to {path}", docs.len());
                }
                _ => print!("{rendered}"),
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod doc_tree_tests {
    use super::render_doc_tree;
    use uteke_core::DocumentSummary;

    fn doc(id: &str, parent: Option<&str>) -> DocumentSummary {
        DocumentSummary {
            id: id.to_string(),
            slug: format!("slug-{id}"),
            title: format!("Title {id}"),
            namespace: None,
            author: None,
            version: 1,
            updated_at: "2026-10-08T00:00:00Z".to_string(),
            parent_id: parent.map(str::to_string),
            depth: 0,
            has_children: false,
            sort_order: 0,
        }
    }

    fn children(id: &str) -> Vec<DocumentSummary> {
        match id {
            "root" => vec![doc("a", Some("root")), doc("b", Some("root"))],
            "a" => vec![doc("a1", Some("a"))],
            _ => vec![],
        }
    }

    #[test]
    fn descendants_are_printed_in_order_with_indentation() {
        let lines = render_doc_tree(&[doc("root", None), doc("solo", None)], children);
        assert_eq!(lines.len(), 5, "root, a, a1, b, solo: {lines:#?}");
        let slugs: Vec<&str> = lines
            .iter()
            .map(|l| l.split_whitespace().nth(1).unwrap())
            .collect();
        assert_eq!(
            slugs,
            ["slug-root", "slug-a", "slug-a1", "slug-b", "slug-solo"]
        );
        assert!(lines[0].starts_with("┬─"), "root has children");
        assert!(
            lines[1].starts_with("  ┬─"),
            "a is indented once and has a child"
        );
        assert!(
            lines[2].starts_with("    ├─"),
            "a1 is indented twice and a leaf"
        );
    }

    #[test]
    fn cyclic_parent_links_do_not_loop_forever() {
        let lines = render_doc_tree(&[doc("x", None)], |id| match id {
            "x" => vec![doc("y", Some("x"))],
            "y" => vec![doc("x", Some("y"))],
            _ => vec![],
        });
        assert_eq!(lines.len(), 2);
    }
}

#[cfg(test)]
mod doc_export_tests {
    use super::render_export;

    fn doc(slug: &str, content: &str) -> uteke_core::Document {
        serde_json::from_value(serde_json::json!({
            "id": slug, "slug": slug, "title": slug, "content": content,
            "tags": [], "metadata": null, "version": 1, "content_type": "markdown",
            "created_at": "2026-10-08T00:00:00Z", "updated_at": "2026-10-08T00:00:00Z",
            "path": format!("/{slug}/"), "depth": 0, "sort_order": 0,
            "has_children": false
        }))
        .unwrap()
    }

    #[test]
    fn text_export_is_the_bodies_separated_by_blank_lines() {
        let out = render_export(&[doc("a", "# A"), doc("b", "# B")], false);
        assert_eq!(out, "# A\n\n# B\n\n");
        assert_eq!(render_export(&[], false), "");
    }

    #[test]
    fn json_export_is_a_parseable_array_of_full_documents() {
        let out = render_export(&[doc("a", "# A")], true);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v[0]["slug"], "a");
        assert_eq!(v[0]["content"], "# A");
        assert_eq!(render_export(&[], true).trim(), "[]");
    }
}
