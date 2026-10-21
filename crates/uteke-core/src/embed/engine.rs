//! ONNX-based embedding engine using EmbeddingGemma Q4 (768d).

use crate::Error;
use crate::embed::Embedder;
use crate::embed::ort_init;
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
/// Implements the [`Embedder`] trait. Uses **separate locks** for the
/// tokenizer and ONNX session so tokenization (Phase 1) and inference
/// (Phase 2) can proceed concurrently across threads.
///
/// **Lazy loading** (#896): The ONNX session and tokenizer are NOT loaded in
/// `new()`. They are loaded on the first `embed()` call via a double-check
/// locking pattern with a dedicated `init_lock`, guaranteeing exactly one
/// model load even under concurrent first calls. This allows
/// `CachingEmbedder` to skip the ~2s model load entirely on cache hits.
pub struct OnnxEmbedder {
    /// Lazily-initialized ONNX session.
    /// `!Sync` — must be serialized via `session_lock` during inference.
    session: Mutex<Option<ort::session::Session>>,
    /// Lazily-initialized tokenizer.
    /// Tokenization only needs `&self` on the tokenizer, so this lock is
    /// held briefly during Phase 1 and does NOT conflict with inference.
    tokenizer: Mutex<Option<tokenizers::Tokenizer>>,
    /// Serializes the one-time model load (double-check locking pattern).
    /// Prevents duplicate 188MB downloads under concurrent first calls.
    init_lock: Mutex<()>,
}

