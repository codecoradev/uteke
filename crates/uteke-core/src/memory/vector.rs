//! Persistent vector index with pluggable backends.
//!
//! Backends are selected at RUNTIME (#1168) while remaining compile-time
//! slim-buildable:
//!
//! - `usearch`: HNSW with disk persistence via C++ FFI.
//! - `vecq`: training-free 4-bit + residual vector quantization, pure Rust,
//!   no C++ toolchain (originally for mobile/FFI builds, #1098).
//!
//! A build may compile in one or both engines (cargo features `usearch` /
//! `vecq`). Which engine a `VectorIndex` uses is decided when it is created:
//! `UTEKE_VECTOR_BACKEND` env → explicit constructor argument (from
//! `uteke.toml [vector] backend`) → compiled-in default. At least one engine
//! feature must be enabled.
//!
//! The public API (`new`, `load_or_create`, `insert`, `remove`, `search`,
//! `build`, `save`, `len`, `dims`, ...) is identical for both backends —
//! callers never branch on the backend.
//!
//! Index files are per-engine: `uteke_index.usearch` vs `uteke_index.vecq`.
//! Switching engines leaves the old file untouched; the new engine finds no
//! index and the caller (lib.rs `finish_open_full`) rebuilds from SQLite,
//! which remains the source of truth.
//!
//! Cross-process safety (#543): Each VectorIndex acquires an exclusive file
//! lock (via fs2) on the index file during construction. The lock is held
//! until the VectorIndex is dropped, serializing concurrent CLI invocations
//! that share the same on-disk index. In-process thread safety uses
//! RwLock<VectorIndex> in lib.rs.
//!
//! Windows compatibility (#647, #684): Both `save()` and `load()` use
//! buffer-based serialization to bypass usearch's C++ file I/O (`fopen`,
//! `fread`, `mmap`) which has Windows-specific issues (MAX_PATH, file lock
//! conflicts, AV interference). Save serializes to memory then atomic-writes
//! via Rust std::fs; load reads via Rust std::fs then deserializes from buffer.
//!
//! vecq format note: vecq has no incremental delete — removed rows are
//! tombstoned (the key vanishes from the key map) and filtered out of search
//! results. Tombstones are derived from the key-mapping sidecar on load, so
//! no extra on-disk state is needed.

#[cfg(not(any(feature = "usearch", feature = "vecq")))]
compile_error!(
    "uteke-core requires a vector index backend; enable `usearch` (default) and/or `vecq`"
);

use crate::Error;
use fs2::FileExt;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Per-operation saves are deferred until this many mutations pile up (#1322).
const SAVE_EVERY_OPS: u32 = 64;
/// ...or until the last save is this old, whichever comes first.
const SAVE_MAX_AGE: Duration = Duration::from_secs(2);

#[cfg(feature = "usearch")]
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};
#[cfg(feature = "vecq")]
use vecq_core::VecqIndex;

/// Extension for the on-disk index file of the compiled-in DEFAULT backend.
///
/// Prefer [`index_ext_for`] when a specific backend is selected at runtime.
#[cfg(feature = "usearch")]
pub const INDEX_EXT: &str = "usearch";
#[cfg(not(feature = "usearch"))]
pub const INDEX_EXT: &str = "vecq";

/// Default dimensions for EmbeddingGemma Q4 (768d).
const DEFAULT_DIMS: usize = 768;

/// Deterministic vecq seed. vecq results are bit-identical across platforms
/// for a given seed + file; pinning the seed keeps rebuilds reproducible.
#[cfg(feature = "vecq")]
const VECQ_SEED: u64 = 0x7574_656b; // "utek"

/// Which vector engine a `VectorIndex` uses (#1168).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorBackend {
    Usearch,
    Vecq,
}

impl VectorBackend {
    /// Parse a user-facing backend name (env var / toml value).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "usearch" => Some(VectorBackend::Usearch),
            "vecq" => Some(VectorBackend::Vecq),
            _ => None,
        }
    }

    /// The compiled-in default engine: usearch when available (long-standing
    /// desktop default, best query latency), else vecq (slim builds).
    pub fn default_backend() -> Self {
        default_backend()
    }

    /// Resolve a preferred backend against the engines compiled in.
    ///
    /// Returns `(resolved, honored)` where `honored` is false when the
    /// preference had to fall back (engine not compiled in), or when the
    /// preference was `None`.
    pub fn resolve(preferred: Option<Self>) -> (Self, bool) {
        let default = default_backend();
        match preferred {
            None => (default, false),
            Some(p) if p.is_compiled_in() => (p, true),
            Some(_p) => (default, false),
        }
    }

    /// Whether this engine's cargo feature is compiled into the binary.
    pub fn is_compiled_in(self) -> bool {
        match self {
            #[cfg(feature = "usearch")]
            VectorBackend::Usearch => true,
            #[cfg(not(feature = "usearch"))]
            VectorBackend::Usearch => false,
            #[cfg(feature = "vecq")]
            VectorBackend::Vecq => true,
            #[cfg(not(feature = "vecq"))]
            VectorBackend::Vecq => false,
        }
    }
}

/// Compiled-in default engine (see [`VectorBackend::default_backend`]).
pub fn default_backend() -> VectorBackend {
    #[cfg(feature = "usearch")]
    {
        VectorBackend::Usearch
    }
    #[cfg(not(feature = "usearch"))]
    {
        VectorBackend::Vecq
    }
}

/// On-disk index file extension for a backend.
pub fn index_ext_for(backend: VectorBackend) -> &'static str {
    match backend {
        VectorBackend::Usearch => "usearch",
        VectorBackend::Vecq => "vecq",
    }
}

