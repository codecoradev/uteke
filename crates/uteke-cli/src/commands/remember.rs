//! Remember command — store a new memory.

use crate::output;
use crate::Cli;
use uteke_core::Uteke;

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    content: &str,
    tags: &[String],
    r#type: &str,
    detect_contradiction: bool,
) -> Result<(), String> {
    tracing::info!("Remembering: {content} (type: {type}, contradiction: {detect_contradiction})");
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

    if detect_contradiction {
        let (id, contradiction) = uteke
            .remember_with_contradiction(content, &tag_refs, ns, Some(r#type), true)
            .map_err(|e| format!("Failed to store memory: {e}"))?;
        tracing::info!("Memory stored with ID: {id}");
        if cli.json {
            let obj = serde_json::json!({
                "id": id,
                "contradiction": {
                    "detected": contradiction.contradicted,
                    "deprecated_id": contradiction.deprecated_id,
                    "similarity": contradiction.similarity
                }
            });
            println!("{}", obj);
        } else {
            output::print_remember_human(&id);
            if contradiction.contradicted {
                if let Some(dep_id) = &contradiction.deprecated_id {
                    println!(
                        "  \u{26a0} Contradiction detected (sim: {:.3}): deprecated {}",
                        contradiction.similarity,
                        dep_id.get(..8).unwrap_or(dep_id)
                    );
                }
            }
        }
    } else {
        let id = uteke
            .remember(content, &tag_refs, None, ns)
            .map_err(|e| format!("Failed to store memory: {e}"))?;
        tracing::info!("Memory stored with ID: {id}");
        if cli.json {
            let obj = serde_json::json!({"id": id});
            println!("{}", obj);
        } else {
            output::print_remember_human(&id);
        }
    }
    Ok(())
}
