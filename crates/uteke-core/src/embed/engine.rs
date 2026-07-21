//! ONNX-based embedding engine using EmbeddingGemma Q4 (768d).

use crate::embed::Embedder;
use crate::Error;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
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

/// How to turn ONNX outputs into a single embedding vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pooling {
    /// Model emits a pre-pooled sentence embedding at `outputs[1]`
    /// (EmbeddingGemma). Use it directly.
    PrePooledOutput1,
    /// Model emits only `last_hidden_state` at `outputs[0]`; mean-pool over
    /// the sequence using the attention mask (Qwen3-style, e.g. voyage-4-nano).
    MeanPoolOutput0,
}

/// Descriptor for a downloadable ONNX embedding model.
#[derive(Debug, Clone)]
pub struct ModelSpec {
    /// Local directory name under `~/.uteke/models/`.
    pub dir_name: &'static str,
    /// Hugging Face repo id.
    pub hf_repo: &'static str,
    /// Embedding dimension.
    pub dims: usize,
    /// Max input sequence length (tokens).
    pub max_seq_len: usize,
    /// Output pooling strategy.
    pub pooling: Pooling,
    /// (filename, sha256) checksums for model.onnx, model.onnx_data, tokenizer.json.
    pub checksums: &'static [(&'static str, &'static str)],
}

/// EmbeddingGemma Q4 (768d) — the default local model.
pub const EMBEDDINGGEMMA_Q4: ModelSpec = ModelSpec {
    dir_name: MODEL_DIR_NAME,
    hf_repo: HF_REPO,
    dims: MODEL_DIMS,
    max_seq_len: MAX_SEQ_LEN,
    pooling: Pooling::PrePooledOutput1,
    checksums: MODEL_CHECKSUMS,
};

/// Voyage-4-nano Q4 (2048d, qwen3) — open-weight, code-friendly local model.
///
/// The ONNX export emits `last_hidden_state (…, 2048)` and a pre-pooled
/// `pooler_output (…, 2048)` at index 1, so it uses the same pooling path as
/// EmbeddingGemma. Native dim is 2048 (config.json's 1024 is the stale base
/// hidden size before the embedding projection).
pub const VOYAGE_4_NANO_Q4: ModelSpec = ModelSpec {
    dir_name: "voyage-4-nano-q4",
    hf_repo: "onnx-community/voyage-4-nano-ONNX",
    dims: 2048,
    max_seq_len: 32_000,
    pooling: Pooling::PrePooledOutput1,
    checksums: VOYAGE_4_NANO_CHECKSUMS,
};

/// SHA256 checksums for voyage-4-nano Q4 ONNX files.
const VOYAGE_4_NANO_CHECKSUMS: &[(&str, &str)] = &[
    (
        "model_q4.onnx",
        "2a2f390055b2ab4f17e9e57ee8a8f948f905ea33e9111af0b297c9d8d372b99a",
    ),
    (
        "model_q4.onnx_data",
        "38e29a9146c9f94bacee268002d453d146fb21805826b0dd26074e2f0e886abf",
    ),
    (
        "tokenizer.json",
        "c40c3736449ad0f4084a187dfe16f6850b2a2933dfe041394f850608b2890140",
    ),
];

/// Resolve a model spec by name. Accepts the config `model` value.
/// Recognized: "embeddinggemma-q4" (default), "voyage-4-nano".
#[allow(dead_code)]
pub fn model_spec_for(name: &str) -> ModelSpec {
    match name {
        "voyage" | "voyage-4-nano" | "voyage-4-nano-q4" => VOYAGE_4_NANO_Q4,
        _ => EMBEDDINGGEMMA_Q4,
    }
}

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
    spec: ModelSpec,
}

impl OnnxEmbedder {
    /// Create the default embedding engine (EmbeddingGemma Q4).
    /// Downloads model if not cached.
    pub fn new() -> Result<Self, Error> {
        Self::with_spec(EMBEDDINGGEMMA_Q4)
    }