/// The concrete engine instance — one variant per compiled-in backend.
#[allow(clippy::large_enum_variant)]
enum Engine {
    #[cfg(feature = "usearch")]
    Usearch(Box<Index>),
    #[cfg(feature = "vecq")]
    Vecq(VecqIndex),
}

impl Engine {
    /// Create an empty engine of `backend` for `dims`-dimensional vectors.
    fn create(backend: VectorBackend, dims: usize) -> Result<Self, Error> {
        match backend {
            #[cfg(feature = "usearch")]
            VectorBackend::Usearch => {
                let options = IndexOptions {
                    dimensions: dims,
                    metric: MetricKind::Cos,
                    quantization: ScalarKind::F32,
                    ..Default::default()
                };
                let index = Index::new(&options).map_err(|e| {
                    Error::embed_msg(format!(
                        "Failed to create usearch index (dims={dims}): {e}. This is likely an out-of-memory condition."
                    ))
                })?;
                Ok(Engine::Usearch(Box::new(index)))
            }
            #[cfg(not(feature = "usearch"))]
            VectorBackend::Usearch => Err(Error::validation(
                "usearch engine requested but not compiled in",
            )),
            #[cfg(feature = "vecq")]
            VectorBackend::Vecq => Ok(Engine::Vecq(VecqIndex::with_residual(dims, VECQ_SEED))),
            #[cfg(not(feature = "vecq"))]
            VectorBackend::Vecq => Err(Error::validation(
                "vecq engine requested but not compiled in",
            )),
        }
    }

    /// Restore an engine from a serialized buffer. The buffer must have been
    /// written by the SAME engine (`Engine::to_bytes`).
    fn from_bytes(backend: VectorBackend, buffer: &[u8]) -> Result<Self, Error> {
        match backend {
            #[cfg(feature = "usearch")]
            VectorBackend::Usearch => {
                let index = Index::restore_from_buffer(buffer)
                    .map_err(|e| Error::embed("load vector index", e))?;
                Ok(Engine::Usearch(Box::new(index)))
            }
            #[cfg(not(feature = "usearch"))]
            VectorBackend::Usearch => Err(Error::validation(
                "usearch engine requested but not compiled in",
            )),
            #[cfg(feature = "vecq")]
            VectorBackend::Vecq => {
                let index = VecqIndex::from_bytes(buffer)
                    .map_err(|e| Error::embed("load vector index (vecq)", e))?;
                Ok(Engine::Vecq(index))
            }
            #[cfg(not(feature = "vecq"))]
            VectorBackend::Vecq => Err(Error::validation(
                "vecq engine requested but not compiled in",
            )),
        }
    }

    /// Serialize to an in-memory buffer (see save() notes on buffer-based I/O).
    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => {
                let buf_len = index.serialized_length();
                let mut buffer = vec![0u8; buf_len];
                index
                    .save_to_buffer(&mut buffer)
                    .map_err(|e| Error::embed("save vector index to buffer", e))?;
                Ok(buffer)
            }
            #[cfg(feature = "vecq")]
            Engine::Vecq(index) => Ok(index.to_bytes()),
        }
    }

    /// Number of physical rows/entries in the engine (including vecq
    /// tombstoned rows; live count is the key map's length).
    fn len(&self) -> usize {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => index.size(),
            #[cfg(feature = "vecq")]
            Engine::Vecq(index) => index.len(),
        }
    }

    /// Capacity (diagnostics).
    fn capacity(&self) -> usize {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => index.capacity(),
            #[cfg(feature = "vecq")]
            Engine::Vecq(index) => index.len(),
        }
    }

    /// Embedding dimensionality.
    fn dims(&self) -> usize {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => index.dimensions(),
            #[cfg(feature = "vecq")]
            Engine::Vecq(index) => index.dim(),
        }
    }

    /// Pre-reserve capacity for a bulk build (usearch only; vecq grows
    /// organically).
    fn reserve(&mut self, #[allow(unused_variables)] additional: usize) {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => {
                if let Err(e) = index.reserve(additional) {
                    tracing::error!("Failed to reserve usearch capacity: {e}");
                }
            }
            #[cfg(feature = "vecq")]
            Engine::Vecq(_) => { /* vecq: no-op */ }
        }
    }

    /// Auto-grow capacity when full (usearch only).
    fn ensure_capacity(&mut self) -> Result<(), Error> {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => {
                if index.size() >= index.capacity() {
                    // Auto-reserve using geometric growth to amortize reallocation cost.
                    // Growth strategy: max(current * 2, current + 4096, 1024).
                    let current = index.capacity();
                    let new_cap = (current * 2).max(current + 4096).max(1024);
                    index.reserve(new_cap).map_err(|e| {
                        Error::embed_msg(format!("Failed to reserve usearch capacity: {e}"))
                    })?;
                }
                Ok(())
            }
            #[cfg(feature = "vecq")]
            Engine::Vecq(_) => Ok(()),
        }
    }

    /// Insert a vector under `key`.
    ///
    /// usearch: keyed insert. vecq: append-only — the row index MUST equal
    /// `key` (the caller maintains the row == key invariant by deriving keys
    /// from `Engine::len()`), the key argument is ignored.
    fn add(&mut self, #[allow(unused_variables)] key: u64, embedding: &[f32]) -> Result<(), Error> {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => index
                .add(key, embedding)
                .map_err(|e| Error::embed_msg(format!("Failed to insert into usearch index: {e}"))),
            #[cfg(feature = "vecq")]
            Engine::Vecq(index) => {
                index.add(embedding);
                Ok(())
            }
        }
    }

    /// Remove a key (usearch). vecq has no incremental delete — the caller
    /// tombstones via the key map instead.
    fn remove(&mut self, #[allow(unused_variables)] key: u64) {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => {
                if let Err(e) = index.remove(key) {
                    tracing::error!("Failed to remove from usearch index: {e}");
                }
            }
            #[cfg(feature = "vecq")]
            Engine::Vecq(_) => { /* tombstone via key map (see remove()) */ }
        }
    }

    /// Top-`k` search. Returns `(key, cosine_distance)` pairs sorted by
    /// distance ascending, engine-agnostic (vecq rows ARE keys).
    fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        match self {
            #[cfg(feature = "usearch")]
            Engine::Usearch(index) => match index.search(query, k) {
                Ok(r) => r
                    .keys
                    .iter()
                    .copied()
                    .zip(r.distances.iter().copied())
                    .collect(),
                Err(e) => {
                    tracing::error!("usearch search failed: {e}");
                    Vec::new()
                }
            },
            #[cfg(feature = "vecq")]
            Engine::Vecq(index) => index
                .search(query, k)
                .into_iter()
                .map(|(row, sim)| (row as u64, 1.0 - sim))
                .collect(),
        }
    }
}

