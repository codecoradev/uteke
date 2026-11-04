//! LLM-backed fact extraction — moved from uteke-cli for server reuse.
//!
//! Raw text (chat transcripts, long notes, exported dumps) is noisy: greetings,
//! tool calls, boilerplate. Extraction sends each document to an OpenAI-compatible
//! **chat completions** endpoint and asks the model to distill it into atomic,
//! durable facts. Only those facts are embedded into uteke.
//!
//! Offline-first stays the default. Extraction is strictly opt-in (`--extract`
//! on CLI, `POST /extract` on server). When not requested, uteke never makes
//! a network call here.

use crate::error::Error;

/// Default chat-completions endpoint path (OpenAI standard).
pub const DEFAULT_ENDPOINT_PATH: &str = "/chat/completions";

/// Default base URL (OpenAI). Override for Ollama / vLLM / custom gateways.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Default extraction model — cheap and capable enough for summarization.
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

/// Default ceiling on facts extracted per document.
pub const DEFAULT_MAX_FACTS: usize = 20;

/// Request timeout. Extraction over a long document can be slow.
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// The instruction that turns raw text into atomic facts.
///
/// Two output formats are supported (#1009):
/// - **Scene-segmented** (preferred): array of `{scene, memories: [{content, type, priority}]}`
/// - **Flat array of strings** (legacy): `["fact one", "fact two"]` — graceful fallback.
const SYSTEM_PROMPT: &str = "You extract durable, atomic facts from the user's text for a long-term memory store. \
Rules:\n\
- Output ONLY valid JSON. No prose, no markdown, no code fences.\n\
- Prefer scene-segmented output: a JSON array where each element has \
\"scene\" (short topic label) and \"memories\" (array of objects).\n\
- Each memory object has: \"content\" (self-contained fact), \"type\" \
(one of: fact, decision, preference, procedure, context), and \"priority\" \
(0.0-1.0, where 1.0 = critical/long-lived, 0.3 = minor/trivial).\n\
- If the text is single-topic, use a single scene.\n\
- Backward compatible: a flat JSON array of strings is also accepted.\n\
- Each fact must be self-contained — resolve pronouns.\n\
- Prefer specific facts (names, dates, numbers, decisions) over vague summaries.\n\
- Drop greetings, filler, tool output, navigation, and anything ephemeral.\n\
- If the text contains nothing worth remembering, output an empty array: []\n\
\n\
Example scene-segmented output:\n\
[{\"scene\":\"auth refactor\",\"memories\":[{\"content\":\"Decided to use OAuth 2.1 with PKCE\",\"type\":\"decision\",\"priority\":0.9},{\"content\":\"Auth middleware runs before route handler\",\"type\":\"fact\",\"priority\":0.6}]}]";

/// Configuration for the extraction pipeline.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct ExtractionConfig {
    /// Extraction mode: "offline" (rule-based, default), "llm" (API-based), or "" (unconfigured).
    ///
    /// When "offline" or "", extraction uses pattern matching with no network calls.
    /// When "llm", extraction sends text to an OpenAI-compatible chat completions endpoint.
    pub mode: String,
    /// Chat-completions model (e.g. "gpt-4o-mini"). Only used when mode = "llm".
    pub model: String,
    /// API key for the extraction endpoint. Only used when mode = "llm".
    pub api_key: String,
    /// Base URL of the OpenAI-compatible endpoint. Only used when mode = "llm".
    pub base_url: String,
    /// Endpoint path appended to base_url. Only used when mode = "llm".
    pub endpoint_path: String,
    /// Maximum facts to keep per document. 0 = default.
    pub max_facts: usize,
}

