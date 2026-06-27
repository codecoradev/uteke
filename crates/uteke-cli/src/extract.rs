//! LLM-backed fact extraction for `uteke import --extract`.
//!
//! Raw text (chat transcripts, long notes, exported dumps) is noisy: greetings,
//! tool calls, boilerplate. Importing it verbatim pollutes recall. This module
//! sends each input document to an OpenAI-compatible **chat completions**
//! endpoint and asks the model to distill it into a list of atomic, durable
//! facts. Only those facts are embedded into uteke.
//!
//! ## Offline-first stays the default
//!
//! Extraction is strictly opt-in (`--extract`). When the flag is absent uteke
//! never makes a network call here — the existing JSONL/markdown/text/CSV paths
//! run unchanged. This mirrors the embedding backend design: local ONNX by
//! default, remote OpenAI-compatible API only when the user configures it.
//!
//! ## Auth & endpoint
//!
//! Same convention as the embedding backend (`embed/openai.rs`):
//! `Authorization: Bearer <key>`, configurable `base_url` + `endpoint_path`
//! (default `/chat/completions`). Works with OpenAI, Ollama, vLLM, or any
//! OpenAI-compatible gateway.

use uteke_core::Error;

/// Default chat-completions endpoint path (OpenAI standard).
pub const DEFAULT_ENDPOINT_PATH: &str = "/chat/completions";

/// Default base URL (OpenAI). Override for Ollama / vLLM / custom gateways.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Default extraction model — cheap and capable enough for summarization.
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

/// Default ceiling on facts extracted per document. Keeps a runaway model from
/// flooding the store from one huge transcript.
pub const DEFAULT_MAX_FACTS: usize = 20;

/// Request timeout. Extraction over a long document can be slow, so this is
/// more generous than the embedding client's 30s.
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// The instruction that turns raw text into atomic facts. Kept terse and
/// strict so models return clean, parseable output.
const SYSTEM_PROMPT: &str = "You extract durable, atomic facts from the user's text for a long-term memory store. \
Rules:\n\
- Output ONLY a JSON array of strings. No prose, no markdown, no code fences.\n\
- Each string is ONE self-contained fact, decision, preference, or piece of context worth remembering later.\n\
- Drop greetings, filler, tool output, navigation, and anything ephemeral.\n\
- Resolve pronouns and make each fact understandable on its own.\n\
- Prefer specific facts (names, dates, numbers, decisions) over vague summaries.\n\
- If the text contains nothing worth remembering, output an empty array: []";

/// An OpenAI-compatible chat-completions client used for fact extraction.
#[derive(Debug)]
pub struct Extractor {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    endpoint_path: String,
    model: String,
    max_facts: usize,
}

impl Extractor {
    /// Build a new extractor.
    ///
    /// `api_key` is required (extraction always hits a remote model). Endpoint
    /// resolution mirrors the embedding backend: empty `endpoint_path` falls
    /// back to [`DEFAULT_ENDPOINT_PATH`]; a path without a leading slash is
    /// normalized.
    pub fn new(
        api_key: &str,
        model: &str,
        base_url: &str,
        endpoint_path: &str,
        max_facts: usize,
    ) -> Result<Self, Error> {
        if api_key.is_empty() {
            return Err(Error::Validation(
                "Extraction requires an API key (set UTEKE_EXTRACTION_API_KEY, \
                 or [extraction] api_key in uteke.toml, or pass --extract-api-key)"
                    .into(),
            ));
        }
        let base = if base_url.is_empty() {
            DEFAULT_BASE_URL
        } else {
            base_url
        };
        validate_base_url(base)?;

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| Error::generic(format!("Failed to build HTTP client: {e}")))?;

        let endpoint = normalize_endpoint_path(endpoint_path);
        let model = if model.is_empty() {
            DEFAULT_MODEL
        } else {
            model
        };
        let max_facts = if max_facts == 0 {
            DEFAULT_MAX_FACTS
        } else {
            max_facts
        };

        Ok(Self {
            client,
            api_key: api_key.to_string(),
            base_url: base.to_string(),
            endpoint_path: endpoint,
            model: model.to_string(),
            max_facts,
        })
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}{}", self.endpoint_path)
    }

    /// Extract atomic facts from a single document.
    ///
    /// Returns the parsed list of facts (already truncated to `max_facts`).
    /// An empty vec means the model found nothing worth keeping.
    pub fn extract(&self, text: &str) -> Result<Vec<String>, Error> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let user_prompt = format!(
            "Extract up to {} facts from the following text:\n\n{}",
            self.max_facts, text
        );

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user", "content": user_prompt },
            ],
            "temperature": 0.0,
        });

        let resp = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| Error::generic(format!("Extraction request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().unwrap_or_default();
            return Err(Error::generic(format!(
                "Extraction endpoint returned HTTP {status}: {detail}"
            )));
        }

        let parsed: ChatResponse = resp
            .json()
            .map_err(|e| Error::generic(format!("Failed to parse extraction response: {e}")))?;

        let content = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| Error::generic("Extraction response had no choices"))?;

        let mut facts = parse_facts(&content);
        facts.truncate(self.max_facts);
        Ok(facts)
    }
}

