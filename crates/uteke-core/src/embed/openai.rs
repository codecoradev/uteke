//! OpenAI embedding backend.
//!
//! POSTs to `{base_url}/embeddings` with `{ model, input }` JSON and parses
//! `data[0].embedding`. No batching — single-text per call, same surface as
//! the ONNX embedder.
//!
//! Auth: `Authorization: Bearer <api_key>` (env `UTEKE_EMBEDDING_API_KEY` or
//! `OPENAI_API_KEY`, or `[embedding] api_key` in uteke.toml).
//!
//! Dimensions are model-specific (1536 for text-embedding-3-small,
//! 3072 for text-embedding-3-large). The caller supplies dims so we don't
//! burn a probe request at startup.

use crate::embed::Embedder;
use crate::Error;

/// Default OpenAI API endpoint.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Default model — cheapest, good enough for semantic recall.
pub const DEFAULT_MODEL: &str = "text-embedding-3-small";

/// Default dimensions for [`DEFAULT_MODEL`].
pub const DEFAULT_DIMS: usize = 1536;

/// Default endpoint path (OpenAI standard). Override via `endpoint_path`
/// config or `UTEKE_EMBEDDING_ENDPOINT_PATH` env var for non-standard
/// OpenAI-compatible APIs (#473).
pub const DEFAULT_ENDPOINT_PATH: &str = "/embeddings";

/// OpenAI max sequence length in tokens (clamped by the API).
pub const MAX_SEQ_LEN: usize = 8191;

/// OpenAI embedding backend.
#[derive(Debug)]
pub struct OpenAiEmbedder {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    endpoint_path: String,
    model: String,
    dims: usize,
}

impl OpenAiEmbedder {
    /// Construct a new OpenAI embedder.
    ///
    /// `api_key` is required; resolution from env/config is the caller's job
    /// (see `lib.rs::ensure_embedder`).
    ///
    /// `endpoint_path` is the API path appended to `base_url` (e.g. `/embeddings`
    /// or `/embed`). Pass empty string to use [`DEFAULT_ENDPOINT_PATH`].
    pub fn new(
        api_key: &str,
        model: &str,
        base_url: &str,
        endpoint_path: &str,
        dims: usize,
    ) -> Result<Self, Error> {
        if api_key.is_empty() {
            return Err(Error::Validation(
                "OpenAI embedder requires an API key (set UTEKE_EMBEDDING_API_KEY or OPENAI_API_KEY)".into(),
            ));
        }
        crate::embed::validate_base_url(base_url)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::generic(format!("Failed to build HTTP client: {e}")))?;
        let endpoint = if endpoint_path.is_empty() {
            DEFAULT_ENDPOINT_PATH.to_string()
        } else {
            // Normalize: ensure leading slash so base_url + path always
            // produces a valid URL (CodeCora review #473).
            let path = endpoint_path;
            if path.starts_with('/') {
                path.to_string()
            } else {
                format!("/{path}")
            }
        };
        Ok(Self {
            client,
            api_key: api_key.to_string(),
            base_url: base_url.to_string(),
            endpoint_path: endpoint,
            model: model.to_string(),
            dims,
        })
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}{}", self.endpoint_path)
    }
}

impl OpenAiEmbedder {
    /// Single embedding request for pre-truncated `input`. The retry/shrink
    /// loop lives in [`Embedder::embed`].
    fn embed_once(&self, input: &str) -> Result<Vec<f32>, Error> {
        // Include `dimensions` when explicitly configured. This keeps the
        // API response size in sync with the configured index dims
        // (CodeCora finding #146) for models that support the field.
        // text-embedding-3-* support it; older models ignore unknown fields
        // so sending it unconditionally is safe.
        let body = if self.dims > 0 {
            serde_json::json!({
                "model": self.model,
                "input": input,
                "dimensions": self.dims,
            })
        } else {
            serde_json::json!({
                "model": self.model,
                "input": input,
            })
        };

        let resp = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| Error::generic(format!("OpenAI embeddings request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            return Err(Error::generic(format!(
                "OpenAI embeddings returned HTTP {status}: {text}"
            )));
        }

        let parsed: EmbeddingsResponse = resp
            .json()
            .map_err(|e| Error::generic(format!("Failed to parse OpenAI response: {e}")))?;

        parsed
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| Error::generic("OpenAI response had no embedding data"))
    }
}

/// Conservative char budget for a given token limit. Code is denser than
/// prose (short identifiers, punctuation), so we use ~3.5 chars/token and
/// leave headroom below the hard limit.
fn char_budget_for(max_tokens: usize) -> usize {
    (max_tokens.saturating_mul(7) / 2).max(512)
}

/// Truncate on a UTF-8 char boundary to at most `max` chars.
fn truncate_chars(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl Embedder for OpenAiEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        // OpenAI rejects empty input with 400; send a single space so the
        // call is always valid (matches ONNX backend's non-empty contract).
        let base = if text.is_empty() { " " } else { text };

        // Guard against the model's 8192-token context limit. We have no
        // tokenizer here, so cap by chars with a conservative code-safe ratio
        // (~3.5 chars/token; code is denser than prose) and, if the API still
        // reports a context-length 400, halve and retry a few times. Long AST
        // chunks (whole functions/files) otherwise fail outright — local
        // backends tolerated them, OpenAI does not.
        let mut budget = char_budget_for(MAX_SEQ_LEN);
        let mut input = truncate_chars(base, budget);
        loop {
            match self.embed_once(input) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let msg = e.to_string();
                    let is_ctx_len = msg.contains("maximum context length")
                        || msg.contains("'input'");
                    if is_ctx_len && budget > 512 {
                        budget /= 2;
                        input = truncate_chars(base, budget);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }

    fn dims(&self) -> usize {
        self.dims
    }

    fn max_seq_len(&self) -> usize {
        MAX_SEQ_LEN
    }

    fn name(&self) -> &str {
        "openai"
    }
}

#[derive(serde::Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingData>,
}