/// A single extracted fact with scene segmentation and priority metadata (#1009).
///
/// When the LLM returns scene-segmented output, each fact carries:
/// - `scene`: topic label (also injected as a `scene:xxx` tag)
/// - `fact_type`: the memory type (fact, decision, preference, procedure, context)
/// - `priority`: importance score 0.0–1.0 (mapped to uteke's `importance` field)
///
/// When the LLM returns a flat array of strings, `scene` is `None`,
/// `fact_type` is `None`, and `priority` is `None` (caller uses defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedFact {
    /// The fact content — self-contained, no pronouns.
    pub content: String,
    /// Scene/topic label if the LLM segmented the output.
    pub scene: Option<String>,
    /// Memory type if the LLM provided one.
    pub fact_type: Option<String>,
    /// Priority/importance score 0.0–1.0 if the LLM provided one.
    pub priority: Option<f64>,
}

impl ExtractedFact {
    /// Create a flat fact with no metadata (legacy backward compat).
    pub fn flat(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            scene: None,
            fact_type: None,
            priority: None,
        }
    }
}

impl ExtractionConfig {
    /// Apply the `UTEKE_EXTRACTION_*` variables over the file values (env wins,
    /// an empty variable is ignored). The CLI and `uteke-serve` both call this,
    /// so a key set only in the environment reaches every extraction path.
    pub fn with_env_overrides(self) -> Self {
        self.with_overrides(|k| std::env::var(k).ok())
    }

    fn with_overrides(mut self, var: impl Fn(&str) -> Option<String>) -> Self {
        let set = |k: &str| var(k).filter(|v| !v.is_empty());
        if let Some(v) = set("UTEKE_EXTRACTION_MODEL") {
            self.model = v;
        }
        if let Some(v) = set("UTEKE_EXTRACTION_API_KEY") {
            self.api_key = v;
        }
        if let Some(v) = set("UTEKE_EXTRACTION_BASE_URL") {
            self.base_url = v;
        }
        if let Some(v) = set("UTEKE_EXTRACTION_ENDPOINT_PATH") {
            self.endpoint_path = v;
        }
        if let Some(v) = set("UTEKE_EXTRACTION_MAX_FACTS") {
            match v.parse::<usize>() {
                Ok(n) => self.max_facts = n,
                Err(_) => tracing::warn!(
                    "Invalid UTEKE_EXTRACTION_MAX_FACTS='{v}', ignoring (expected integer)"
                ),
            }
        }
        self
    }
}

#[cfg(test)]
mod env_override_tests {
    use super::ExtractionConfig;

