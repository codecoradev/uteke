//! LLM executor for segment-level room consolidation (#1088, final phase).
//!
//! Reuses the extraction endpoint setup (`ExtractionConfig` — same model,
//! key, and base URL as the import/extraction pipeline) so one setup serves
//! both. The consolidation prompt differs: it *merges* a batch of same-topic
//! memories into one dense summary, preserving hedging markers.
//!
//! Safety (per #1089):
//! - Every generated record passes [`crate::provenance`] validation before
//!   it is written: non-amplification (never above the weakest source tier)
//!   and hedging guard (never upgrade confidence).
//! - Source memories are soft-deprecated (`deprecated=true`), never deleted.
//! - Budget-capped: at most `max_llm_calls` LLM requests per run.

use crate::consolidation_plan::ConsolidationPlan;
use crate::error::Error;
use crate::extraction::ExtractionConfig;
use crate::memory::types::Memory;
use crate::provenance::ProvenancePolicy;
use serde::{Deserialize, Serialize};

/// Per-request timeout for consolidation LLM calls.
const LLM_TIMEOUT_SECS: u64 = 120;

const CONSOLIDATE_SYSTEM_PROMPT: &str = "\
You consolidate a batch of related memory records from one discussion into a \
single dense record. Rules:\n\
- Merge duplicates and closely related statements into one concise record.\n\
- Preserve uncertainty: if ANY source says maybe/mungkin/perhaps/sepertinya \
etc., the output MUST keep that hedge word. Never turn a hedge into a fact.\n\
- Keep names, dates, numbers, decisions verbatim where possible.\n\
- Do not invent information not present in the sources.\n\
- Output ONLY a JSON array with exactly one object: \
[{\"content\": \"...\", \"type\": \"fact|decision|preference|procedure|context\"}]\n\
- The content must be in the same language as the sources.";

/// Result of one executor run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationExecution {
    /// Batches processed (LLM calls made).
    pub batches_processed: usize,
    /// Records written to the store.
    pub records_written: usize,
    /// Outputs rejected by the provenance policy (#1089).
    pub rejected_by_policy: usize,
    /// Source memories soft-deprecated.
    pub sources_deprecated: usize,
    /// LLM calls skipped because the budget was exhausted.
    pub budget_skipped: usize,
    /// Batches that failed (network/HTTP/parse error) and were skipped so
    /// the rest of the run could proceed. Their sources are left untouched.
    pub batch_errors: Vec<String>,
}