    /// Create an embedding engine for a specific model spec.
    /// Downloads model files if not cached.
    pub fn with_spec(spec: ModelSpec) -> Result<Self, Error> {
        let model_dir = Self::model_dir_for(&spec)?;
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
        let needs_download =
            !model_path.exists() || !model_data_path.exists() || !tokenizer_path.exists();
        if needs_download {
            eprintln!("Downloading embedding model '{}' (first run)...", spec.dir_name);
        }
        if !model_path.exists() {
            download_hf_file(spec.hf_repo, "onnx/model_q4.onnx", &model_path)?;
            verify_checksum_with(&model_path, "model_q4.onnx", spec.checksums)?;
        }
        if !model_data_path.exists() {
            download_hf_file(spec.hf_repo, "onnx/model_q4.onnx_data", &model_data_path)?;
            verify_checksum_with(&model_data_path, "model_q4.onnx_data", spec.checksums)?;
        }
        if !tokenizer_path.exists() {
            download_hf_file(spec.hf_repo, "tokenizer.json", &tokenizer_path)?;
            verify_checksum_with(&tokenizer_path, "tokenizer.json", spec.checksums)?;
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
            spec,
        })
    }

    /// Embed a text string, returning a spec-dimensional f32 vector.
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
        let seq_len = input_ids.len().min(self.spec.max_seq_len);

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
            attention_mask_i64.clone().into_boxed_slice(),
        ))
        .map_err(|e| Error::embed("create attention_mask tensor", e))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| Error::lock("session lock during embedding"))?;
        let outputs = session
            .run(ort::inputs![input_ids_tensor, attention_mask_tensor])
            .map_err(|e| Error::embed("ONNX inference", e))?;

        let mut embedding: Vec<f32> = match self.spec.pooling {
            Pooling::PrePooledOutput1 => {
                // outputs[1] = sentence_embedding (1, dims) — already pooled.
                let sentence_emb = &outputs[1];
                let emb_view = sentence_emb
                    .try_extract_tensor::<f32>()
                    .map_err(|e| Error::embed("extract sentence embedding", e))?;
                emb_view.1.to_vec()
            }
            Pooling::MeanPoolOutput0 => {
                // outputs[0] = last_hidden_state (1, seq_len, hidden). Mean-pool
                // over non-masked tokens.
                let hidden = &outputs[0];
                let (shape, data) = hidden
                    .try_extract_tensor::<f32>()
                    .map_err(|e| Error::embed("extract last_hidden_state", e))?;
                mean_pool(shape, data, &attention_mask_i64, self.spec.dims)?
            }
        };

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
    /// Returns the default model's dims.
    pub fn dims() -> usize {
        MODEL_DIMS
    }

    fn model_dir_for(spec: &ModelSpec) -> Result<PathBuf, Error> {
        crate::uteke_home().map(|p| p.join("models").join(spec.dir_name))
    }
}

/// Mean-pool a `last_hidden_state` tensor `(1, seq_len, hidden)` over tokens
/// whose attention mask is non-zero. Returns a `hidden`-length vector.
fn mean_pool(
    shape: &[i64],
    data: &[f32],
    attention_mask: &[i64],
    expected_dims: usize,
) -> Result<Vec<f32>, Error> {
    // shape = [batch=1, seq_len, hidden]
    if shape.len() != 3 {
        return Err(Error::embed_msg(format!(
            "unexpected last_hidden_state rank {} (expected 3)",
            shape.len()
        )));
    }
    let seq_len = shape[1] as usize;
    let hidden = shape[2] as usize;
    if hidden != expected_dims {
        return Err(Error::embed_msg(format!(
            "model hidden size {hidden} != expected dims {expected_dims}"
        )));
    }
    let mut acc = vec![0f32; hidden];
    let mut count = 0f32;
    for t in 0..seq_len {
        let masked = attention_mask.get(t).copied().unwrap_or(1) != 0;
        if !masked {
            continue;
        }
        let base = t * hidden;
        for h in 0..hidden {
            acc[h] += data[base + h];
        }
        count += 1.0;
    }
    if count > 0.0 {
        for v in acc.iter_mut() {
            *v /= count;
        }
    }
    Ok(acc)
}

impl Embedder for OnnxEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        // Delegate to inherent method (which locks the tokenizer mutex).
        OnnxEmbedder::embed(self, text)
    }

    fn dims(&self) -> usize {
        self.spec.dims
    }

    fn max_seq_len(&self) -> usize {
        self.spec.max_seq_len
    }

    fn name(&self) -> &str {
        self.spec.dir_name
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

/// Maximum number of download retries.
const MAX_RETRIES: u32 = 3;

/// Connect timeout for HTTP downloads (seconds).
const CONNECT_TIMEOUT_SECS: u64 = 30;

/// Read timeout for HTTP downloads (seconds) — generous for the 187MB data file.
const READ_TIMEOUT_SECS: u64 = 300;

/// Download a file from HuggingFace repo to local path.
///
/// Uses streaming write to a `.tmp` file + atomic rename to prevent corrupt
/// files on crash. Includes connect/read timeouts, retry on transient errors,
/// and a progress indicator for large files.
fn download_hf_file(
    repo: &str,
    path_in_repo: &str,
    local_path: &std::path::Path,
) -> Result<(), Error> {
    let url = format!("https://huggingface.co/{repo}/resolve/main/{path_in_repo}");
    let file_name = local_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| path_in_repo.to_string());

    let tmp_path = local_path.with_file_name(format!("{file_name}.tmp"));

    let mut last_err: Option<String> = None;
    for attempt in 1..=MAX_RETRIES {
        if attempt > 1 {
            eprintln!("  Retry {attempt}/{MAX_RETRIES}...");
        }

        match download_hf_file_once(&url, &file_name, &tmp_path) {
            Ok(()) => {
                std::fs::rename(&tmp_path, local_path)
                    .map_err(|e| Error::embed("rename temp to final path", e))?;
                set_owner_only_permissions(local_path);
                return Ok(());
            }
            Err(e) => {
                last_err = Some(format!("{e}"));
                // Clean up partial download so retry starts fresh.
                std::fs::remove_file(&tmp_path).ok();
            }
        }
    }

    Err(Error::embed_msg(format!(
        "Download failed after {MAX_RETRIES} attempts: {}",
        last_err.unwrap_or_default()
    )))
}

