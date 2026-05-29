//! ONNX-based embedding engine using all-MiniLM-L6-v2.

use crate::Error;
use std::path::PathBuf;

const MODEL_DIR_NAME: &str = "minilm-l6-v2";
const MODEL_FILE: &str = "model.onnx";
const TOKENIZER_FILE: &str = "tokenizer.json";
const MODEL_DIMS: usize = 384;
const MAX_SEQ_LEN: usize = 256;

const MODEL_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/model.onnx";
const TOKENIZER_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json";

/// ONNX-based embedding engine.
pub struct EmbeddingEngine {
    session: ort::session::Session,
    tokenizer: tokenizers::Tokenizer,
}

impl EmbeddingEngine {
    /// Create a new embedding engine. Downloads model if not cached.
    pub fn new() -> Result<Self, Error> {
        let model_dir = Self::model_dir()?;
        std::fs::create_dir_all(&model_dir)
            .map_err(|e| Error::Embedding(format!("Failed to create model dir: {e}")))?;

        let model_path = model_dir.join(MODEL_FILE);
        let tokenizer_path = model_dir.join(TOKENIZER_FILE);

        // Download model if not present
        if !model_path.exists() {
            download_file(MODEL_URL, &model_path)?;
        }

        // Download tokenizer if not present
        if !tokenizer_path.exists() {
            download_file(TOKENIZER_URL, &tokenizer_path)?;
        }

        // Load ONNX session
        let session = ort::session::Session::builder()
            .and_then(|mut b| b.commit_from_file(&model_path))
            .map_err(|e| Error::Embedding(format!("Failed to load ONNX model: {e}")))?;

        // Load tokenizer
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| Error::Embedding(format!("Failed to load tokenizer: {e}")))?;

        Ok(Self { session, tokenizer })
    }

    /// Embed a text string, returning a 384-dimensional f32 vector.
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>, Error> {
        // Tokenize
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| Error::Embedding(format!("Tokenization failed: {e}")))?;

        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();
        let token_type_ids = encoding.get_type_ids();

        // Truncate to max sequence length
        let seq_len = input_ids.len().min(MAX_SEQ_LEN);

        // Prepare input arrays as i64
        let input_ids_i64: Vec<i64> = input_ids[..seq_len].iter().map(|&v| v as i64).collect();
        let attention_mask_i64: Vec<i64> = attention_mask[..seq_len]
            .iter()
            .map(|&v| v as i64)
            .collect();
        let token_type_ids_i64: Vec<i64> = token_type_ids[..seq_len]
            .iter()
            .map(|&v| v as i64)
            .collect();

        // Create tensors using (shape, data) tuples
        let input_ids_tensor = ort::value::Tensor::<i64>::from_array((
            vec![1i64, seq_len as i64],
            input_ids_i64.into_boxed_slice(),
        ))
        .map_err(|e| Error::Embedding(format!("Failed to create input_ids tensor: {e}")))?;

        let attention_mask_tensor = ort::value::Tensor::<i64>::from_array((
            vec![1i64, seq_len as i64],
            attention_mask_i64.clone().into_boxed_slice(),
        ))
        .map_err(|e| Error::Embedding(format!("Failed to create attention_mask tensor: {e}")))?;

        let token_type_ids_tensor = ort::value::Tensor::<i64>::from_array((
            vec![1i64, seq_len as i64],
            token_type_ids_i64.into_boxed_slice(),
        ))
        .map_err(|e| Error::Embedding(format!("Failed to create token_type_ids tensor: {e}")))?;

        // Run ONNX inference
        let outputs = self
            .session
            .run(ort::inputs![
                input_ids_tensor,
                attention_mask_tensor,
                token_type_ids_tensor
            ])
            .map_err(|e| Error::Embedding(format!("ONNX inference failed: {e}")))?;

        // Extract output tensor: shape (1, seq_len, 384)
        let output = &outputs[0];

        let output_view = output
            .try_extract_array::<f32>()
            .map_err(|e| Error::Embedding(format!("Failed to extract output: {e}")))?;

        // Mean pool over non-padding tokens, then normalize
        let embedding = mean_pool_and_normalize(&output_view, &attention_mask_i64, seq_len);

        Ok(embedding)
    }

    /// Get the embedding dimension.
    pub fn dims() -> usize {
        MODEL_DIMS
    }

    fn model_dir() -> Result<PathBuf, Error> {
        let home = dirs::home_dir()
            .ok_or_else(|| Error::Embedding("Cannot determine home directory".into()))?;
        Ok(home.join(".uteke").join("models").join(MODEL_DIR_NAME))
    }
}

/// Mean pool the token embeddings using attention mask, then L2-normalize.
fn mean_pool_and_normalize(
    output: &ndarray::ArrayViewD<f32>,
    attention_mask: &[i64],
    seq_len: usize,
) -> Vec<f32> {
    let mut pooled = vec![0.0f32; MODEL_DIMS];
    let mut mask_sum = 0.0f32;

    for t in 0..seq_len {
        if attention_mask[t] == 1 {
            mask_sum += 1.0;
            for d in 0..MODEL_DIMS {
                pooled[d] += output[[t, d]];
            }
        }
    }

    if mask_sum > 0.0 {
        for v in pooled.iter_mut() {
            *v /= mask_sum;
        }
    }

    // L2 normalize
    let norm = pooled.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in pooled.iter_mut() {
            *v /= norm;
        }
    }

    pooled
}

/// Download a file from URL to path.
fn download_file(url: &str, path: &std::path::Path) -> Result<(), Error> {
    let response = reqwest::blocking::Client::new()
        .get(url)
        .send()
        .map_err(|e| Error::Embedding(format!("Failed to download {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(Error::Embedding(format!(
            "Download failed with status {} for {url}",
            response.status()
        )));
    }

    let bytes = response
        .bytes()
        .map_err(|e| Error::Embedding(format!("Failed to read download: {e}")))?;

    std::fs::write(path, bytes.as_ref())
        .map_err(|e| Error::Embedding(format!("Failed to write file: {e}")))?;

    Ok(())
}