/// Execute a consolidation plan with the shared extraction LLM setup.
///
/// `uteke` provides store access (`recall_room`, `insert`, `deprecate`).
/// The plan is produced by `room_consolidation_plan` (segment-based
/// batching). Each batch is sent as one LLM call; the returned record is
/// validated against the provenance policy, embedded, written, and its
/// sources are soft-deprecated with a reason pointing at the new record.
pub fn execute_plan<U: ConsolidationStore>(
    uteke: &U,
    room_id: &str,
    plan: &ConsolidationPlan,
    config: &ExtractionConfig,
    max_llm_calls: usize,
) -> Result<ConsolidationExecution, Error> {
    // No default timeout on reqwest clients — set one so a slow endpoint
    // cannot hang the consolidation run indefinitely (cf. update_check).
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(LLM_TIMEOUT_SECS))
        .build()
        .map_err(|e| Error::Generic(format!("http client build failed: {e}")))?;
    let endpoint = consolidation_endpoint(config);
    let mut result = ConsolidationExecution {
        batches_processed: 0,
        records_written: 0,
        rejected_by_policy: 0,
        sources_deprecated: 0,
        budget_skipped: 0,
        batch_errors: Vec::new(),
    };

    // Fetch room memories once for provenance checks.
    let memories: Vec<Memory> = uteke.room_memories(room_id)?;
    let by_id = memories
        .iter()
        .map(|m| (m.id.clone(), m.clone()))
        .collect::<std::collections::HashMap<_, _>>();

    // Budget counts LLM requests made, not successful ones: a failed call
    // still consumed an API request against the run's quota.
    let mut llm_calls_made: usize = 0;
    for batch in &plan.batches {
        if llm_calls_made >= max_llm_calls {
            result.budget_skipped += 1;
            continue;
        }
        let sources: Vec<Memory> = batch
            .memory_ids
            .iter()
            .filter_map(|id| by_id.get(id).cloned())
            .collect();
        if sources.len() < 2 {
            // Nothing to consolidate.
            continue;
        }
        let payload = batch.payload.clone();
        llm_calls_made += 1;
        let output_text = match call_llm(&client, &endpoint, config, &payload) {
            Ok(t) => t,
            Err(e) => {
                // A failed batch must not abort the run: earlier batches
                // have already been written, and remaining ones are still
                // worth processing. Record and continue (sources of this
                // batch stay untouched).
                tracing::warn!("consolidation batch failed: {e}");
                result.batch_errors.push(e.to_string());
                continue;
            }
        };
        result.batches_processed += 1;

        let parsed = parse_output(&output_text);
        let Some(fact) = parsed else {
            result.rejected_by_policy += 1;
            continue;
        };

        // Build the candidate record at the agent/derived tier (the floor
        // for consolidated output) — provenance policy then validates.
        let mut record = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            content: fact.content,
            embedding: vec![],
            tags: vec![format!("room:{room_id}"), "consolidated".to_string()],
            metadata: serde_json::json!({
                "consolidated_from": batch.memory_ids,
                "room": room_id,
            }),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: sources
                .first()
                .map(|s| s.namespace.clone())
                .unwrap_or_else(|| "default".to_string()),
            access_count: 0,
            recall_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: None,
            valid_until: None,
            memory_type: sanitize_memory_type(fact.fact_type.as_deref()),
            importance: sources.iter().map(|s| s.importance).fold(0.0_f64, f64::max),
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: Some(format!("room:{room_id}#consolidation")),
            source_type: "derived".to_string(),
            author_type: "agent".to_string(),
        };

        let source_refs: Vec<&Memory> = sources.iter().collect();
        if ProvenancePolicy::validate_output(&source_refs, &record).is_err() {
            result.rejected_by_policy += 1;
            continue;
        }

        // Embed before write so the record is vector-searchable. Embedding
        // failure aborts only this batch (sources stay untouched).
        record.embedding = match uteke.embed_content(&record.content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("consolidation embedding failed: {e}");
                result.batch_errors.push(format!("embedding: {e}"));
                continue;
            }
        };

        // Store failures after this point also abort only this batch. The
        // write/deprecate pair is not transactional across the two calls,
        // so a deprecate failure after a successful insert leaves the new
        // record active alongside its sources — recorded here rather than
        // silently killing the run (duplicate content is recoverable by a
        // later dedup pass; lost batches are not).
        if let Err(e) = uteke.insert_memory(&mut record) {
            tracing::warn!("consolidation write failed: {e}");
            result.batch_errors.push(format!("store write: {e}"));
            continue;
        }
        result.records_written += 1;
        for id in &batch.memory_ids {
            let reason = format!(
                "consolidated into {} (room {room_id} segment batching)",
                record.id
            );
            if let Err(e) = uteke.deprecate_memory(id, &reason) {
                tracing::warn!("consolidation deprecate failed for {id}: {e}");
                result.batch_errors.push(format!("deprecate {id}: {e}"));
            } else {
                result.sources_deprecated += 1;
            }
        }
    }
    Ok(result)
}

/// Store operations the executor needs (implemented by `Uteke`; mocked in tests).
pub trait ConsolidationStore {
    fn room_memories(&self, room_id: &str) -> Result<Vec<Memory>, Error>;
    fn insert_memory(&self, memory: &mut Memory) -> Result<(), Error>;
    fn deprecate_memory(&self, id: &str, reason: &str) -> Result<(), Error>;
    /// Compute the embedding vector for a record before it is written, so
    /// consolidated records stay visible to vector/semantic recall.
    fn embed_content(&self, content: &str) -> Result<Vec<f32>, Error>;
}

