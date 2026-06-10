//! Remember command — store a new memory.

use crate::output;
use crate::Cli;
use uteke_core::Uteke;

/// Parse --meta key:value pairs into a JSON object.
fn parse_meta_pairs(pairs: &[String]) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for pair in pairs {
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

    if detect_contradiction {
        let (id, contradiction) = uteke
            .remember_with_contradiction(content, &tag_refs, ns, Some(r#type), true)
            .map_err(|e| format!("Failed to store memory: {e}"))?;
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
                obj.as_object_mut()
                    .unwrap()
                    .insert("metadata".to_string(), meta.clone());
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
        let id = uteke
            .remember(content, &tag_refs, metadata, ns)
            .map_err(|e| format!("Failed to store memory: {e}"))?;
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
                obj.as_object_mut()
                    .unwrap()
                    .insert("metadata".to_string(), serde_json::Value::Object(m));
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
    Ok(())
}