/// Persistent vector index.
///
/// - **Startup**: loads from disk (~5ms), no rebuild needed
/// - **Insert**: incremental, no rebuild
/// - **Delete**: incremental, no rebuild (vecq: tombstone)
/// - **Save**: persists to disk after mutations
///
/// **Cross-process safety (#543):** An exclusive file lock on the index
/// file serializes concurrent access from separate CLI processes. The lock is
/// held for the lifetime of the VectorIndex. In-process thread safety uses
/// `RwLock` in `Uteke`.
pub struct VectorIndex {
    /// Active engine (runtime-selected, #1168).
    engine: Engine,
    /// Which backend `engine` is (mirrors the enum variant; kept separately
    /// for cheap comparisons without matching).
    backend: VectorBackend,
    /// Maps integer key (u64) → memory UUID string.
    key_to_id: HashMap<u64, String>,
    /// Maps memory UUID → integer key.
    id_to_key: HashMap<String, u64>,
    /// Next available integer key.
    next_key: u64,
    /// Path to the index file.
    path: Option<PathBuf>,
    /// Whether the index has unsaved changes.
    dirty: bool,
    /// Mutations since the last successful save (#1322).
    unsaved_ops: u32,
    /// When the index was last written to disk (or loaded/created).
    last_saved: Instant,
    /// Successful `save()` calls over this instance's lifetime (diagnostics, tests).
    saves: u64,
    /// Cross-process file lock on the index file (#543).
    /// Held until the VectorIndex is dropped.
    _lock_file: Option<File>,
}

impl VectorIndex {
    /// The engine this index runs on (#1168).
    pub fn backend(&self) -> VectorBackend {
        self.backend
    }

    /// Create a new empty vector index using the compiled-in default engine.
    pub fn new(dims: usize) -> Result<Self, Error> {
        Self::with_backend(default_backend(), dims)
    }

    /// Create a new empty vector index for an explicit engine (#1168).
    pub fn with_backend(backend: VectorBackend, dims: usize) -> Result<Self, Error> {
        let engine = Engine::create(backend, dims)?;
        Ok(Self {
            engine,
            backend,
            key_to_id: HashMap::new(),
            id_to_key: HashMap::new(),
            next_key: 0,
            path: None,
            dirty: false,
            unsaved_ops: 0,
            last_saved: Instant::now(),
            saves: 0,
            _lock_file: None,
        })
    }

    /// Load index from disk, or create empty if file doesn't exist.
    /// `path` is the path to the index file. The engine is inferred from the
    /// file extension (`uteke_index.usearch` / `uteke_index.vecq`).
    ///
    /// Acquires an **exclusive file lock** on the index file to prevent
    /// cross-process race conditions (e.g., `xargs -P5 uteke remember`).
    /// The lock is held until this `VectorIndex` is dropped (#543).
    ///
    /// Note: Both `save()` and `load()` use buffer-based serialization (#647,
    /// #684) to bypass usearch's C++ file I/O on Windows. The on-disk format is
    /// identical — `save_to_buffer` and `restore_from_buffer` produce/consume
    /// the same byte stream as the native file-based methods.
    pub fn load_or_create(path: &Path, dims: usize) -> Result<Self, Error> {
        // The extension determines which engine reads the file (#1112 kept
        // per-engine extensions for exactly this purpose).
        let backend = match path.extension().and_then(|e| e.to_str()) {
            Some("vecq") => VectorBackend::Vecq,
            _ => VectorBackend::Usearch,
        };
        Self::load_or_create_for(path, dims, backend)
    }

    /// [`load_or_create`] with an explicit engine (#1168). Use when the path
    /// may not carry a backend-specific extension.
    pub fn load_or_create_for(
        path: &Path,
        dims: usize,
        backend: VectorBackend,
    ) -> Result<Self, Error> {
        // Atomically create the file if it doesn't exist (avoids TOCTOU race
        // where another process creates the file between our exists() and write()).
        // O_CREAT | O_EXCL ensures only one writer wins; failure is harmless.
        use std::fs::OpenOptions;
        let _ = OpenOptions::new().create_new(true).write(true).open(path);
        // Regardless of who created it, the file now exists — open + lock it.
        let mut lock_file = acquire_file_lock(path)?;

        let mut idx = if lock_file
            .metadata()
            .map_err(|e| Error::embed("read file metadata", e))?
            .len()
            == 0
        {
            Self::with_backend(backend, dims)?
        } else {
            Self::load_from_file_for(&mut lock_file, path, backend)?
        };
        idx.path = Some(path.to_path_buf());
        idx._lock_file = Some(lock_file);
        Ok(idx)
    }

