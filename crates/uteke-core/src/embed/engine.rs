//! ONNX-based embedding engine using EmbeddingGemma Q4 (768d).

use crate::embed::Embedder;
use crate::Error;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const MODEL_DIR_NAME: &str = "embeddinggemma-q4";
const MODEL_FILE: &str = "model_q4.onnx";
const MODEL_DATA_FILE: &str = "model_q4.onnx_data";
const TOKENIZER_FILE: &str = "tokenizer.json";
const MODEL_DIMS: usize = 768;
const MAX_SEQ_LEN: usize = 2048;

const HF_REPO: &str = "onnx-community/embeddinggemma-300m-ONNX";

/// Expected SHA256 checksums for model files.
/// Pin these to prevent corrupted/tampered downloads from causing cryptic ONNX failures.
const MODEL_CHECKSUMS: &[(&str, &str)] = &[
    (
        "model_q4.onnx",
        "ad1dfee81a70f7944b9b9d1cc6e48075b832881cf33fab2f2b248be78f3f0043",
    ),
    (
        "model_q4.onnx_data",
        "599962c3143b040de2dd05e5975be3e9091dd067cacc6a8f7186e3203bab9e02",
    ),
    (
        "tokenizer.json",
        "4dda02faaf32bc91031dc8c88457ac272b00c1016cc679757d1c441b248b9c47",
    ),
];

/// ONNX-based embedding engine using EmbeddingGemma Q4 (768d).
///
/// Implements the [`Embedder`] trait. The tokenizer is wrapped in a `Mutex`
/// so `embed()` can take `&self` (required by the trait) instead of `&mut self`.
pub struct OnnxEmbedder {
    session: Mutex<ort::session::Session>,
    tokenizer: Mutex<tokenizers::Tokenizer>,
}

impl OnnxEmbedder {
    /// Create a new embedding engine. Downloads model if not cached.
    pub fn new() -> Result<Self, Error> {
        let model_dir = Self::model_dir()?;
        std::fs::create_dir_all(&model_dir)
            .map_err(|e| Error::embed("create model directory", e))?;

        let onnx_dir = model_dir.join("onnx");
        std::fs::create_dir_all(&onnx_dir).map_err(|e| Error::embed("create onnx directory", e))?;

        // Set model directory permissions to owner-only (0700) on Unix
        #[cfg(unix)]
        {
            std::fs::set_permissions(&model_dir, std::fs::Permissions::from_mode(0o700)).ok();
            std::fs::set_permissions(&onnx_dir, std::fs::Permissions::from_mode(0o700)).ok();
        }

        let model_path = onnx_dir.join(MODEL_FILE);
        let model_data_path = onnx_dir.join(MODEL_DATA_FILE);
        let tokenizer_path = model_dir.join(TOKENIZER_FILE);

        // Clean up leftover .tmp files from interrupted downloads
        clean_tmp_files(&onnx_dir);
        clean_tmp_files(&model_dir);

        // Download model files if not present
        if !model_path.exists() {
            download_hf_file(HF_REPO, "onnx/model_q4.onnx", &model_path)?;
            verify_checksum(&model_path, "model_q4.onnx")?;
        }
        if !model_data_path.exists() {
            download_hf_file(HF_REPO, "onnx/model_q4.onnx_data", &model_data_path)?;
            verify_checksum(&model_data_path, "model_q4.onnx_data")?;
        }
        if !tokenizer_path.exists() {
            download_hf_file(HF_REPO, "tokenizer.json", &tokenizer_path)?;
            verify_checksum(&tokenizer_path, "tokenizer.json")?;
        }

        // Load ONNX session
        let session = ort::session::Session::builder()
            .and_then(|mut b| b.commit_from_file(&model_path))
            .map_err(|e| Error::embed("load ONNX model", e))?;

        // Load tokenizer
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| Error::embed("load tokenizer", e))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer: Mutex::new(tokenizer),
        })
    }

    /// Embed a text string, returning a 768-dimensional f32 vector.
    ///
    /// Takes `&self` — the tokenizer mutex is locked internally.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        // Tokenize (lock mutex)
        let tokenizer = self
            .tokenizer
            .lock()
            .map_err(|_| Error::lock("tokenizer lock during embedding"))?;
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| Error::embed("tokenize text", e))?;
        drop(tokenizer); // release lock before inference

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
        .map_err(|e| Error::embed("create input_ids tensor", e))?;

        let attention_mask_tensor = ort::value::Tensor::<i64>::from_array((
            vec![1i64, seq_len as i64],
            attention_mask_i64.into_boxed_slice(),
        ))
        .map_err(|e| Error::embed("create attention_mask tensor", e))?;

        // Run ONNX inference — EmbeddingGemma has 2 outputs:
        //   output[0] = last_hidden_state (1, seq_len, 768)
        //   output[1] = sentence_embedding (1, 768) — already mean-pooled
        let mut session = self
            .session
            .lock()
            .map_err(|_| Error::lock("session lock during embedding"))?;
        let outputs = session
            .run(ort::inputs![input_ids_tensor, attention_mask_tensor])
            .map_err(|e| Error::embed("ONNX inference", e))?;

        // Use output[1] (sentence_embedding) — already pooled by the model
        let sentence_emb = &outputs[1];

        let emb_view = sentence_emb
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::embed("extract sentence embedding", e))?;

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

    /// Get the embedding dimension (associated function for backward compat).
    pub fn dims() -> usize {
        MODEL_DIMS
    }

    fn model_dir() -> Result<PathBuf, Error> {
        crate::uteke_home().map(|p| p.join("models").join(MODEL_DIR_NAME))
    }
}