/// Whitelist for LLM-provided memory types — anything else falls back to
/// "fact" so arbitrary LLM output can never set an unknown memory_type.
fn sanitize_memory_type(raw: Option<&str>) -> String {
    const ALLOWED: [&str; 5] = ["fact", "decision", "preference", "procedure", "context"];
    match raw {
        Some(t) if ALLOWED.contains(&t) => t.to_string(),
        _ => "fact".to_string(),
    }
}

fn consolidation_endpoint(config: &ExtractionConfig) -> String {
    let base = config.base_url.trim_end_matches('/');
    let path = if config.endpoint_path.starts_with('/') {
        config.endpoint_path.clone()
    } else if config.endpoint_path.is_empty() {
        "/chat/completions".to_string()
    } else {
        format!("/{}", config.endpoint_path)
    };
    format!("{base}{path}")
}

fn call_llm(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    config: &ExtractionConfig,
    payload: &str,
) -> Result<String, Error> {
    let body = serde_json::json!({
        "model": config.model,
        "messages": [
            { "role": "system", "content": CONSOLIDATE_SYSTEM_PROMPT },
            { "role": "user", "content": format!("Sources:\n{payload}") },
        ],
        "temperature": 0.0,
    });
    let resp = client
        .post(endpoint)
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .map_err(|e| Error::generic(format!("Consolidation request failed: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let detail = resp.text().unwrap_or_default();
        return Err(Error::generic(format!(
            "Consolidation endpoint returned HTTP {status}: {detail}"
        )));
    }
    #[derive(serde::Deserialize)]
    struct ChatResponse {
        choices: Vec<Choice>,
    }
    #[derive(serde::Deserialize)]
    struct Choice {
        message: Message,
    }
    #[derive(serde::Deserialize)]
    struct Message {
        content: String,
    }
    let parsed: ChatResponse = resp
        .json()
        .map_err(|e| Error::generic(format!("Failed to parse consolidation response: {e}")))?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| Error::generic("Consolidation response had no choices"))
}

struct ParsedFact {
    content: String,
    fact_type: Option<String>,
}

/// Parse `[{\"content\": \"...\", \"type\": \"...\"}]` — tolerate markdown fences.
fn parse_output(text: &str) -> Option<ParsedFact> {
    let trimmed = text.trim().trim_matches('`');
    let start = trimmed.find('[')?;
    let end = trimmed.rfind(']')?;
    let json = &trimmed[start..=end];
    #[derive(serde::Deserialize)]
    struct RawFact {
        content: String,
        #[serde(rename = "type")]
        fact_type: Option<String>,
    }
    let facts: Vec<RawFact> = serde_json::from_str(json).ok()?;
    let f = facts.into_iter().next()?;
    if f.content.trim().is_empty() {
        return None;
    }
    Some(ParsedFact {
        content: f.content,
        fact_type: f.fact_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_markdown_fenced_output() {
        let parsed =
            parse_output("```json\n[{\"content\": \"Uses bun\", \"type\": \"fact\"}]\n```")
                .unwrap();
        assert_eq!(parsed.content, "Uses bun");
        assert_eq!(parsed.fact_type.as_deref(), Some("fact"));
    }

    #[test]
    fn parses_plain_array() {
        let parsed = parse_output("[{\"content\": \"Rilis mungkin besok\"}]").unwrap();
        assert_eq!(parsed.content, "Rilis mungkin besok");
        assert!(parsed.fact_type.is_none());
    }

    #[test]
    fn rejects_empty_content() {
        assert!(parse_output("[{\"content\": \"  \"}]").is_none());
        assert!(parse_output("no json here").is_none());
    }

    #[test]
    fn endpoint_joins_base_and_path() {
        let mut cfg = ExtractionConfig {
            base_url: "https://api.example.com/v1".into(),
            endpoint_path: "/chat/completions".into(),
            ..Default::default()
        };
        assert_eq!(
            consolidation_endpoint(&cfg),
            "https://api.example.com/v1/chat/completions"
        );
        cfg.endpoint_path = String::new();
        assert_eq!(
            consolidation_endpoint(&cfg),
            "https://api.example.com/v1/chat/completions"
        );
    }
}