/// Single download attempt: stream the response body to a temp file.
fn download_hf_file_once(
    url: &str,
    file_name: &str,
    tmp_path: &std::path::Path,
) -> Result<(), Error> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(std::time::Duration::from_secs(READ_TIMEOUT_SECS))
        .build()
        .map_err(|e| Error::embed("build HTTP client", e))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| Error::embed("download model file", e))?;

    if !response.status().is_success() {
        return Err(Error::embed_msg(format!(
            "Download failed with status {} for {url}",
            response.status()
        )));
    }

    let total_size = response.content_length().unwrap_or(0);

    eprintln!(
        "  {file_name} ({total_human})",
        total_human = human_bytes(total_size)
    );

    // Stream the response body to disk — avoids buffering the entire 187MB in RAM.
    let mut tmp_file =
        std::fs::File::create(tmp_path).map_err(|e| Error::embed("create temp file", e))?;
    let mut downloaded: u64 = 0;
    let mut last_pct: u8 = 0;

    let mut reader = response;
    let mut buf = vec![0u8; 64 * 1024]; // 64KB chunks
    loop {
        let bytes_read = reader
            .read(&mut buf)
            .map_err(|e| Error::embed("read download stream", e))?;
        if bytes_read == 0 {
            break;
        }
        tmp_file
            .write_all(&buf[..bytes_read])
            .map_err(|e| Error::embed("write temp file", e))?;
        downloaded += bytes_read as u64;

        // Print progress every 10% for large files.
        if total_size > 0 {
            if let Some(pct) = downloaded
                .checked_mul(100)
                .and_then(|v| v.checked_div(total_size))
            {
                let pct = pct as u8;
                if pct != last_pct && pct % 10 == 0 {
                    eprintln!(
                        "  {file_name}: {pct}% ({}/{} bytes)",
                        downloaded, total_size
                    );
                    last_pct = pct;
                }
            }
        }
    }
    tmp_file
        .sync_all()
        .map_err(|e| Error::embed("flush temp file", e))?;
    drop(tmp_file);

    eprintln!("  ✓ {file_name} downloaded ({})", human_bytes(downloaded));

    // Verify we got the expected bytes when content-length was known.
    if total_size > 0 && downloaded != total_size {
        return Err(Error::embed_msg(format!(
            "Incomplete download: expected {} bytes, got {}",
            total_size, downloaded
        )));
    }

    Ok(())
}

/// Format a byte count as a human-readable string (e.g. "187.0 MB").
fn human_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} bytes")
    }
}

/// Verify SHA256 checksum of a downloaded model file.
#[allow(dead_code)]
fn verify_checksum(path: &std::path::Path, filename: &str) -> Result<(), Error> {
    verify_checksum_with(path, filename, MODEL_CHECKSUMS)
}

/// Verify a downloaded file against a supplied checksum table.
fn verify_checksum_with(
    path: &std::path::Path,
    filename: &str,
    checksums: &[(&str, &str)],
) -> Result<(), Error> {
    let expected = checksums
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_spec_for_resolves_voyage_and_default() {
        assert_eq!(model_spec_for("voyage-4-nano").dir_name, "voyage-4-nano-q4");
        assert_eq!(model_spec_for("voyage").dir_name, "voyage-4-nano-q4");
        assert_eq!(model_spec_for("anything-else").dir_name, MODEL_DIR_NAME);
        assert_eq!(model_spec_for("").dir_name, MODEL_DIR_NAME);
    }

    #[test]
    fn voyage_spec_is_2048d_prepooled() {
        assert_eq!(VOYAGE_4_NANO_Q4.dims, 2048);
        assert_eq!(VOYAGE_4_NANO_Q4.pooling, Pooling::PrePooledOutput1);
        assert_eq!(VOYAGE_4_NANO_Q4.max_seq_len, 32_000);
    }

    #[test]
    fn mean_pool_averages_unmasked_tokens() {
        // shape (1, 2, 2), tokens: [1,2] and [3,4], both unmasked -> mean [2,3]
        let shape = [1i64, 2, 2];
        let data = [1.0f32, 2.0, 3.0, 4.0];
        let mask = [1i64, 1];
        let out = mean_pool(&shape, &data, &mask, 2).unwrap();
        assert_eq!(out, vec![2.0, 3.0]);
    }

    #[test]
    fn mean_pool_skips_masked_tokens() {
        // second token masked -> mean = first token [1,2]
        let shape = [1i64, 2, 2];
        let data = [1.0f32, 2.0, 100.0, 100.0];
        let mask = [1i64, 0];
        let out = mean_pool(&shape, &data, &mask, 2).unwrap();
        assert_eq!(out, vec![1.0, 2.0]);
    }

    #[test]
    fn mean_pool_rejects_dim_mismatch() {
        let shape = [1i64, 1, 3];
        let data = [1.0f32, 2.0, 3.0];
        let mask = [1i64];
        assert!(mean_pool(&shape, &data, &mask, 2).is_err());
    }
}