#[derive(serde::Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_respects_utf8_boundary() {
        // Multi-byte chars must not be split mid-sequence.
        let s = "aéééé"; // 'a' + 4x 2-byte chars
        let t = truncate_chars(s, 4);
        assert!(s.starts_with(t));
        assert!(t.len() <= 4);
        // Valid UTF-8 by construction (would panic otherwise).
        assert!(t.chars().count() >= 1);
    }

    #[test]
    fn truncate_chars_noop_when_short() {
        assert_eq!(truncate_chars("hello", 100), "hello");
    }

    #[test]
    fn char_budget_below_hard_limit_but_generous() {
        let b = char_budget_for(MAX_SEQ_LEN);
        assert!(b >= 512);
        // ~3.5 chars/token → well above the token count, well within reason.
        assert!(b > MAX_SEQ_LEN);
    }

    #[test]
    fn rejects_empty_api_key() {
        let err = OpenAiEmbedder::new(
            "",
            DEFAULT_MODEL,
            DEFAULT_BASE_URL,
            DEFAULT_ENDPOINT_PATH,
            DEFAULT_DIMS,
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("API key"), "got: {msg}");
    }

    #[test]
    fn endpoint_normalizes_trailing_slash() {
        let e = OpenAiEmbedder::new(
            "k",
            DEFAULT_MODEL,
            "https://api.openai.com/v1/",
            DEFAULT_ENDPOINT_PATH,
            DEFAULT_DIMS,
        )
        .unwrap();
        assert_eq!(e.endpoint(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn defaults_match_docs() {
        assert_eq!(DEFAULT_MODEL, "text-embedding-3-small");
        assert_eq!(DEFAULT_DIMS, 1536);
        assert_eq!(DEFAULT_BASE_URL, "https://api.openai.com/v1");
        assert_eq!(DEFAULT_ENDPOINT_PATH, "/embeddings");
        assert_eq!(MAX_SEQ_LEN, 8191);
    }

    #[test]
    fn embedder_name_and_dims() {
        let e = OpenAiEmbedder::new(
            "k",
            DEFAULT_MODEL,
            DEFAULT_BASE_URL,
            DEFAULT_ENDPOINT_PATH,
            3072,
        )
        .unwrap();
        assert_eq!(e.name(), "openai");
        assert_eq!(e.dims(), 3072);
        assert_eq!(e.max_seq_len(), MAX_SEQ_LEN);
    }

    #[test]
    fn parses_successful_response() {
        let raw = r#"{
            "object": "list",
            "data": [
                { "object": "embedding", "index": 0, "embedding": [0.1, 0.2, 0.3] }
            ],
            "model": "text-embedding-3-small",
            "usage": { "prompt_tokens": 1, "total_tokens": 1 }
        }"#;
        let parsed: EmbeddingsResponse = serde_json::from_str(raw).unwrap();
        let emb = parsed.data.into_iter().next().unwrap().embedding;
        assert_eq!(emb, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn rejects_invalid_base_url() {
        // Schemeless URL — CodeCora #155.
        let err = OpenAiEmbedder::new(
            "k",
            DEFAULT_MODEL,
            "api.openai.com/v1",
            DEFAULT_ENDPOINT_PATH,
            DEFAULT_DIMS,
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("http"), "got: {msg}");

        // Empty string.
        let err = OpenAiEmbedder::new("k", DEFAULT_MODEL, "", DEFAULT_ENDPOINT_PATH, DEFAULT_DIMS)
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("empty") || msg.contains("http"), "got: {msg}");

        // Unparseable.
        let err = OpenAiEmbedder::new(
            "k",
            DEFAULT_MODEL,
            "https://",
            DEFAULT_ENDPOINT_PATH,
            DEFAULT_DIMS,
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("valid URL") || msg.contains("http"),
            "got: {msg}"
        );
    }

    #[test]
    fn empty_endpoint_path_uses_default() {
        let e =
            OpenAiEmbedder::new("k", DEFAULT_MODEL, DEFAULT_BASE_URL, "", DEFAULT_DIMS).unwrap();
        assert_eq!(e.endpoint(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn custom_endpoint_path() {
        let e = OpenAiEmbedder::new(
            "k",
            DEFAULT_MODEL,
            "https://codecora-embed.example.com/v1",
            "/embed",
            DEFAULT_DIMS,
        )
        .unwrap();
        assert_eq!(e.endpoint(), "https://codecora-embed.example.com/v1/embed");
    }

    #[test]
    fn endpoint_path_without_leading_slash_normalized() {
        let e = OpenAiEmbedder::new(
            "k",
            DEFAULT_MODEL,
            "https://codecora-embed.example.com/v1",
            "embed",
            DEFAULT_DIMS,
        )
        .unwrap();
        // Should auto-prepend "/"
        assert_eq!(e.endpoint(), "https://codecora-embed.example.com/v1/embed");
    }
}
