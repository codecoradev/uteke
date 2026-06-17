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

/// OpenAI max sequence length in tokens (clamped by the API).
pub const MAX_SEQ_LEN: usize = 8191;

/// OpenAI embedding backend.
#[derive(Debug)]
pub struct OpenAiEmbedder {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    model: String,
    dims: usize,
}

impl OpenAiEmbedder {
    /// Construct a new OpenAI embedder.
    ///
    /// `api_key` is required; resolution from env/config is the caller's job
    /// (see `lib.rs::ensure_embedder`).
    pub fn new(api_key: &str, model: &str, base_url: &str, dims: usize) -> Result<Self, Error> {
        if api_key.is_empty() {
            return Err(Error::Validation(
                "OpenAI embedder requires an API key (set UTEKE_EMBEDDING_API_KEY or OPENAI_API_KEY)".into(),
            ));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::generic(format!("Failed to build HTTP client: {e}")))?;
        Ok(Self {
            client,
            api_key: api_key.to_string(),
            base_url: base_url.to_string(),
            model: model.to_string(),
            dims,
        })
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/embeddings")
    }
}

impl Embedder for OpenAiEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        // OpenAI rejects empty input with 400; send a single space so the
        // call is always valid (matches ONNX backend's non-empty contract).
        let input = if text.is_empty() { " " } else { text };
        let body = serde_json::json!({
            "model": self.model,
            "input": input,
        });

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
    fn rejects_empty_api_key() {
        let err =
            OpenAiEmbedder::new("", DEFAULT_MODEL, DEFAULT_BASE_URL, DEFAULT_DIMS).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("API key"), "got: {msg}");
    }

    #[test]
    fn endpoint_normalizes_trailing_slash() {
        let e = OpenAiEmbedder::new(
            "k",
            DEFAULT_MODEL,
            "https://api.openai.com/v1/",
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
        assert_eq!(MAX_SEQ_LEN, 8191);
    }

    #[test]
    fn embedder_name_and_dims() {
        let e = OpenAiEmbedder::new("k", DEFAULT_MODEL, DEFAULT_BASE_URL, 3072).unwrap();
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
}
