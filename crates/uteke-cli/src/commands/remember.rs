//! Remember command — store a new memory.

use crate::cli::Cli;
use crate::output;
use uteke_core::Uteke;

/// Tell the user when the privacy filter changed what was stored (#1464).
fn print_redaction(outcome: &uteke_core::RememberOutcome) {
    if let Some(summary) = outcome.redaction_summary() {
        println!("  \u{26a0} Secrets masked in the stored text: {summary}");
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    content: &str,
    tags: &[String],
    r#type: &str,
    detect_contradiction: bool,
    entity: Option<&str>,
    category: Option<&str>,
    author_type: Option<&str>,
    meta: &[String],
    room: Option<&str>,
    author: Option<&str>,
    source: Option<&str>,
    source_type: Option<&str>,
    timestamp: Option<&str>,
) -> Result<(), String> {
    // The content is not logged: it may hold a secret the privacy filter has
    // not masked yet (#1464).
    tracing::debug!(
        "Remembering {} bytes (type: {type}, contradiction: {detect_contradiction})",
        content.len()
    );

    // #1106: validate BEFORE any write so an invalid value fails loudly
    // without leaving a stored row behind (the post-store set would have
    // been too late — the row already exists with the schema default).
    if let Some(at) = author_type {
        uteke_core::memory::types::validate_author_type(at)
            .map_err(|e| format!("Invalid --author-type: {e}"))?;
    }
    // #1427 phase 2: the author is the memory's own, with or without a room;
    // validated (and trimmed) before any write for the same reason.
    let author: Option<&str> = author
        .map(|a| {
            uteke_core::memory::types::validate_author(a)
                .map_err(|e| format!("Invalid --author: {e}"))
        })
        .transpose()?;

    // #1464: the source is stored after the memory, so strict mode refuses a
    // source with a secret before anything is written.
    if let Some(src) = source {
        uteke
            .privacy_config()
            .mask_text(src)
            .map_err(|e| format!("Failed to store memory: {e}"))?;
    }

    // Read from stdin when content argument is "-" (#620).
    // Standard Unix convention: pipe content via echo "text" | uteke remember - --tags "test"
    let content = if content == "-" {
        use std::io::Read;
        let mut buf = String::new();
        let stdin = std::io::stdin();
        let mut handle = stdin.lock();
        handle
            .read_to_string(&mut buf)
            .map_err(|e| format!("Failed to read from stdin: {e}"))?;
        let trimmed = buf.trim().to_string();
        if trimmed.is_empty() {
            return Err(
                "Stdin is empty. Pipe content to remember or use positional argument.".to_string(),
            );
        }
        // We need to extend the lifetime — leak is safe since we only need it for this function
        Box::leak(trimmed.into_boxed_str())
    } else {
        content
    };
    // #1232: timestamp anchor — validate + prepend BEFORE any write so an
    // invalid value fails loudly without storing a row.
    let content: &str = match timestamp {
        Some(ts) => {
            let anchored = super::anchor::apply_timestamp_anchor(content, ts)?;
            Box::leak(anchored.into_boxed_str())
        }
        None => content,
    };
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

    let metadata = uteke_core::metadata_from_cli_flags(entity, category, meta);

    let stored_id: String; // captured for set_source (#348)

    if detect_contradiction {
        let (outcome, contradiction) = uteke
            .remember_with_contradiction(
                content,
                &tag_refs,
                metadata.clone(),
                ns,
                Some(r#type),
                true,
                0.65,
            )
            .map_err(|e| format!("Failed to store memory: {e}"))?;
        let id = outcome.id.clone();
        let embed_warning = outcome.warning.clone();
        stored_id = id.clone();
        tracing::info!("Memory stored with ID: {id}");
        // #1084/#1106: persist the (already-validated) author_type.
        if let Some(at) = author_type {
            if let Err(e) = uteke.set_author_type(&id, at) {
                return Err(format!("Failed to set author_type: {e}"));
            }
        }
        if let Some(a) = author {
            uteke
                .set_author(&id, Some(a))
                .map_err(|e| format!("Failed to set author: {e}"))?;
        }
        if cli.json {
            let mut obj = serde_json::json!({
                "id": id,
                "author_type": author_type.unwrap_or("agent"),
                "embedding_written": embed_warning.is_none(),
                "redacted": outcome.redacted(),
                "warning": embed_warning,
                "contradiction": {
                    "detected": contradiction.contradicted,
                    "deprecated_id": contradiction.deprecated_id,
                    "similarity": contradiction.similarity
                }
            });
            if let Some(ref meta) = metadata {
                if let Some(map) = obj.as_object_mut() {
                    map.insert("metadata".to_string(), meta.clone());
                }
            }
            if let Some(a) = author {
                obj["author"] = serde_json::json!(a);
            }
            println!("{obj}");
        } else {
            output::print_remember_human(&id);
            print_redaction(&outcome);
            if let Some(ref warn) = embed_warning {
                println!("  \u{26a0} Embedding NOT written: {warn}");
                println!("    Memory is keyword-searchable only; run `uteke repair` to backfill.");
            }
            if contradiction.contradicted {
                if let Some(dep_id) = &contradiction.deprecated_id {
                    println!(
                        "  \u{26a0} Contradiction detected (sim: {:.3}): deprecated {}",
                        contradiction.similarity,
                        dep_id.get(..8).unwrap_or(dep_id)
                    );
                }
            }
            if let Some(entity_name) = entity {
                println!("  entity: {entity_name}");
            }
            if let Some(cat) = category {
                println!("  category: {cat}");
            }
        }
    } else {
        let outcome = if let Some(room_id) = room {
            // Room mode: store memory and link to room with author
            let author_name = author.unwrap_or("anonymous");
            uteke
                .remember_in_room(
                    content,
                    &tag_refs,
                    metadata.clone(),
                    ns,
                    r#type,
                    room_id,
                    author_name,
                )
                .map_err(|e| format!("Failed to store memory in room: {e}"))?
        } else {
            uteke
                .remember_detailed(content, &tag_refs, metadata, ns)
                .map_err(|e| format!("Failed to store memory: {e}"))?
        };
        let id = outcome.id.clone();
        stored_id = id.clone();
        tracing::info!("Memory stored with ID: {id}");
        // #1084/#1106: persist the (already-validated) author_type.
        if let Some(at) = author_type {
            if let Err(e) = uteke.set_author_type(&id, at) {
                return Err(format!("Failed to set author_type: {e}"));
            }
        }
        if let Some(a) = author {
            uteke
                .set_author(&id, Some(a))
                .map_err(|e| format!("Failed to set author: {e}"))?;
        }
        if cli.json {
            let mut obj = serde_json::json!({
                "id": id,
                "author_type": author_type.unwrap_or("agent"),
                "embedding_written": outcome.embedding_written,
                "redacted": outcome.redacted(),
                "warning": outcome.warning,
            });
            // Metadata for JSON output (the write consumed its copy).
            if let Some(m) = uteke_core::metadata_from_cli_flags(entity, category, meta) {
                obj["metadata"] = m;
            }
            if let Some(a) = author {
                obj["author"] = serde_json::json!(a);
            }
            println!("{obj}");
        } else {
            output::print_remember_human(&id);
            print_redaction(&outcome);
            if let Some(ref warn) = outcome.warning {
                println!("  \u{26a0} Embedding NOT written: {warn}");
                println!("    Memory is keyword-searchable only; run `uteke repair` to backfill.");
            }
            if let Some(entity_name) = entity {
                println!("  entity: {entity_name}");
            }
            if let Some(cat) = category {
                println!("  category: {cat}");
            }
        }
    }

    // Set source provenance using the exact stored ID (#348).
    if source.is_some() || source_type.is_some() {
        let st = source_type.unwrap_or("user");
        uteke
            .set_source(&stored_id, source, st)
            .map_err(|e| format!("Failed to set source: {e}"))?;
    }

    Ok(())
}