    /// Load an existing index from an already-open file handle.
    ///
    /// This prevents `ERROR_LOCK_VIOLATION` (os error 33) on Windows when the
    /// file is locked exclusively by the same process (#732), since we read
    /// directly from the locked handle instead of opening a second one.
    pub fn load_from_file(file: &mut File, path: &Path) -> Result<Self, Error> {
        let backend = match path.extension().and_then(|e| e.to_str()) {
            Some("vecq") => VectorBackend::Vecq,
            _ => VectorBackend::Usearch,
        };
        Self::load_from_file_for(file, path, backend)
    }

    /// [`load_from_file`] with an explicit engine (#1168).
    pub fn load_from_file_for(
        file: &mut File,
        path: &Path,
        backend: VectorBackend,
    ) -> Result<Self, Error> {
        use std::io::{Read, Seek, SeekFrom};

        file.seek(SeekFrom::Start(0))
            .map_err(|e| Error::embed("seek index file", e))?;

        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .map_err(|e| Error::embed("read index file from locked handle", e))?;

        let engine = Engine::from_bytes(backend, &buffer)?;
        // Legacy usearch-only builds: touch size() to validate the restored
        // handle (kept from the pre-#1168 load path). On builds with both
        // engines the validation already happened inside from_bytes.
        #[cfg(all(feature = "usearch", not(feature = "vecq")))]
        let engine = match engine {
            Engine::Usearch(idx) => {
                let _ = idx.size();
                Engine::Usearch(idx)
            }
        };

        // Rebuild key mappings from the sidecar file
        let mut key_to_id = HashMap::new();
        let mut id_to_key = HashMap::new();
        let mut next_key = 0u64;

        let mapping_path = path.with_extension("keys");
        match std::fs::read_to_string(&mapping_path) {
            Ok(data) => {
                for line in data.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Some((key_str, id)) = line.split_once('\t') {
                        if let Ok(key) = key_str.parse::<u64>() {
                            key_to_id.insert(key, id.to_string());
                            id_to_key.insert(id.to_string(), key);
                            next_key = next_key.max(key.saturating_add(1));
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // No key mapping sidecar — fresh index, start from key 0.
            }
            Err(e) => return Err(Error::embed("read key mapping", e)),
        }

        Ok(Self {
            engine,
            backend,
            key_to_id,
            id_to_key,
            next_key,
            path: None,
            dirty: false,
            unsaved_ops: 0,
            last_saved: Instant::now(),
            saves: 0,
            _lock_file: None,
        })
    }

    /// Load an existing index from disk.
    ///
    /// Uses buffer-based deserialization (#684): reads the file into memory via
    /// Rust's `std::fs::read()`, then deserializes via the backend's
    /// from-buffer API. This bypasses usearch's C++ `fopen("rb")` + `mmap()`
    /// which causes "Permission denied" errors on Windows (#684).
    pub fn load(path: &Path) -> Result<Self, Error> {
        let mut file = File::open(path).map_err(|e| Error::embed("open index file", e))?;
        Self::load_from_file(&mut file, path)
    }

    /// Save index and key mappings to disk.
    ///
    /// Uses buffer-based serialization (#647): serializes the index into an
    /// in-memory buffer, then writes the buffer to disk using Rust's
    /// `std::fs` with atomic write (temp file + rename).
    ///
    /// This bypasses usearch's C++ `fopen("wb")` file I/O which has known
    /// issues on Windows:
    /// - `fopen` fails silently on paths > 260 chars (MAX_PATH)
    /// - `fopen("wb")` exclusive access conflicts with fs2 exclusive lock
    /// - Windows Defender can intercept `fwrite` calls
    ///
    /// The in-memory buffer approach is safe because:
    /// - The index data is already fully in RAM (both backends load fully)
    /// - Atomic write prevents corruption on crash
    pub fn save(&mut self) -> Result<(), Error> {
        if let Some(ref path) = self.path {
            // Serialize index to in-memory buffer, bypassing C++ file I/O (#647)
            let buffer = self.engine.to_bytes()?;

            // Write buffer to disk via atomic write (temp file + rename)
            let tmp_path = path.with_extension(format!("{}.tmp", index_ext_for(self.backend)));
            // Windows: the existing locked handle is refreshed after the rename
            // (below). Elsewhere the rename replaces the inode our flock lives
            // on, so lock the NEW file before it is published: the exclusive
            // lock then follows the inode onto `path` with no unlocked window,
            // and a second process can never lock a different inode.
            #[cfg(windows)]
            write_synced(&tmp_path, &buffer)
                .map_err(|e| Error::embed("write temp index file", e))?;
            #[cfg(not(windows))]
            let new_lock = {
                use std::io::Write;
                let mut f = File::options()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&tmp_path)
                    .map_err(|e| Error::embed("create temp index file", e))?;
                f.try_lock_exclusive()
                    .map_err(|e| Error::embed("lock temp index file", e))?;
                f.write_all(&buffer)
                    .and_then(|()| f.sync_all())
                    .map_err(|e| Error::embed("write temp index file", e))?;
                f
            };

            // On Windows, `std::fs::rename` fails with `ERROR_ACCESS_DENIED` if
            // the destination file is locked by `LockFileEx` via fs2 (#926).
            // Instead of releasing the lock (which creates a race window #982),
            // retry the rename with exponential backoff — Windows often releases
            // the lock momentarily between operations.
            #[cfg(windows)]
            {
                let mut delay = std::time::Duration::from_millis(10);
                let max_delay = std::time::Duration::from_millis(640);
                loop {
                    match std::fs::rename(&tmp_path, path) {
                        Ok(()) => break,
                        Err(e) if e.raw_os_error() == Some(5) => {
                            // ERROR_ACCESS_DENIED — retry after backoff
                            if delay > max_delay {
                                return Err(Error::embed_msg(format!(
                                    "rename timeout (ACCESS_DENIED) after retries: {e}"
                                )));
                            }
                            std::thread::sleep(delay);
                            delay *= 2;
                        }
                        Err(e) => {
                            return Err(Error::embed("rename temp to final index file", e));
                        }
                    }
                }
            }
            #[cfg(not(windows))]
            {
                std::fs::rename(&tmp_path, path)
                    .map_err(|e| Error::embed("rename temp to final index file", e))?;
                // The old (now orphaned) inode's lock is released when the
                // previous handle drops; the new handle holds the live lock.
                self._lock_file = Some(new_lock);
            }

            // On Windows, reopen the file after rename to refresh the lock
            // handle — it now points to the new file written via rename.
            #[cfg(windows)]
            {
                if let Some(ref mut lock_file) = self._lock_file {
                    let new_file = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(path)
                        .map_err(|e| {
                            Error::embed_msg(format!(
                                "Failed to reopen index file after save rename: {e}"
                            ))
                        })?;
                    fs2::FileExt::try_lock_exclusive(&new_file)
                        .map_err(|e| Error::embed("re-lock index file after save", e))?;
                    *lock_file = new_file;
                }
            }

            // Save key→id mapping as sidecar file using atomic write
            let mapping_path = path.with_extension("keys");
            let mut lines = Vec::new();
            for (&key, id) in &self.key_to_id {
                lines.push(format!("{key}\t{id}"));
            }
            atomic_write(&mapping_path, lines.join("\n").as_bytes())?;

            self.dirty = false;
            self.unsaved_ops = 0;
            self.last_saved = Instant::now();
            self.saves += 1;
        }
        Ok(())
    }

    /// Save only when enough has piled up since the last save (#1322).
    ///
    /// `save()` rewrites the whole index and the whole `.keys` sidecar, so
    /// saving on every write makes bulk ingest O(N²). Per-operation paths call
    /// this instead: it writes once `SAVE_EVERY_OPS` mutations or
    /// `SAVE_MAX_AGE` have accumulated. Nothing is lost for good in between —
    /// SQLite already holds every row, and `shutdown()`, `Drop`, and the
    /// server's periodic flush write whatever is still pending. After a hard
    /// kill the on-disk index can trail SQLite by up to that window;
    /// `uteke verify` / `repair` resync it.
    pub fn save_if_due(&mut self) -> Result<(), Error> {
        if !self.dirty {
            return Ok(());
        }
        if self.unsaved_ops >= SAVE_EVERY_OPS || self.last_saved.elapsed() >= SAVE_MAX_AGE {
            return self.save();
        }
        Ok(())
    }

    /// Build the index from a list of (id, embedding) pairs.
    /// Used for migration from old HNSW or full rebuild.
    pub fn build(&mut self, items: &[(String, Vec<f32>)]) -> Result<(), Error> {
        // Reset (same engine, fresh instance)
        let dims = if items.is_empty() {
            DEFAULT_DIMS
        } else {
            items[0].1.len()
        };
        self.engine = Engine::create(self.backend, dims)?;
        self.key_to_id.clear();
        self.id_to_key.clear();
        self.next_key = 0;

        if !items.is_empty() {
            // Validate all items have consistent dimensions
            for (id, emb) in items {
                if emb.len() != dims {
                    return Err(Error::validation(format!(
                        "embedding dimension mismatch in build(): item '{id}' has {} dims, expected {dims}",
                        emb.len()
                    )));
                }
            }
            self.engine.reserve(items.len());
        }

        for (id, embedding) in items {
            self.insert(id, embedding)?;
        }
        Ok(())
    }

    /// Insert a single item into the index.
    /// If the ID already exists, removes the old entry first to prevent duplicates.
    /// Returns error if the underlying index operation fails.
    pub fn insert(&mut self, id: &str, embedding: &[f32]) -> Result<(), Error> {
        // Validate dimensions up front, before any map mutation, so error
        // paths leave the key maps consistent with the physical index.
        if embedding.len() != self.engine.dims() {
            return Err(Error::validation(format!(
                "embedding dimension mismatch: got {}, expected {}",
                embedding.len(),
                self.engine.dims()
            )));
        }

        let key = if self.backend == VectorBackend::Vecq {
            // vecq rows are append-only: the new entry lands at physical row
            // `len()`, and search maps rows back via key_to_id — so the
            // key MUST equal that row exactly (no gaps).
            //
            // Crash window: save() writes the index file and the `.keys`
            // sidecar as two separate atomic writes. If the process dies after
            // the sidecar but before the index, the reloaded sidecar can hold
            // keys ≥ len() ("phantom" keys pointing at rows that were
            // never written). Overwriting such a phantom key here is correct:
            // its row never existed, so nothing live can be shadowed.
            let key = self.engine.len() as u64;
            if let Some(phantom_id) = self.key_to_id.remove(&key) {
                tracing::warn!(
                    "overwriting phantom key {key} (id '{phantom_id}' had no row in the vecq index — likely a crash between sidecar and index writes)"
                );
                self.id_to_key.remove(&phantom_id);
            }
            key
        } else {
            // Auto-grow usearch capacity when full (fallible — runs before any
            // state is touched so a failure leaves the index unchanged).
            self.engine.ensure_capacity()?;
            let key = self.next_key;
            self.next_key = self.next_key.saturating_add(1);
            key
        };

        // vecq assigns rows sequentially; row == key by construction.
        // Add first: if it fails, the previous entry for `id` is still intact.
        self.engine.add(key, embedding)?;

        // Replace the old entry only once the new vector is in the engine.
        if let Some(old_key) = self.id_to_key.get(id).copied() {
            self.key_to_id.remove(&old_key);
            self.engine.remove(old_key);
            // vecq has no incremental delete — the dead row is filtered out of
            // search via the key map (key_to_id no longer contains old_key).
        }
        self.key_to_id.insert(key, id.to_string());
        self.id_to_key.insert(id.to_string(), key);

        self.dirty = true;
        self.unsaved_ops = self.unsaved_ops.saturating_add(1);
        Ok(())
    }

    /// Remove an item by memory ID. Incremental — no rebuild.
    pub fn remove(&mut self, id: &str) -> bool {
        if let Some(key) = self.id_to_key.remove(id) {
            self.key_to_id.remove(&key);
            self.engine.remove(key);
            // vecq: tombstone is implicit — the key vanishes from the map, so
            // search results referencing that row are filtered out below.
            self.dirty = true;
            self.unsaved_ops = self.unsaved_ops.saturating_add(1);
            true
        } else {
            false
        }
    }

    /// Search for the k nearest neighbors of the query vector.
    /// Returns (memory_id, distance_f32) pairs, sorted by distance ascending.
    /// Note: `ef` parameter is accepted for API compatibility but not passed to
    /// usearch v2.25.3 (Rust bindings don't expose `ef` in `search()`).
    pub fn search(&self, query: &[f32], k: usize, _ef: usize) -> Vec<(String, f32)> {
        if self.is_empty() {
            return Vec::new();
        }

        let count = k.max(1);

        // vecq: tombstoned rows (physically present but absent from the key
        // map) are filtered out below — over-fetch by the exact dead-row
        // count so we still return up to `k` live results. usearch: len() ==
        // live size, dead = 0.
        let dead = self.engine.len().saturating_sub(self.key_to_id.len());
        let results: Vec<(String, f32)> = self
            .engine
            .search(query, count + dead)
            .into_iter()
            .filter_map(|(key, dist)| self.key_to_id.get(&key).map(|id| (id.clone(), dist)))
            .collect();
        // Tombstones may shrink results below k; over-fetch above mitigates
        // this. Cap at k and keep ascending-distance order.
        let mut results = results;
        results.truncate(count);
        results
    }

    /// Number of live (non-tombstoned) items in the index.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        #[cfg(feature = "usearch")]
        if self.backend == VectorBackend::Usearch {
            return self.key_to_id.len().max(self.engine.len());
        }
        self.key_to_id.len()
    }

