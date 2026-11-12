//! Document CLI commands (#411, #438).

use crate::Config;
use crate::cli::{Cli, DocCommands};
use crate::output;
use std::collections::HashMap;
use uteke_core::Uteke;

/// Documents fetched per round trip by `doc export`.
const EXPORT_PAGE_SIZE: usize = 500;

/// JSON mode: one array of full documents. Text mode: each document body
/// followed by a blank line (the historical format). With `front_matter`
/// (text mode only, #1470) each body is preceded by a YAML block naming its
/// slug, title, tags and parent slug; the map resolves parent IDs to slugs.
fn render_export(
    docs: &[uteke_core::Document],
    json: bool,
    front_matter: Option<&HashMap<String, String>>,
) -> String {
    if json {
        let mut out = serde_json::to_string_pretty(docs).unwrap_or_else(|_| "[]".to_string());
        out.push('\n');
        out
    } else {
        docs.iter()
            .map(|d| match front_matter {
                Some(parent_slugs) => {
                    format!("{}{}\n\n", render_front_matter(d, parent_slugs), d.content)
                }
                None => format!("{}\n\n", d.content),
            })
            .collect::<String>()
    }
}

/// YAML front-matter for one document. Every value is written as a JSON
/// string, which is a valid YAML double-quoted scalar, so `:`, `#`, quotes and
/// newlines in titles and tags need no further escaping.
fn render_front_matter(
    doc: &uteke_core::Document,
    parent_slugs: &HashMap<String, String>,
) -> String {
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
    let tags = doc
        .tags
        .iter()
        .map(|t| quote(t))
        .collect::<Vec<_>>()
        .join(", ");
    let parent = doc
        .parent_id
        .as_deref()
        .and_then(|id| parent_slugs.get(id))
        .map_or("", String::as_str);
    format!(
        "---\nslug: {}\ntitle: {}\ntags: [{tags}]\nparent: {}\n---\n",
        quote(&doc.slug),
        quote(&doc.title),
        quote(parent)
    )
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

/// Where `doc similar` takes its subject from.
#[derive(Debug, PartialEq, Eq)]
enum SimilarSource {
    /// A stored document, by slug or id.
    Document(String),
    /// A draft text that is not stored.
    Text(String),
}

/// Exactly one of a slug, `--text` and `--file` (`-` reads stdin through
/// `read_stdin`; clap already refuses two at once).
fn similar_source(
    slug: Option<&str>,
    text: Option<&str>,
    file: Option<&str>,
    read_stdin: impl FnOnce() -> Result<String, String>,
) -> Result<SimilarSource, String> {
    match (slug, text, file) {
        (Some(s), None, None) => Ok(SimilarSource::Document(s.to_string())),
        (None, Some(t), None) => Ok(SimilarSource::Text(t.to_string())),
        (None, None, Some("-")) => read_stdin().map(SimilarSource::Text),
        (None, None, Some(f)) => std::fs::read_to_string(f)
            .map(SimilarSource::Text)
            .map_err(|e| format!("Failed to read file: {e}")),
        (None, None, None) => {
            Err("Provide a document slug, or --text <text>, or --file <path> (- for stdin)".into())
        }
        _ => Err("Give only one of: a document slug, --text, --file".into()),
    }
}

fn read_stdin_to_string() -> Result<String, String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("Failed to read stdin: {e}"))?;
    Ok(buf)
}

/// The text view of `doc similar`.
fn render_similar(subject: &str, r: &uteke_core::DocSimilarReport) -> Vec<String> {
    let mut lines = Vec::new();
    if !r.source_embedded {
        lines.push(match &r.source {
            Some(_) => format!(
                "{subject} has no vectors yet, so nothing can be compared. \
                 Run `uteke repair --reembed` to embed its chunks."
            ),
            None => "The text could not be embedded.".to_string(),
        });
        return lines;
    }
    lines.push(format!(
        "Documents similar to {subject} (threshold {:.2}; similar_to {:.2}, possible_duplicate {:.2})",
        r.threshold, r.similar_threshold, r.duplicate_threshold
    ));
    lines.push("─────────────────────────────────────────────────────".to_string());
    for h in &r.hits {
        lines.push(format!(
            "  {:<18} {:.3}  {:<20} {}",
            h.relation,
            h.score,
            output::char_prefix(&h.document.slug, 20),
            output::char_prefix(&h.document.title, 40),
        ));
    }
    if r.hits.is_empty() {
        lines.push("  none".to_string());
    }
    lines.push(String::new());
    lines.push(if r.matches > r.hits.len() {
        format!("{} match(es), showing {}", r.matches, r.hits.len())
    } else {
        format!("{} match(es)", r.matches)
    });
    if r.documents_without_vector > 0 {
        lines.push(format!(
            "{} document(s) skipped: no vectors yet (`uteke repair --reembed`)",
            r.documents_without_vector
        ));
    }
    lines
}

