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
    r#type: Option<&str>,
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
        "Remembering {} bytes (type: {:?}, contradiction: {detect_contradiction})",
        content.len(),
        r#type
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
    // The same decode as `POST /remember` and MCP (#1439): content, tags and
    // type are validated, the type is copied into the metadata, and an omitted
    // type stays `None`, which means "infer it from the content".
    let request = uteke_core::RememberRequest::decode(uteke_core::RememberInput {
        content: content.to_string(),
        tags: tags.to_vec(),
        namespace: ns.map(str::to_string),
        memory_type: r#type.map(str::to_string),
        detect_contradiction,
        metadata: uteke_core::metadata_from_cli_flags(entity, category, meta),
        source: source.map(str::to_string),
        source_type: source_type.map(str::to_string),
        author_type: author_type.map(str::to_string),
        room: room.map(str::to_string),
        author: author.map(str::to_string),
        ..uteke_core::RememberInput::default()
    })
    .map_err(|e| format!("Failed to store memory: {e}"))?;

    // The write and its provenance follow-ups (source, author_type, author)
    // run in the core, with one failure policy (#1516).
    let run = uteke
        .run_remember(&request)
        .map_err(|e| format!("Failed to store memory: {e}"))?;
    let outcome = run.outcome;
    let id = outcome.id.clone();
    tracing::info!("Memory stored with ID: {id}");
    for w in &run.provenance_warnings {
        eprintln!("warning: stored, but could not record {w}");
    }

    if cli.json {
        let mut obj = serde_json::json!({
            "id": id,
            "author_type": author_type.unwrap_or("agent"),
            "embedding_written": outcome.embedding_written,
            "redacted": outcome.redacted(),
            "warning": outcome.warning,
        });
        if let Some(c) = &run.contradiction {
            obj["contradiction"] = serde_json::json!({
                "detected": c.contradicted,
                "deprecated_id": c.deprecated_id,
                "similarity": c.similarity
            });
        }
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
        if let Some(c) = &run.contradiction
            && c.contradicted
            && let Some(dep_id) = &c.deprecated_id
        {
            println!(
                "  \u{26a0} Contradiction detected (sim: {:.3}): deprecated {}",
                c.similarity,
                dep_id.get(..8).unwrap_or(dep_id)
            );
        }
        if let Some(entity_name) = entity {
            println!("  entity: {entity_name}");
        }
        if let Some(cat) = category {
            println!("  category: {cat}");
        }
    }
    Ok(())
}
