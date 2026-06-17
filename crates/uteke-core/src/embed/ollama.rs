//! Ollama embedding backend.
//!
//! POSTs to `{base_url}/api/embed` with `{ model, input }` JSON and parses
//! `embeddings[0]` (Ollama returns `embeddings` array, not `data`).
//!
//! Default endpoint: `http://localhost:11434` — no API key required.
//!
//! Dimensions are model-specific (nomic-embed-text = 768, mxbai-embed-large
//! = 1024). The caller supplies dims so we don't need a probe request.

use crate::embed::Embedder;
use crate::Error;

/// Default Ollama endpoint.
pub const DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// Default model — small, fast, good for local recall.
pub const DEFAULT_MODEL: &str = "nomic-embed-text";

/// Default dimensions for [`DEFAULT_MODEL`].
pub const DEFAULT_DIMS: usize = 768;

/// Ollama context window. We cap at a safe sub-window; the embed API does
/// its own truncation.
pub const MAX_SEQ_LEN: usize = 2048;

/// Ollama embedding backend.
#[derive(Debug)]
pub struct OllamaEmbedder {
    client: reqwest::blocking::Client,
    base_url: String,
    model: String,
    dims: usize,
}

impl OllamaEmbedder {
    pub fn new(base_url: &str, model: &str, dims: usize) -> Result<Self, Error> {
        if base_url.is_empty() {
            return Err(Error::Validation(
                "Ollama embedder requires a base_url (default: http://localhost:11434)".into(),
            ));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| Error::generic(format!("Failed to build HTTP client: {e}")))?;
        Ok(Self {
            client,
            base_url: base_url.to_string(),
            model: model.to_string(),
            dims,
        })
    }

    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/api/embed")
    }
}

impl Embedder for OllamaEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        let input = if text.is_empty() { " " } else { text };
        let body = serde_json::json!({
            "model": self.model,
            "input": input,
        });

        let resp = self
            .client
            .post(self.endpoint())
            .json(&body)
            .send()
            .map_err(|e| Error::generic(format!("Ollama embed request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            return Err(Error::generic(format!(
                "Ollama embed returned HTTP {status}: {text}"
            )));
        }

        // Ollama's /api/embed returns { "embeddings": [[...], ...] }
        let parsed: EmbedResponse = resp
            .json()
            .map_err(|e| Error::generic(format!("Failed to parse Ollama response: {e}")))?;

        let mut outer = parsed.embeddings.into_iter();
        let inner_vec = outer
            .next()
            .ok_or_else(|| Error::generic("Ollama response had no embeddings array"))?;
        Ok(inner_vec)
    }

    fn dims(&self) -> usize {
        self.dims
    }

    fn max_seq_len(&self) -> usize {
        MAX_SEQ_LEN
    }

    fn name(&self) -> &str {
        "ollama"
    }
}

#[derive(serde::Deserialize)]
struct EmbedResponse {
    // Ollama returns embeddings as Vec<Vec<f32>>. For single-input calls
    // there is one inner Vec.
    embeddings: Vec<Vec<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_base_url() {
        let err = OllamaEmbedder::new("", DEFAULT_MODEL, DEFAULT_DIMS).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("base_url"), "got: {msg}");
    }

    #[test]
    fn endpoint_normalizes_trailing_slash() {
        let e =
            OllamaEmbedder::new("http://localhost:11434/", DEFAULT_MODEL, DEFAULT_DIMS).unwrap();
        assert_eq!(e.endpoint(), "http://localhost:11434/api/embed");
    }

    #[test]
    fn defaults_match_docs() {
        assert_eq!(DEFAULT_MODEL, "nomic-embed-text");
        assert_eq!(DEFAULT_DIMS, 768);
        assert_eq!(DEFAULT_BASE_URL, "http://localhost:11434");
    }

    #[test]
    fn embedder_name_and_dims() {
        let e = OllamaEmbedder::new(DEFAULT_BASE_URL, "mxbai-embed-large", 1024).unwrap();
        assert_eq!(e.name(), "ollama");
        assert_eq!(e.dims(), 1024);
        assert_eq!(e.max_seq_len(), MAX_SEQ_LEN);
    }

    #[test]
    fn parses_successful_response() {
        let raw = r#"{
            "model": "nomic-embed-text",
            "embeddings": [[0.4, 0.5, 0.6]]
        }"#;
        let parsed: EmbedResponse = serde_json::from_str(raw).unwrap();
        let emb = parsed.embeddings.into_iter().next().unwrap();
        assert_eq!(emb, vec![0.4, 0.5, 0.6]);
    }
}