    /// Capacity of the underlying index (diagnostics).
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.engine.capacity()
    }

    /// Embedding dimensionality of this index.
    ///
    /// Used by backend dispatch to detect dim mismatch when the user swaps
    /// embedding backends on an existing store (#337).
    pub fn dims(&self) -> usize {
        self.engine.dims()
    }

    /// Check if the index is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Successful saves over this instance's lifetime (diagnostics, tests).
    #[allow(dead_code)]
    pub(crate) fn save_count(&self) -> u64 {
        self.saves
    }

    /// Whether the index has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// Flush pending mutations when the index goes away, so a process that exits
/// without calling `shutdown()` (CLI, MCP stdio) still persists its writes.
impl Drop for VectorIndex {
    fn drop(&mut self) {
        if self.dirty && self.path.is_some() {
            if let Err(e) = self.save() {
                tracing::warn!("Failed to save vector index on drop: {e}");
            }
        }
    }
}

impl Default for VectorIndex {
    fn default() -> Self {
        Self::new(DEFAULT_DIMS).expect("Failed to create default vector index")
    }
}

/// Convert cosine distance (0..2) to cosine similarity (0..1).
/// Both backends return cosine *distance* (1 - similarity) from `search()`.
pub fn cosine_distance_to_similarity(distance: f32) -> f32 {
    // cosine distance = 1 - cosine_similarity
    let sim = 1.0 - distance;
    sim.clamp(0.0, 1.0)
}