/// Normalize an endpoint path: empty -> default, ensure leading slash.
fn normalize_endpoint_path(path: &str) -> String {
    if path.is_empty() {
        DEFAULT_ENDPOINT_PATH.to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// Validate that a base URL has an http(s) scheme and parses.
///
/// Mirrors `uteke_core::embed::validate_base_url`, which is `pub(crate)` and
/// therefore not reachable from this crate.
fn validate_base_url(base_url: &str) -> Result<(), Error> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(Error::Validation("base_url must not be empty".into()));
    }
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err(Error::Validation(format!(
            "base_url must start with 'http://' or 'https://' (got '{trimmed}')"
        )));
    }
    if reqwest::Url::parse(trimmed).is_err() {
        return Err(Error::Validation(format!(
            "base_url is not a valid URL: '{trimmed}'"
        )));
    }
    Ok(())
}

/// Parse the model's reply into a clean list of facts.
///
/// Models are asked for a bare JSON array of strings, but real-world output
/// varies: code fences, a stray sentence before the array, or a fallback to
/// line-delimited text. This handles all three defensively so a slightly
/// chatty model doesn't abort the whole import.
fn parse_facts(content: &str) -> Vec<String> {
    let cleaned = strip_code_fences(content.trim());

    // Preferred path: a JSON array of strings somewhere in the reply.
    if let Some(arr) = extract_json_array(cleaned) {
        if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(arr) {
            let facts: Vec<String> = values
                .into_iter()
                .filter_map(|v| match v {
                    serde_json::Value::String(s) => Some(s),
                    // Tolerate models that return objects like {"fact": "..."}.
                    serde_json::Value::Object(map) => map
                        .get("fact")
                        .or_else(|| map.get("text"))
                        .or_else(|| map.get("content"))
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string()),
                    _ => None,
                })
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return dedup(facts);
        }
    }

    // Fallback: treat each non-empty line as a fact, stripping list markers.
    let facts: Vec<String> = cleaned
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•']).trim())
        .map(strip_leading_number)
        .filter(|l| l.len() > 2)
        .map(|l| l.to_string())
        .collect();
    dedup(facts)
}

/// Remove a leading ```...``` / ``` fence wrapper if present.
fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```") {
        // Drop optional language tag on the first line.
        let after_lang = rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or("");
        return after_lang
            .trim_end()
            .strip_suffix("```")
            .unwrap_or(after_lang)
            .trim();
    }
    s
}

/// Find the outermost `[...]` JSON array substring, if any.
fn extract_json_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

/// Strip a leading "1. " / "2) " enumeration marker.
fn strip_leading_number(s: &str) -> &str {
    let trimmed = s.trim_start();
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return trimmed;
    }
    let rest = &trimmed[digits.len()..];
    if let Some(after) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')')) {
        after.trim_start()
    } else {
        trimmed
    }
}

/// Drop duplicate and empty facts while preserving order.
fn dedup(facts: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(facts.len());
    for f in facts {
        let f = f.trim().to_string();
        if !f.is_empty() && !seen.iter().any(|x| x == &f) {
            seen.push(f);
        }
    }
    seen
}

#[derive(serde::Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(serde::Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(serde::Deserialize)]
struct ChatMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_api_key() {
        let err =
            Extractor::new("", DEFAULT_MODEL, DEFAULT_BASE_URL, "", DEFAULT_MAX_FACTS).unwrap_err();
        assert!(format!("{err}").contains("API key"));
    }

    #[test]
    fn endpoint_defaults_and_normalizes() {
        let e = Extractor::new("k", "", "https://api.openai.com/v1/", "", 0).unwrap();
        assert_eq!(e.endpoint(), "https://api.openai.com/v1/chat/completions");
        assert_eq!(e.model, DEFAULT_MODEL);
        assert_eq!(e.max_facts, DEFAULT_MAX_FACTS);
    }

    #[test]
    fn endpoint_path_without_slash_normalized() {
        let e = Extractor::new("k", "m", "https://gw.example.com/v1", "v1/chat", 5).unwrap();
        assert_eq!(e.endpoint(), "https://gw.example.com/v1/v1/chat");
        assert_eq!(e.max_facts, 5);
    }

    #[test]
    fn parses_clean_json_array() {
        let facts = parse_facts(r#"["User prefers Indonesian", "Bootcamp has 8 sessions"]"#);
        assert_eq!(
            facts,
            vec!["User prefers Indonesian", "Bootcamp has 8 sessions"]
        );
    }

    #[test]
    fn parses_json_array_inside_code_fence() {
        let raw = "```json\n[\"Fact A\", \"Fact B\"]\n```";
        assert_eq!(parse_facts(raw), vec!["Fact A", "Fact B"]);
    }

    #[test]
    fn parses_array_with_preamble() {
        let raw = "Here are the facts:\n[\"Only this matters\"]";
        assert_eq!(parse_facts(raw), vec!["Only this matters"]);
    }

    #[test]
    fn parses_object_array_with_fact_key() {
        let raw = r#"[{"fact": "Deadline is July 31"}, {"fact": "Promo is 65 percent"}]"#;
        assert_eq!(
            parse_facts(raw),
            vec!["Deadline is July 31", "Promo is 65 percent"]
        );
    }

    #[test]
    fn falls_back_to_line_parsing() {
        let raw = "- First fact\n- Second fact\n1. Third fact";
        assert_eq!(
            parse_facts(raw),
            vec!["First fact", "Second fact", "Third fact"]
        );
    }

    #[test]
    fn empty_array_yields_no_facts() {
        assert!(parse_facts("[]").is_empty());
    }

    #[test]
    fn dedups_repeated_facts() {
        let raw = r#"["Same", "Same", "Different"]"#;
        assert_eq!(parse_facts(raw), vec!["Same", "Different"]);
    }
}
