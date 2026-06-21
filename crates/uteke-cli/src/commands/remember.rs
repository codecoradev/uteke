//! Remember command — store a new memory.

use crate::cli::Cli;
use crate::output;
use uteke_core::Uteke;

/// Parse --meta key:value pairs into a JSON object.
/// Special handling for `rel:type:target` directives which are
/// accumulated into a `relationships` array.
fn parse_meta_pairs(pairs: &[String]) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    let mut relationships: Vec<serde_json::Value> = Vec::new();

    for pair in pairs {
        // Check for relationship directive: rel:type:target
        if let Some((rel_type, target)) = uteke_core::is_relationship_meta(pair) {
            relationships.push(serde_json::json!({
                "type": rel_type,
                "target": target
            }));
            continue;
        }

        if let Some((key, value)) = pair.split_once(':') {
            let val = if let Ok(n) = value.parse::<f64>() {
                serde_json::Value::from(n)
            } else if value == "true" {
                serde_json::Value::Bool(true)
            } else if value == "false" {
                serde_json::Value::Bool(false)
            } else {
                serde_json::Value::String(value.to_string())
            };
            map.insert(key.to_string(), val);
        } else {
            map.insert(pair.clone(), serde_json::Value::Bool(true));
        }
    }

    if !relationships.is_empty() {
        map.insert(
            "relationships".to_string(),
            serde_json::Value::Array(relationships),
        );
    }

    map
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
    meta: &[String],
    room: Option<&str>,
    author: Option<&str>,
    source: Option<&str>,
    source_type: Option<&str>,
) -> Result<(), String> {
    tracing::debug!("Remembering: {content} (type: {type}, contradiction: {detect_contradiction})");
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

    // Build metadata JSON from flags
    let mut meta_map = serde_json::Map::new();
    if let Some(entity_name) = entity {
        meta_map.insert(
            "entity".to_string(),
            serde_json::Value::String(entity_name.to_string()),
        );
    }
    if let Some(cat) = category {
        meta_map.insert(
            "category".to_string(),
            serde_json::Value::String(cat.to_string()),
        );
    }
    let extra = parse_meta_pairs(meta);
    for (k, v) in extra {
        meta_map.insert(k, v);
    }

    let metadata = if meta_map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(meta_map))
    };

    let stored_id: String; // captured for set_source (#348)

    if detect_contradiction {
        let (id, contradiction) = uteke
            .remember_with_contradiction(content, &tag_refs, ns, Some(r#type), true, 0.65)
            .map_err(|e| format!("Failed to store memory: {e}"))?;
        stored_id = id.clone();
        tracing::info!("Memory stored with ID: {id}");
        if cli.json {
            let mut obj = serde_json::json!({
                "id": id,
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
            println!("{obj}");
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
            if let Some(entity_name) = entity {
                println!("  entity: {entity_name}");
            }
            if let Some(cat) = category {
                println!("  category: {cat}");
            }
        }
    } else {
        let id = if let Some(room_id) = room {
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
                .remember(content, &tag_refs, metadata, ns)
                .map_err(|e| format!("Failed to store memory: {e}"))?
        };
        stored_id = id.clone();
        tracing::info!("Memory stored with ID: {id}");
        if cli.json {
            let mut obj = serde_json::json!({"id": id});
            // Reconstruct metadata for JSON output (already consumed by remember)
            if entity.is_some() || category.is_some() || !meta.is_empty() {
                let mut m = serde_json::Map::new();
                if let Some(e) = entity {
                    m.insert(
                        "entity".to_string(),
                        serde_json::Value::String(e.to_string()),
                    );
                }
                if let Some(c) = category {
                    m.insert(
                        "category".to_string(),
                        serde_json::Value::String(c.to_string()),
                    );
                }
                for (k, v) in parse_meta_pairs(meta) {
                    m.insert(k, v);
                }
                if let Some(map) = obj.as_object_mut() {
                    map.insert("metadata".to_string(), serde_json::Value::Object(m));
                }
            }
            println!("{obj}");
        } else {
            output::print_remember_human(&id);
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