/// Acquire an exclusive file lock on the index file (#543).
///
/// Retry loop with timeout: tries `try_lock_exclusive` every 200ms up to 30s,
/// then fails with a helpful diagnostic. This prevents indefinite blocking
/// on Windows where `lock_exclusive` has no built-in timeout (#922).
fn acquire_file_lock(path: &Path) -> Result<File, Error> {
    let file = File::options()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| Error::embed_msg(format!("Failed to open index file for locking: {e}")))?;

    // Fast path: try non-blocking lock first.
    if file.try_lock_exclusive().is_ok() {
        tracing::debug!("index file lock acquired: {}", path.display());
        return Ok(file);
    }

    // Contended: retry with timeout (#922).
    // On Linux, flock(LOCK_EX) would block indefinitely anyway, so the retry
    // loop adds a bounded timeout on all platforms. On Windows,
    // LockFileEx(LOCKFILE_EXCLUSIVE_LOCK) also blocks forever — the retry loop
    // with try_lock_exclusive gives us control over the timeout.
    tracing::debug!(
        "index file lock busy on {}, retrying with timeout...",
        path.display()
    );

    const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);
    const MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
    let deadline = std::time::Instant::now() + MAX_WAIT;
    let mut waited = std::time::Duration::ZERO;

    loop {
        std::thread::sleep(RETRY_INTERVAL);
        waited += RETRY_INTERVAL;

        if file.try_lock_exclusive().is_ok() {
            tracing::debug!(
                "index file lock acquired (after {:?}): {}",
                waited,
                path.display()
            );
            return Ok(file);
        }

        if std::time::Instant::now() >= deadline {
            return Err(Error::Lock {
                context: format!(
                    "Could not acquire lock on {} after {MAX_WAIT:?}. \
                     Another uteke process (uteke-serve or CLI) may be running. \
                     Stop it and retry.",
                    path.display()
                ),
            });
        }

        tracing::trace!(
            "index file lock still busy after {:?}: {}",
            waited,
            path.display()
        );
    }
}

/// Write `data` to `path` and fsync before returning, so a following rename
/// can never publish a zero-length or partial file after power loss.
fn write_synced(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    f.write_all(data)?;
    f.sync_all()
}