impl OnnxEmbedder {
    /// Create a new embedding engine.
    ///
    /// Does NOT load the ONNX model — model loading is deferred to the first
    /// `embed()` call (`lazy_load()`). This allows `CachingEmbedder` to skip
    /// the model load entirely when serving from cache (#896).
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            session: Mutex::new(None),
            tokenizer: Mutex::new(None),
            init_lock: Mutex::new(()),
        })
    }

    /// Lazily load the ONNX model on first use.
    /// Uses a sentinel-based double-check to guarantee exactly one
    /// `load_model()` call even under concurrent first calls — no duplicate
    /// 188MB downloads. Does NOT permanently cache failures: if `load_model()`
    /// returns an error, the next call will retry.
    fn lazy_load(&self) -> Result<(), Error> {
        // Fast path: already initialized (session is the sentinel — it is
        // stored LAST, so if session.is_some() then tokenizer is guaranteed set)
        {
            let session = self
                .session
                .lock()
                .map_err(|_| Error::lock("ONNX session during lazy_load fast path"))?;
            if session.is_some() {
                return Ok(());
            }
        }

        // Slow path: acquire init lock. This serializes the one-time load.
        let _init_guard = self
            .init_lock
            .lock()
            .map_err(|_| Error::lock("ONNX embedder init_lock"))?;

        // Double-check after acquiring init_lock: another thread may have
        // completed the load while we were waiting.
        {
            let session = self
                .session
                .lock()
                .map_err(|_| Error::lock("ONNX session during lazy_load double-check"))?;
            if session.is_some() {
                return Ok(());
            }
        }

        // We are the winner: load the model.
        // Note: we do NOT cache failures — transient errors should retry.
        let loaded = Self::load_model()?;

        // Store tokenizer FIRST, then session. Session is the sentinel —
        // other threads see is_some() only after both are stored, so there
        // is no window where one is set and the other is not.
        {
            let mut tokenizer = self
                .tokenizer
                .lock()
                .map_err(|_| Error::lock("ONNX tokenizer during store"))?;
            *tokenizer = Some(loaded.1);
        }
        let mut session = self
            .session
            .lock()
            .map_err(|_| Error::lock("ONNX session during store"))?;
        *session = Some(loaded.0);
        Ok(())
    }

    /// Download model files (if needed) and load ONNX session + tokenizer.
    /// Called once on first `embed()` invocation.
    fn load_model() -> Result<(ort::session::Session, tokenizers::Tokenizer), Error> {
        // ── Pre-flight: Initialize ORT environment with the correct library ──
        // This detects AVX2 support and loads the appropriate ONNX Runtime shared
        // library (standard AVX2 or legacy SSE4.2 sidecar). Must run before any
        // Session is created. The once_lock ensures we only init once per process.
        static ORT_INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
        ORT_INIT
            .get_or_init(|| ort_init::init_ort_environment().map(|_| ()))
            .as_ref()
            .map_err(|e| {
                Error::embed_msg(format!(
                    "ONNX Runtime initialization failed: {e}. \
                     Set ORT_LIB_PATH to the library file, or use the standard release bundle."
                ))
            })?;

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
        let needs_download =
            !model_path.exists() || !model_data_path.exists() || !tokenizer_path.exists();
        if needs_download {
            eprintln!("Downloading embedding model (first run)...");
        }
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

        // Load ONNX session — use all cores for intra-op parallelism
        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let environment = ort_init::environment().ok_or_else(|| {
            Error::embed_msg("ONNX Runtime environment is not initialized".to_string())
        })?;
        let session = ort::session::Session::builder(environment)
            .map_err(|e| Error::embed("ONNX session builder", e))?
            .with_intra_threads(num_threads)
            .map_err(|e| Error::embed("ONNX intra_threads config", e))?
            .commit_from_file(&model_path)
            .map_err(|e| Error::embed("load ONNX model", e))?;

        // Load tokenizer
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| Error::embed("load tokenizer", e))?;

        Ok((session, tokenizer))
    }

    /// Embed a text string, returning a 768-dimensional f32 vector.
    ///
    /// Takes `&self` — the tokenizer mutex is locked internally.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        // Lazy-load model on first embed() call
        self.lazy_load()?;

        // Phase 1: Tokenize + prepare tensors
        // Holds tokenizer_lock only — does NOT block inference (session_lock).
        let (input_ids_tensor, attention_mask_tensor) = {
            let tokenizer_guard = self
                .tokenizer
                .lock()
                .map_err(|_| Error::lock("ONNX tokenizer during tokenize"))?;

            let tokenizer = tokenizer_guard
                .as_ref()
                .ok_or_else(|| Error::embed_msg("tokenizer not loaded after lazy_load"))?;

            // Pre-truncate text to avoid tokenizing content beyond MAX_SEQ_LEN (#1002).
            // ~4 chars per token is a conservative estimate; truncating slightly short
            // is harmless because we discard tokens past MAX_SEQ_LEN anyway.
            const CHARS_PER_TOKEN: usize = 4;
            let max_chars = MAX_SEQ_LEN.saturating_mul(CHARS_PER_TOKEN);
            let truncated_text: &str = if text.len() > max_chars {
                // Find the nearest char boundary at or below max_chars (MSRV-safe).
                let mut end = max_chars;
                while end > 0 && !text.is_char_boundary(end) {
                    end -= 1;
                }
                &text[..end]
            } else {
                text
            };

            let encoding = tokenizer
                .encode(truncated_text, true)
                .map_err(|e| Error::embed("tokenize text", e))?;

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

            (input_ids_tensor, attention_mask_tensor)
        };
        // tokenizer_lock dropped — other threads can tokenize while we run inference.

        // Phase 2: Run ONNX inference
        // Holds session_lock only — does NOT block tokenization (tokenizer_lock).
        // session.run() is !Sync so this serializes inference calls, but
        // tokenization in Phase 1 proceeds concurrently on other threads.
        // EmbeddingGemma has 2 outputs:
        //   output[0] = last_hidden_state (1, seq_len, 768)
        //   output[1] = sentence_embedding (1, 768) — already mean-pooled
        let embedding: Vec<f32> = {
            let mut session_guard = self
                .session
                .lock()
                .map_err(|_| Error::lock("ONNX session during inference"))?;

            let session = session_guard
                .as_mut()
                .ok_or_else(|| Error::embed_msg("session not loaded after lazy_load"))?;

            let outputs = session
                .run(ort::inputs![input_ids_tensor, attention_mask_tensor])
                .map_err(|e| Error::embed("ONNX inference", e))?;

            // Use output[1] (sentence_embedding) — already pooled by the model
            let sentence_emb = &outputs[1];
            let emb_view = sentence_emb
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::embed("extract sentence embedding", e))?;

            emb_view.1.to_vec()
        };
        // session_lock dropped — post-processing runs without any lock.

        // L2 normalize
        let mut embedding = embedding;
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