/// The text view of `doc duplicates`.
fn render_duplicates(r: &uteke_core::DocDuplicateReport) -> Vec<String> {
    let mut lines = vec![
        format!(
            "Near-duplicate documents (threshold {:.2}, {} document(s) scanned)",
            r.threshold, r.scanned
        ),
        "─────────────────────────────────────────────────────".to_string(),
    ];
    for p in &r.pairs {
        lines.push(format!(
            "  {:.3}  {}  ~  {}",
            p.score,
            output::char_prefix(&p.a.slug, 28),
            output::char_prefix(&p.b.slug, 28),
        ));
    }
    if r.pairs.is_empty() {
        lines.push("  none".to_string());
    }
    lines.push(String::new());
    lines.push(if r.total_pairs > r.pairs.len() {
        format!("{} pair(s), showing {}", r.total_pairs, r.pairs.len())
    } else {
        format!("{} pair(s)", r.total_pairs)
    });
    for (i, g) in r.groups.iter().enumerate() {
        lines.push(format!("Group {}: {}", i + 1, g.join(", ")));
    }
    if r.documents_without_vector > 0 {
        lines.push(format!(
            "{} document(s) skipped: no vectors yet (`uteke repair --reembed`)",
            r.documents_without_vector
        ));
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

        DocCommands::Similar {
            slug,
            text,
            file,
            threshold,
            limit,
            namespace,
        } => {
            let source = similar_source(
                slug.as_deref(),
                text.as_deref(),
                file.as_deref(),
                read_stdin_to_string,
            )?;
            let ns = namespace.as_deref();
            let (subject, report) = match &source {
                SimilarSource::Document(s) => (
                    format!("'{s}'"),
                    uteke.doc_similar(s, ns, *threshold, *limit),
                ),
                SimilarSource::Text(t) => (
                    "the text".to_string(),
                    uteke.doc_similar_to_text(t, ns, *threshold, *limit),
                ),
            };
            let report = report.map_err(|e| format!("Failed to find similar documents: {e}"))?;
            if cli.json {
                output::print_json(&report);
            } else {
                for line in render_similar(&subject, &report) {
                    println!("{line}");
                }
            }
        }

        DocCommands::Duplicates {
            subtree,
            namespace,
            threshold,
            limit,
        } => {
            let report = uteke
                .doc_duplicate_report(namespace.as_deref(), subtree.as_deref(), *threshold, *limit)
                .map_err(|e| format!("Failed to build the duplicate report: {e}"))?;
            if cli.json {
                output::print_json(&report);
            } else {
                for line in render_duplicates(&report) {
                    println!("{line}");
                }
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

        DocCommands::Export {
            output,
            front_matter,
        } => {
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
            let parent_slugs = front_matter.then(|| {
                let mut slugs: HashMap<String, String> = docs
                    .iter()
                    .map(|d| (d.id.clone(), d.slug.clone()))
                    .collect();
                // A parent outside the exported set (another namespace) is
                // looked up once so the hierarchy is still named.
                for parent_id in docs.iter().filter_map(|d| d.parent_id.as_deref()) {
                    if !slugs.contains_key(parent_id)
                        && let Some(parent) = uteke.doc_get(parent_id).ok().flatten()
                    {
                        slugs.insert(parent.id, parent.slug);
                    }
                }
                slugs
            });
            let rendered = render_export(&docs, cli.json, parent_slugs.as_ref());
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
    use std::collections::HashMap;
    use yaml_rust2::{Yaml, YamlLoader};

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

    fn no_parents() -> HashMap<String, String> {
        HashMap::new()
    }

    /// Splits front-matter output into (front-matter YAML, body) per document.
    fn split_front_matter(out: &str) -> Vec<(String, String)> {
        let mut docs = Vec::new();
        let mut rest = out;
        while let Some(after_open) = rest.strip_prefix("---\n") {
            let end = after_open.find("\n---\n").expect("closing fence");
            let yaml = after_open[..end].to_string();
            let after_close = &after_open[end + "\n---\n".len()..];
            let next = after_close
                .find("\n\n---\n")
                .map_or(after_close.len(), |i| i + 2);
            docs.push((yaml, after_close[..next].to_string()));
            rest = &after_close[next..];
        }
        assert!(rest.is_empty(), "unparsed tail: {rest:?}");
        docs
    }

    fn parse(yaml: &str) -> Yaml {
        let mut docs = YamlLoader::load_from_str(yaml).expect("front-matter is valid YAML");
        assert_eq!(docs.len(), 1);
        docs.remove(0)
    }

    #[test]
    fn text_export_is_the_bodies_separated_by_blank_lines() {
        let out = render_export(&[doc("a", "# A"), doc("b", "# B")], false, None);
        assert_eq!(out, "# A\n\n# B\n\n");
        assert_eq!(render_export(&[], false, None), "");
    }

    #[test]
    fn json_export_is_a_parseable_array_of_full_documents() {
        let out = render_export(&[doc("a", "# A")], true, None);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v[0]["slug"], "a");
        assert_eq!(v[0]["content"], "# A");
        assert_eq!(render_export(&[], true, None).trim(), "[]");
    }

    #[test]
    fn front_matter_keeps_the_bodies_byte_identical() {
        let docs = [doc("a", "# A\n\ntext"), doc("b", "# B")];
        let plain = render_export(&docs, false, None);
        let with = render_export(&docs, false, Some(&no_parents()));
        assert!(with.starts_with("---\nslug: "), "{with}");
        let bodies: String = split_front_matter(&with)
            .into_iter()
            .map(|(_, body)| body)
            .collect();
        assert_eq!(bodies, plain);
    }

    #[test]
    fn front_matter_parses_as_yaml_with_awkward_titles() {
        let mut d = doc("tricky", "body");
        d.title = "Note: \"quoted\" 'single' # not a comment\nsecond line\t\\".to_string();
        d.tags = vec!["a:b".to_string(), "#hash".to_string(), "- dash".to_string()];
        let out = render_export(std::slice::from_ref(&d), false, Some(&no_parents()));
        let parts = split_front_matter(&out);
        assert_eq!(parts.len(), 1);
        let y = parse(&parts[0].0);
        assert_eq!(y["slug"].as_str(), Some("tricky"));
        assert_eq!(y["title"].as_str(), Some(d.title.as_str()));
        let tags: Vec<&str> = y["tags"]
            .as_vec()
            .unwrap()
            .iter()
            .filter_map(Yaml::as_str)
            .collect();
        assert_eq!(tags, ["a:b", "#hash", "- dash"]);
        assert_eq!(y["parent"].as_str(), Some(""), "a root has an empty parent");
    }

    #[test]
    fn front_matter_names_the_parent_slug_for_nested_documents() {
        let root = doc("book", "# Book");
        let mut chapter = doc("chapter-1", "# One");
        chapter.parent_id = Some("id-book".to_string());
        let mut section = doc("section-1", "# 1.1");
        section.parent_id = Some("id-chapter".to_string());
        let parents = HashMap::from([
            ("id-book".to_string(), "book".to_string()),
            ("id-chapter".to_string(), "chapter-1".to_string()),
        ]);
        let out = render_export(&[root, chapter, section], false, Some(&parents));
        let parents: Vec<String> = split_front_matter(&out)
            .iter()
            .map(|(yaml, _)| parse(yaml)["parent"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(parents, ["", "book", "chapter-1"]);
    }

    #[test]
    fn json_export_ignores_the_front_matter_flag() {
        let docs = [doc("a", "# A")];
        assert_eq!(
            render_export(&docs, true, Some(&no_parents())),
            render_export(&docs, true, None)
        );
    }
}

#[cfg(test)]
mod doc_similar_tests {
    use super::{SimilarSource, render_duplicates, render_similar, similar_source};
    use uteke_core::{
        DocDuplicatePair, DocDuplicateReport, DocSimilarHit, DocSimilarReport, DocumentSummary,
    };

    fn summary(slug: &str, title: &str) -> DocumentSummary {
        DocumentSummary {
            id: format!("id-{slug}"),
            slug: slug.to_string(),
            title: title.to_string(),
            namespace: None,
            author: None,
            version: 1,
            updated_at: "2026-11-12T00:00:00Z".to_string(),
            parent_id: None,
            depth: 0,
            has_children: false,
            sort_order: 0,
        }
    }

    fn hit(slug: &str, score: f32, relation: &str) -> DocSimilarHit {
        DocSimilarHit {
            document: summary(slug, &format!("Title of {slug}")),
            score,
            relation: relation.to_string(),
        }
    }

    fn report(hits: Vec<DocSimilarHit>) -> DocSimilarReport {
        DocSimilarReport {
            source: Some(summary("src", "Source")),
            threshold: 0.8,
            similar_threshold: 0.8,
            duplicate_threshold: 0.92,
            source_embedded: true,
            documents_without_vector: 0,
            matches: hits.len(),
            hits,
        }
    }

    fn no_stdin() -> Result<String, String> {
        panic!("stdin must not be read")
    }

    #[test]
    fn the_source_is_exactly_one_of_slug_text_and_file() {
        assert_eq!(
            similar_source(Some("a"), None, None, no_stdin),
            Ok(SimilarSource::Document("a".into()))
        );
        assert_eq!(
            similar_source(None, Some("draft"), None, no_stdin),
            Ok(SimilarSource::Text("draft".into()))
        );
        assert_eq!(
            similar_source(None, None, Some("-"), || Ok("from stdin".into())),
            Ok(SimilarSource::Text("from stdin".into()))
        );
        let e = similar_source(None, None, None, no_stdin).unwrap_err();
        assert!(e.starts_with("Provide a document slug"), "{e}");
        let e = similar_source(Some("a"), Some("t"), None, no_stdin).unwrap_err();
        assert_eq!(e, "Give only one of: a document slug, --text, --file");
        assert!(similar_source(None, Some("t"), Some("f"), no_stdin).is_err());
        let e = similar_source(None, None, Some("-"), || Err("boom".into())).unwrap_err();
        assert_eq!(e, "boom");
    }

    #[test]
    fn a_file_is_read_as_the_candidate_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.md");
        std::fs::write(&path, "# Draft\n\nbody").unwrap();
        assert_eq!(
            similar_source(None, None, path.to_str(), no_stdin),
            Ok(SimilarSource::Text("# Draft\n\nbody".into()))
        );
        let missing = dir.path().join("none.md");
        let e = similar_source(None, None, missing.to_str(), no_stdin).unwrap_err();
        assert!(e.starts_with("Failed to read file:"), "{e}");
    }

    #[test]
    fn similar_text_lists_each_hit_with_relation_score_slug_and_title() {
        let lines = render_similar(
            "'src'",
            &report(vec![
                hit("dup", 0.96, "possible_duplicate"),
                hit("sim", 0.8, "similar_to"),
            ]),
        );
        assert_eq!(
            lines,
            vec![
                "Documents similar to 'src' (threshold 0.80; similar_to 0.80, possible_duplicate 0.92)",
                "─────────────────────────────────────────────────────",
                "  possible_duplicate 0.960  dup                  Title of dup",
                "  similar_to         0.800  sim                  Title of sim",
                "",
                "2 match(es)",
            ]
        );
    }

    #[test]
    fn similar_text_says_when_there_is_nothing_and_when_the_list_is_cut() {
        let lines = render_similar("'src'", &report(vec![]));
        assert_eq!(lines[2], "  none");
        assert_eq!(lines.last().unwrap(), "0 match(es)");
        let mut cut = report(vec![hit("dup", 1.0, "possible_duplicate")]);
        cut.matches = 5;
        assert_eq!(
            render_similar("'src'", &cut).last().unwrap(),
            "5 match(es), showing 1"
        );
    }

    #[test]
    fn similar_text_points_to_repair_for_documents_without_vectors() {
        let mut r = report(vec![]);
        r.documents_without_vector = 3;
        let lines = render_similar("'src'", &r);
        assert_eq!(
            lines.last().unwrap(),
            "3 document(s) skipped: no vectors yet (`uteke repair --reembed`)"
        );
        r.source_embedded = false;
        let lines = render_similar("'src'", &r);
        assert_eq!(
            lines,
            vec![
                "'src' has no vectors yet, so nothing can be compared. \
                 Run `uteke repair --reembed` to embed its chunks."
            ]
        );
    }

    #[test]
    fn duplicates_text_lists_pairs_and_groups() {
        let pair = |a: &str, b: &str, score: f32| DocDuplicatePair {
            a: summary(a, a),
            b: summary(b, b),
            score,
        };
        let r = DocDuplicateReport {
            threshold: 0.92,
            scanned: 6,
            documents_without_vector: 1,
            total_pairs: 3,
            pairs: vec![pair("a", "b", 1.0), pair("a", "c", 0.96)],
            groups: vec![vec!["a".into(), "b".into(), "c".into()]],
        };
        assert_eq!(
            render_duplicates(&r),
            vec![
                "Near-duplicate documents (threshold 0.92, 6 document(s) scanned)",
                "─────────────────────────────────────────────────────",
                "  1.000  a  ~  b",
                "  0.960  a  ~  c",
                "",
                "3 pair(s), showing 2",
                "Group 1: a, b, c",
                "1 document(s) skipped: no vectors yet (`uteke repair --reembed`)",
            ]
        );
        let empty = DocDuplicateReport {
            threshold: 0.92,
            scanned: 0,
            documents_without_vector: 0,
            total_pairs: 0,
            pairs: vec![],
            groups: vec![],
        };
        assert_eq!(
            render_duplicates(&empty),
            vec![
                "Near-duplicate documents (threshold 0.92, 0 document(s) scanned)",
                "─────────────────────────────────────────────────────",
                "  none",
                "",
                "0 pair(s)",
            ]
        );
    }
}