impl Embedder for OnnxEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        // Delegate to inherent method (which locks the tokenizer mutex).
        OnnxEmbedder::embed(self, text)
    }

    fn dims(&self) -> usize {
        MODEL_DIMS
    }

    fn max_seq_len(&self) -> usize {
        MAX_SEQ_LEN
    }

    fn name(&self) -> &str {
        "embeddinggemma-q4"
    }
}

/// Delete leftover .tmp files from interrupted atomic downloads.
fn clean_tmp_files(dir: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "tmp") {
                tracing::debug!("Cleaning up temp file: {}", path.display());
                std::fs::remove_file(&path).ok();
            }
        }
    }
}

/// Download a file from HuggingFace repo to local path.
/// Uses atomic write (.tmp + rename) to prevent corrupt files on crash.
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
        .map_err(|e| Error::embed("download model file", e))?;

    if !response.status().is_success() {
        return Err(Error::embed_msg(format!(
            "Download failed with status {} for {url}",
            response.status()
        )));
    }

    let bytes = response
        .bytes()
        .map_err(|e| Error::embed("read download response", e))?;

    // Atomic write: write to .tmp then rename — prevents corrupt files on crash.
    let tmp_path = local_path.with_file_name(format!(
        "{}.tmp",
        local_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    std::fs::write(&tmp_path, bytes.as_ref())
        .map_err(|e| Error::embed("write temporary file", e))?;
    std::fs::rename(&tmp_path, local_path)
        .map_err(|e| Error::embed("rename temp to final path", e))?;

    // Set file permissions to owner-only (0600) on Unix
    set_owner_only_permissions(local_path);

    Ok(())
}

/// Verify SHA256 checksum of a downloaded model file.
fn verify_checksum(path: &std::path::Path, filename: &str) -> Result<(), Error> {
    let expected = MODEL_CHECKSUMS
        .iter()
        .find(|(name, _)| name == &filename)
        .map(|(_, hash)| *hash)
        .ok_or_else(|| Error::embed_msg(format!("No checksum pinned for {filename}")))?;

    let data = std::fs::read(path).map_err(|e| Error::embed("read file for checksum", e))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let digest = hasher.finalize();
    // sha2 0.11 dropped the LowerHex impl on the digest Array type, so format
    // the 32 bytes as lowercase hex manually.
    let actual: String = digest.iter().map(|b| format!("{b:02x}")).collect();

    if actual != expected {
        // Delete corrupted file so next run re-downloads
        std::fs::remove_file(path).ok();
        return Err(Error::embed_msg(format!(
            "SHA256 checksum mismatch for {filename}.\n\
             Expected: {expected}\n\
             Actual:   {actual}\n\
             File deleted. Re-run to re-download."
        )));
    }
    tracing::debug!("Checksum verified: {filename}");
    Ok(())
}

/// Set file permissions to owner-only (0600) on Unix systems.
fn set_owner_only_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!("Failed to set permissions on {}: {e}", path.display());
        }
    }
}