    #[test]
    fn the_environment_wins_over_the_file_and_empty_or_invalid_values_are_ignored() {
        let file = ExtractionConfig {
            mode: "llm".into(),
            model: "file-model".into(),
            api_key: "file-key".into(),
            max_facts: 7,
            ..Default::default()
        };
        let env = |k: &str| match k {
            "UTEKE_EXTRACTION_API_KEY" => Some("env-key".to_string()),
            "UTEKE_EXTRACTION_MODEL" => Some(String::new()),
            "UTEKE_EXTRACTION_BASE_URL" => Some("https://env.test".to_string()),
            "UTEKE_EXTRACTION_MAX_FACTS" => Some("lots".to_string()),
            _ => None,
        };
        let c = file.with_overrides(env);
        assert_eq!(c.api_key, "env-key");
        assert_eq!(c.model, "file-model", "an empty variable is ignored");
        assert_eq!(c.base_url, "https://env.test");
        assert_eq!(c.max_facts, 7, "an invalid number is ignored");
        assert_eq!(c.mode, "llm", "mode has no variable");
    }
}

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
    /// Build a new extractor from a config.
    ///
    /// CLI flag / HTTP body overrides are resolved *before* calling this,
    /// so `config` already has the final values.
    pub fn new(config: &ExtractionConfig) -> Result<Self, Error> {
        if config.api_key.is_empty() {
            return Err(Error::Validation(
                "Extraction requires an API key (set UTEKE_EXTRACTION_API_KEY, \
                 or [extraction] api_key in uteke.toml)"
                    .into(),
            ));
        }
        let base = if config.base_url.is_empty() {
            DEFAULT_BASE_URL
        } else {
            &config.base_url
        };
        validate_base_url(base)?;

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| Error::generic(format!("Failed to build HTTP client: {e}")))?;

        let endpoint = normalize_endpoint_path(&config.endpoint_path);
        let model: String = if config.model.is_empty() {
            DEFAULT_MODEL.to_owned()
        } else {
            config.model.clone()
        };
        let max_facts = if config.max_facts == 0 {
            DEFAULT_MAX_FACTS
        } else {
            config.max_facts
        };

        Ok(Self {
            client,
            api_key: config.api_key.clone(),
            base_url: base.to_string(),
            endpoint_path: endpoint,
            model,
            max_facts,
        })
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}{}", self.endpoint_path)
    }

    /// Extract atomic facts from a single document.
    ///
    /// Returns the parsed list of facts (truncated to `max_facts`).
    /// Each fact may carry scene, type, and priority metadata (#1009).
    /// An empty vec means the model found nothing worth keeping.
    pub fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, Error> {
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

/// Whether an extraction error says the provider refused the input as too
/// large (#1664): HTTP 400 or 413, or another 4xx (not 408 or 429) whose body
/// names the context length. Such an input fails the same way on every retry,
/// so a caller fails that input alone instead of treating the provider as down.
/// The status decides first: a 429, a 5xx or an error with no HTTP status (a
/// network failure) is never "too large", whatever its body says.
pub fn is_input_too_large(err: &Error) -> bool {
    let msg = err.to_string();
    let Some(status) = msg.split_once("returned HTTP ").and_then(|(_, rest)| {
        rest.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u16>()
            .ok()
    }) else {
        return false;
    };
    match status {
        400 | 413 => true,
        408 | 429 => false,
        400..=499 => {
            let lower = msg.to_ascii_lowercase();
            [
                "context length",
                "context_length",
                "too large",
                "too long",
                "maximum context",
            ]
            .iter()
            .any(|m| lower.contains(m))
        }
        _ => false,
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

/// Parse the model's reply into a clean list of extracted facts.
///
/// Three formats are handled (#1009):
/// 1. **Scene-segmented**: `[{scene, memories: [{content, type, priority}]}]`
/// 2. **Flat object array**: `[{content, type, priority}]` or `[{fact: "..."}]`
/// 3. **Flat string array**: `["fact one", "fact two"]` (legacy)
/// 4. **Line-by-line fallback** when JSON parsing fails entirely.
fn parse_facts(content: &str) -> Vec<ExtractedFact> {
    let cleaned = strip_code_fences(content.trim());

    if let Some(arr) = extract_json_array(cleaned) {
        if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(arr) {
            // Try scene-segmented format first: [{scene, memories: [...]}]
            if let Some(facts) = try_parse_scene_segmented(&values) {
                return dedup_facts(facts);
            }

            // Fall back to flat array parsing (strings or objects)
            let facts: Vec<ExtractedFact> = values
                .into_iter()
                .filter_map(|v| match v {
                    serde_json::Value::String(s) => Some(ExtractedFact::flat(s)),
                    serde_json::Value::Object(map) => parse_fact_object(&map),
                    _ => None,
                })
                .map(|f| ExtractedFact {
                    content: f.content.trim().to_string(),
                    ..f
                })
                .filter(|f| !f.content.is_empty())
                .collect();
            return dedup_facts(facts);
        }
    }

    // Fallback: treat each non-empty line as a fact, stripping list markers.
    let facts: Vec<ExtractedFact> = cleaned
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•']).trim())
        .map(strip_leading_number)
        .filter(|l| l.len() > 2)
        .map(ExtractedFact::flat)
        .collect();
    dedup_facts(facts)
}

/// Try to parse scene-segmented output: `[{scene, memories: [{content, type, priority}]}]`.
///
/// Returns `Some(vec)` only if at least one element matches the scene structure.
/// If no element has a `memories` array, returns `None` (caller tries flat parsing).
fn try_parse_scene_segmented(values: &[serde_json::Value]) -> Option<Vec<ExtractedFact>> {
    let mut all_facts = Vec::new();
    let mut found_scene = false;

    for val in values {
        let obj = val.as_object()?;
        // A scene element must have a "memories" array.
        let memories = obj.get("memories")?.as_array()?;
        found_scene = true;

        let scene = obj
            .get("scene")
            .and_then(|s| s.as_str())
            .map(|s| s.trim().to_string());

        for mem in memories {
            if let Some(mem_obj) = mem.as_object() {
                if let Some(fact) = parse_fact_object(mem_obj) {
                    all_facts.push(ExtractedFact {
                        scene: scene.clone(),
                        ..fact
                    });
                }
            }
        }
    }

    if found_scene { Some(all_facts) } else { None }
}

/// Parse a single fact object: `{content, type, priority}` or `{fact: "..."}`.
fn parse_fact_object(map: &serde_json::Map<String, serde_json::Value>) -> Option<ExtractedFact> {
    let content = map
        .get("content")
        .or_else(|| map.get("fact"))
        .or_else(|| map.get("text"))
        .and_then(|v| v.as_str())?
        .trim()
        .to_string();
    if content.is_empty() {
        return None;
    }

    let fact_type = map
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_lowercase());

    let priority = map
        .get("priority")
        .and_then(|v| v.as_f64())
        .filter(|&p| (0.0..=1.0).contains(&p));

    let scene = map
        .get("scene")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string());

    Some(ExtractedFact {
        content,
        scene,
        fact_type,
        priority,
    })
}

/// Remove a leading backtick-fence wrapper (```` ``` ```` or ```` ```lang ````) if present.
fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```") {
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

/// Strip a leading \"1. \" / \"2) \" enumeration marker.
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
fn dedup_facts(facts: Vec<ExtractedFact>) -> Vec<ExtractedFact> {
    let mut seen: Vec<String> = Vec::with_capacity(facts.len());
    let mut result = Vec::with_capacity(facts.len());
    for f in facts {
        let key = f.content.trim().to_lowercase();
        if !key.is_empty() && !seen.iter().any(|x| x == &key) {
            seen.push(key);
            result.push(f);
        }
    }
    result
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

    fn default_config() -> ExtractionConfig {
        ExtractionConfig {
            mode: "llm".into(),
            api_key: "test-key".into(),
            model: String::new(),
            base_url: String::new(),
            endpoint_path: String::new(),
            max_facts: 0,
        }
    }

    #[test]
    fn rejects_empty_api_key() {
        let cfg = ExtractionConfig::default();
        assert!(
            Extractor::new(&cfg)
                .unwrap_err()
                .to_string()
                .contains("API key")
        );
    }

    #[test]
    fn endpoint_defaults_and_normalizes() {
        let e = Extractor::new(&default_config()).unwrap();
        assert_eq!(e.endpoint(), "https://api.openai.com/v1/chat/completions");
        assert_eq!(e.model, DEFAULT_MODEL);
        assert_eq!(e.max_facts, DEFAULT_MAX_FACTS);
    }

    #[test]
    fn endpoint_path_without_slash_normalized() {
        let cfg = ExtractionConfig {
            mode: "llm".into(),
            api_key: "k".into(),
            model: "m".into(),
            base_url: "https://gw.example.com/v1".into(),
            endpoint_path: "v1/chat".into(),
            max_facts: 5,
        };
        let e = Extractor::new(&cfg).unwrap();
        assert_eq!(e.endpoint(), "https://gw.example.com/v1/v1/chat");
        assert_eq!(e.max_facts, 5);
    }

    #[test]
    fn parses_clean_json_array() {
        let facts = parse_facts(r#"["User prefers Indonesian", "Bootcamp has 8 sessions"]"#);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].content, "User prefers Indonesian");
        assert_eq!(facts[1].content, "Bootcamp has 8 sessions");
        assert!(facts[0].scene.is_none());
        assert!(facts[0].fact_type.is_none());
        assert!(facts[0].priority.is_none());
    }

    #[test]
    fn parses_json_array_inside_code_fence() {
        let raw = "```json\n[\"Fact A\", \"Fact B\"]\n```";
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].content, "Fact A");
        assert_eq!(facts[1].content, "Fact B");
    }

    #[test]
    fn parses_array_with_preamble() {
        let raw = "Here are the facts:\n[\"Only this matters\"]";
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].content, "Only this matters");
    }

    #[test]
    fn parses_object_array_with_fact_key() {
        let raw = r#"[{"fact": "Deadline is July 31"}, {"fact": "Promo is 65 percent"}]"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].content, "Deadline is July 31");
        assert_eq!(facts[1].content, "Promo is 65 percent");
    }

    #[test]
    fn falls_back_to_line_parsing() {
        let raw = "- First fact\n- Second fact\n1. Third fact";
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 3);
        assert_eq!(facts[0].content, "First fact");
        assert_eq!(facts[2].content, "Third fact");
    }

    #[test]
    fn empty_array_yields_no_facts() {
        assert!(parse_facts("[]").is_empty());
    }

    #[test]
    fn dedups_repeated_facts() {
        let raw = r#"["Same", "Same", "Different"]"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].content, "Same");
        assert_eq!(facts[1].content, "Different");
    }

    // --- Scene-segmented parsing tests (#1009) ---

    #[test]
    fn parses_scene_segmented_output() {
        let raw = r#"[
          {"scene": "auth", "memories": [
            {"content": "Decided to use OAuth 2.1", "type": "decision", "priority": 0.9},
            {"content": "Auth middleware runs first", "type": "fact", "priority": 0.6}
          ]},
          {"scene": "database", "memories": [
            {"content": "Migration adds indexes", "type": "fact", "priority": 0.7}
          ]}
        ]"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 3);
        assert_eq!(facts[0].content, "Decided to use OAuth 2.1");
        assert_eq!(facts[0].scene.as_deref(), Some("auth"));
        assert_eq!(facts[0].fact_type.as_deref(), Some("decision"));
        assert_eq!(facts[0].priority, Some(0.9));
        assert_eq!(facts[1].scene.as_deref(), Some("auth"));
        assert_eq!(facts[2].scene.as_deref(), Some("database"));
        assert_eq!(facts[2].content, "Migration adds indexes");
    }

    #[test]
    fn parses_flat_object_array_with_type_priority() {
        let raw = r#"[{"content": "Important decision", "type": "decision", "priority": 0.85}]"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].content, "Important decision");
        assert_eq!(facts[0].fact_type.as_deref(), Some("decision"));
        assert_eq!(facts[0].priority, Some(0.85));
        assert!(facts[0].scene.is_none());
    }

    #[test]
    fn scene_segmented_with_string_memories_falls_back() {
        // If "memories" contains strings not objects, they should be skipped gracefully.
        let raw = r#"[{"scene": "test", "memories": ["plain string fact"]}]"#;
        let facts = parse_facts(raw);
        // String elements inside memories are not objects → no facts extracted
        // But the array was detected as scene-segmented, so flat parsing is NOT attempted.
        assert!(facts.is_empty());
    }

    #[test]
    fn dedup_is_case_insensitive() {
        let raw = r#"["Same Fact", "same fact", "Different"]"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].content, "Same Fact");
        assert_eq!(facts[1].content, "Different");
    }

    #[test]
    fn priority_out_of_range_ignored() {
        let raw = r#"[{"content": "Test", "priority": 1.5}]"#;
        let facts = parse_facts(raw);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].priority.is_none()); // 1.5 is out of 0.0-1.0 range
    }
}
