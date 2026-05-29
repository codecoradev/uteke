//! ONNX-based embedding engine using EmbeddingGemma Q4 (768d).

use crate::Error;
use std::path::PathBuf;

const MODEL_DIR_NAME: &str = "embeddinggemma-q4";
const MODEL_FILE: &str = "model_q4.onnx";
const MODEL_DATA_FILE: &str = "model_q4.onnx_data";
const TOKENIZER_FILE: &str = "tokenizer.json";
const MODEL_DIMS: usize = 768;
const MAX_SEQ_LEN: usize = 256;

const HF_REPO: &str = "onnx-community/embeddinggemma-300m-ONNX";

/// ONNX-based embedding engine using EmbeddingGemma Q4 (768d).
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

        let onnx_dir = model_dir.join("onnx");
        std::fs::create_dir_all(&onnx_dir)
            .map_err(|e| Error::Embedding(format!("Failed to create onnx dir: {e}")))?;

        let model_path = onnx_dir.join(MODEL_FILE);
        let model_data_path = onnx_dir.join(MODEL_DATA_FILE);
        let tokenizer_path = model_dir.join(TOKENIZER_FILE);

        // Download model files if not present
        if !model_path.exists() {
            download_hf_file(HF_REPO, "onnx/model_q4.onnx", &model_path)?;
        }
        if !model_data_path.exists() {
            download_hf_file(HF_REPO, "onnx/model_q4.onnx_data", &model_data_path)?;
        }
        if !tokenizer_path.exists() {
            download_hf_file(HF_REPO, "tokenizer.json", &tokenizer_path)?;
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

    /// Embed a text string, returning a 768-dimensional f32 vector.
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>, Error> {
        // Tokenize
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| Error::Embedding(format!("Tokenization failed: {e}")))?;

        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        // Truncate to max sequence length
        let seq_len = input_ids.len().min(MAX_SEQ_LEN);

        // Prepare input arrays as i64
        let input_ids_i64: Vec<i64> = input_ids[..seq_len].iter().map(|&v| v as i64).collect();
        let attention_mask_i64: Vec<i64> = attention_mask[..seq_len]
            .iter()
            .map(|&v| v as i64)
            .collect();

        // Create tensors
        let input_ids_tensor = ort::value::Tensor::<i64>::from_array((
            vec![1i64, seq_len as i64],
            input_ids_i64.into_boxed_slice(),
        ))
        .map_err(|e| Error::Embedding(format!("Failed to create input_ids tensor: {e}")))?;

        let attention_mask_tensor = ort::value::Tensor::<i64>::from_array((
            vec![1i64, seq_len as i64],
            attention_mask_i64.into_boxed_slice(),
        ))
        .map_err(|e| Error::Embedding(format!("Failed to create attention_mask tensor: {e}")))?;

        // Run ONNX inference — EmbeddingGemma has 2 outputs:
        //   output[0] = last_hidden_state (1, seq_len, 768)
        //   output[1] = sentence_embedding (1, 768) — already mean-pooled
        let outputs = self
            .session
            .run(ort::inputs![input_ids_tensor, attention_mask_tensor])
            .map_err(|e| Error::Embedding(format!("ONNX inference failed: {e}")))?;

        // Use output[1] (sentence_embedding) — already pooled by the model
        let sentence_emb = &outputs[1];

        let emb_view = sentence_emb
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::Embedding(format!("Failed to extract sentence embedding: {e}")))?;

        let mut embedding: Vec<f32> = emb_view.1.to_vec();

        // L2 normalize
        let norm = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in embedding.iter_mut() {
                *v /= norm;
            }
        }

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

/// Download a file from HuggingFace repo to local path.
fn download_hf_file(
    repo: &str,
    path_in_repo: &str,
    local_path: &std::path::Path,
) -> Result<(), Error> {
    let url = format!("https://huggingface.co/{repo}/resolve/main/{path_in_repo}");
    eprintln!("Downloading {url}...");

    let response = reqwest::blocking::Client::new()
        .get(&url)
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

    std::fs::write(local_path, bytes.as_ref())
        .map_err(|e| Error::Embedding(format!("Failed to write file: {e}")))?;

    Ok(())
}