/// Atomic file write: write to temp file then rename.
/// Prevents corruption if process crashes mid-write.
/// POSIX guarantees rename() is atomic on the same filesystem.
fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<(), Error> {
    let tmp_path = path.with_extension("keys.tmp");
    write_synced(&tmp_path, data).map_err(|e| Error::embed("write temp key mapping", e))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|e| Error::embed("rename temp to final key mapping", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vec(dims: usize, idx: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; dims];
        if idx < dims {
            v[idx] = 1.0;
        }
        v
    }

    #[test]
    fn test_empty_index() {
        let idx = VectorIndex::new(768).unwrap();
        assert!(idx.is_empty());
        let results = idx.search(&[0.0; 768], 5, 50);
        assert!(results.is_empty());
    }

    #[test]
    fn test_insert_and_search() {
        let mut idx = VectorIndex::new(768).unwrap();

        let v1 = make_vec(768, 0);
        let v2 = make_vec(768, 1);
        let mut v3 = vec![0.0f32; 768];
        v3[0] = 0.9;
        v3[1] = 0.1;
        let norm = v3.iter().map(|x| x * x).sum::<f32>().sqrt();
        v3.iter_mut().for_each(|x| *x /= norm);

        idx.insert("m1", &v1).unwrap();
        idx.insert("m2", &v2).unwrap();
        idx.insert("m3", &v3).unwrap();

        assert_eq!(idx.len(), 3);

        let results = idx.search(&v1, 3, 50);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "m1");
    }

    #[test]
    fn test_remove() {
        let mut idx = VectorIndex::new(768).unwrap();

        let v1 = make_vec(768, 0);
        let v2 = make_vec(768, 1);

        idx.insert("m1", &v1).unwrap();
        idx.insert("m2", &v2).unwrap();

        assert_eq!(idx.len(), 2);

        // Remove m1 — no rebuild needed
        assert!(idx.remove("m1"));
        assert_eq!(idx.len(), 1);

        // Search should only return m2
        let results = idx.search(&v1, 5, 50);
        assert!(results.iter().all(|(id, _)| id != "m1"));
    }

    /// Regression: after `save()` renames a new file over the index, the
    /// exclusive lock must live on the file now at `path` (not the orphaned
    /// inode), so a second opener cannot lock it.
    #[cfg(unix)]
    #[test]
    fn test_save_keeps_exclusive_lock_on_published_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("lock.{INDEX_EXT}"));
        let mut idx = VectorIndex::load_or_create(&path, 8).unwrap();
        idx.insert("a", &make_vec(8, 0)).unwrap();
        idx.save().unwrap();
        idx.insert("b", &make_vec(8, 1)).unwrap();
        idx.save().unwrap(); // second save: lock must survive repeated renames

        let other = File::options().read(true).write(true).open(&path).unwrap();
        assert!(
            other.try_lock_exclusive().is_err(),
            "published index file must still be exclusively locked by its owner"
        );
        drop(idx);
        let other = File::options().read(true).write(true).open(&path).unwrap();
        assert!(other.try_lock_exclusive().is_ok(), "lock released on drop");
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("test.{INDEX_EXT}"));

        // Create and insert
        let mut idx = VectorIndex::new(64).unwrap();
        idx.path = Some(path.clone());

        let v1: Vec<f32> = {
            let mut v = vec![0.0f32; 64];
            v[0] = 1.0;
            v
        };
        let v2: Vec<f32> = {
            let mut v = vec![0.0f32; 64];
            v[1] = 1.0;
            v
        };

        idx.insert("mem-1", &v1).unwrap();
        idx.insert("mem-2", &v2).unwrap();
        idx.save().unwrap();

        // Verify on-disk files are non-empty (#647 regression)
        assert!(
            path.metadata().unwrap().len() > 0,
            "index file must not be 0 bytes"
        );
        let keys_path = path.with_extension("keys");
        assert!(
            keys_path.metadata().unwrap().len() > 0,
            ".keys file must not be 0 bytes"
        );

        // Load from disk — must work because buffer format == file format
        let loaded = VectorIndex::load(&path).unwrap();
        assert_eq!(loaded.len(), 2);

        // Search on loaded index
        let results = loaded.search(&v1, 5, 50);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "mem-1");
    }

    #[test]
    fn test_save_buffer_produces_valid_index() {
        // Round-trip test: save via buffer → load via buffer (#647, #684)
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("roundtrip.{INDEX_EXT}"));

        let mut idx = VectorIndex::new(32).unwrap();
        idx.path = Some(path.clone());

        let v: Vec<f32> = {
            let mut v = vec![0.0f32; 32];
            v[5] = 1.0;
            v
        };
        idx.insert("round-1", &v).unwrap();
        idx.save().unwrap();

        // Verify the saved file is loadable by the backend's buffer API
        let buffer = std::fs::read(&path).unwrap();
        #[cfg(feature = "usearch")]
        if idx.backend() == VectorBackend::Usearch {
            let raw_index = usearch::Index::restore_from_buffer(&buffer);
            assert!(
                raw_index.is_ok(),
                "Buffer-saved index must be loadable by usearch restore_from_buffer"
            );
            assert_eq!(raw_index.unwrap().size(), 1);
        }
        #[cfg(feature = "vecq")]
        if idx.backend() == VectorBackend::Vecq {
            let raw_index = VecqIndex::from_bytes(&buffer);
            assert!(
                raw_index.is_ok(),
                "Buffer-saved index must be loadable by VecqIndex::from_bytes"
            );
            assert_eq!(raw_index.unwrap().len(), 1);
        }
    }

    #[test]
    fn test_build_from_items() {
        let items: Vec<(String, Vec<f32>)> = (0..10)
            .map(|i| {
                let mut v = vec![0.0f32; 768];
                v[i] = 1.0;
                (format!("item-{i}"), v)
            })
            .collect();

        let mut idx = VectorIndex::new(768).unwrap();
        idx.build(&items).unwrap();
        assert_eq!(idx.len(), 10);

        let query = make_vec(768, 0);
        let results = idx.search(&query, 3, 50);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "item-0");
    }

    #[cfg(feature = "vecq")]
    #[test]
    fn test_vecq_tombstones_filter_removed_rows() {
        // Removed IDs must never appear in search results (vecq has no
        // incremental delete — rows are tombstoned via the key map, #1098).
        let mut idx = VectorIndex::with_backend(VectorBackend::Vecq, 64).unwrap();
        let v1 = make_vec(64, 0);
        let v2 = make_vec(64, 1);

        idx.insert("alive", &v1).unwrap();
        idx.insert("dead", &v2).unwrap();
        assert!(idx.remove("dead"));

        let results = idx.search(&v2, 2, 50);
        assert!(results.iter().all(|(id, _)| id != "dead"));
        assert!(results.iter().any(|(id, _)| id == "alive"));
    }

    /// #1168: runtime selection metadata.
    #[test]
    fn test_backend_selection_metadata() {
        // default_backend is one of the compiled-in engines
        assert!(VectorBackend::default_backend().is_compiled_in());

        // resolve: None → default, not honored
        let (resolved, honored) = VectorBackend::resolve(None);
        assert_eq!(resolved, VectorBackend::default_backend());
        assert!(!honored);

        // resolve: compiled-in preference honored
        if VectorBackend::Vecq.is_compiled_in() {
            let (resolved, honored) = VectorBackend::resolve(Some(VectorBackend::Vecq));
            assert_eq!(resolved, VectorBackend::Vecq);
            assert!(honored);
        }
        if VectorBackend::Usearch.is_compiled_in() {
            let (resolved, honored) = VectorBackend::resolve(Some(VectorBackend::Usearch));
            assert_eq!(resolved, VectorBackend::Usearch);
            assert!(honored);
        }

        // parse: case-insensitive, whitespace tolerant
        assert_eq!(VectorBackend::parse(" vecq "), Some(VectorBackend::Vecq));
        assert_eq!(
            VectorBackend::parse("USEARCH"),
            Some(VectorBackend::Usearch)
        );
        assert_eq!(VectorBackend::parse("bogus"), None);
    }

    /// #1168: with_backend creates the requested engine (when compiled in).
    #[test]
    fn test_with_backend_creates_requested_engine() {
        if VectorBackend::Vecq.is_compiled_in() {
            let idx = VectorIndex::with_backend(VectorBackend::Vecq, 768).unwrap();
            assert_eq!(idx.backend(), VectorBackend::Vecq);
        }
        if VectorBackend::Usearch.is_compiled_in() {
            let idx = VectorIndex::with_backend(VectorBackend::Usearch, 768).unwrap();
            assert_eq!(idx.backend(), VectorBackend::Usearch);
        }
    }

    /// #1168 (dual-engine builds): each engine round-trips through its own
    /// per-extension file, and both files can coexist independently.
    #[cfg(all(feature = "usearch", feature = "vecq"))]
    #[test]
    fn test_dual_engine_roundtrip_per_extension() {
        let dir = tempfile::tempdir().unwrap();
        let v = make_vec(64, 3);

        for backend in [VectorBackend::Usearch, VectorBackend::Vecq] {
            let path = dir.path().join(format!("idx.{}", index_ext_for(backend)));
            let mut idx = VectorIndex::with_backend(backend, 64).unwrap();
            idx.path = Some(path.clone());
            idx.insert(&format!("mem-{backend:?}"), &v).unwrap();
            idx.save().unwrap();

            let loaded = VectorIndex::load(&path).unwrap();
            assert_eq!(loaded.backend(), backend, "ext must select the engine");
            assert_eq!(loaded.len(), 1);
            let results = loaded.search(&v, 1, 50);
            assert_eq!(results.len(), 1);
        }
    }

    // ── #1322: deferred saves ──────────────────────────────────────────────

    fn deferred_backends() -> Vec<VectorBackend> {
        [VectorBackend::Usearch, VectorBackend::Vecq]
            .into_iter()
            .filter(|b| b.is_compiled_in())
            .collect()
    }

    #[test]
    fn save_if_due_batches_writes_and_drop_flushes_the_rest() {
        for backend in deferred_backends() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(format!("idx.{}", index_ext_for(backend)));
            let mut idx = VectorIndex::with_backend(backend, 64).unwrap();
            idx.path = Some(path.clone());

            // 130 per-operation writes: a save at op 64 and 128, not 130 saves.
            for i in 0..130 {
                idx.insert(&format!("mem-{i}"), &make_vec(64, i % 64))
                    .unwrap();
                idx.save_if_due().unwrap();
            }
            assert_eq!(idx.save_count(), 2, "{backend:?}: expected batched saves");
            assert!(idx.is_dirty(), "{backend:?}: two ops are still pending");

            // Going away flushes the pending tail.
            drop(idx);
            let loaded = VectorIndex::load(&path).unwrap();
            assert_eq!(loaded.len(), 130, "{backend:?}: pending ops lost on drop");
        }
    }

    #[test]
    fn save_if_due_is_a_noop_when_clean() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(format!("idx.{}", index_ext_for(deferred_backends()[0])));
        let mut idx = VectorIndex::with_backend(deferred_backends()[0], 64).unwrap();
        idx.path = Some(path);
        idx.insert("mem-0", &make_vec(64, 0)).unwrap();
        idx.save().unwrap();
        let before = idx.save_count();
        idx.save_if_due().unwrap();
        assert_eq!(idx.save_count(), before);
    }
}
