//! Persistent vector index using usearch (HNSW with disk persistence).
//!
//! Cross-process safety (#543): Each VectorIndex acquires an exclusive file
//! lock (via fs2) on the .usearch file during construction. The lock is held
//! until the VectorIndex is dropped, serializing concurrent CLI invocations
//! that share the same on-disk index. In-process thread safety uses
//! RwLock<VectorIndex> in lib.rs.
//!
//! Windows compatibility (#647, #684): Both `save()` and `load()` use
//! buffer-based serialization to bypass usearch's C++ file I/O (`fopen`,
//! `fread`, `mmap`) which has Windows-specific issues (MAX_PATH, file lock
//! conflicts, AV interference). Save serializes to memory then atomic-writes
//! via Rust std::fs; load reads via Rust std::fs then deserializes from buffer.

use crate::Error;
use fs2::FileExt;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Extension for usearch index files.
const USEARCH_EXT: &str = "usearch";

/// Default dimensions for EmbeddingGemma Q4 (768d).
const DEFAULT_DIMS: usize = 768;

/// Persistent vector index backed by usearch.
///
/// - **Startup**: loads from disk (~5ms), no rebuild needed
/// - **Insert**: incremental, no rebuild
/// - **Delete**: incremental, no rebuild
/// - **Save**: persists to disk after mutations
///
/// **Cross-process safety (#543):** An exclusive file lock on the `.usearch`
/// file serializes concurrent access from separate CLI processes. The lock is
/// held for the lifetime of the VectorIndex. In-process thread safety uses
/// `RwLock` in `Uteke`.
pub struct VectorIndex {
    index: Index,
    /// Maps integer key (u64) → memory UUID string.
    key_to_id: HashMap<u64, String>,
    /// Maps memory UUID → integer key.
    id_to_key: HashMap<String, u64>,
    /// Next available integer key.
    next_key: u64,
    /// Path to the usearch index file.
    path: Option<PathBuf>,
    /// Whether the index has unsaved changes.
    dirty: bool,
    /// Cross-process file lock on the .usearch file (#543).
    /// Held until the VectorIndex is dropped.
    _lock_file: Option<File>,
}

impl VectorIndex {
    /// Create a new empty vector index.
    pub fn new(dims: usize) -> Result<Self, Error> {
        let index = Self::create_index(dims)?;
        Ok(Self {
            index,
            key_to_id: HashMap::new(),
            id_to_key: HashMap::new(),
            next_key: 0,
            path: None,
            dirty: false,
            _lock_file: None,
        })
    }

    /// Load index from disk, or create empty if file doesn't exist.
    /// `path` is the path to the `.usearch` file.
    ///
    /// Acquires an **exclusive file lock** on the `.usearch` file to prevent
    /// cross-process race conditions (e.g., `xargs -P5 uteke remember`).
    /// The lock is held until this `VectorIndex` is dropped (#543).
    ///
    /// Note: Both `save()` and `load()` use buffer-based serialization (#647,
    /// #684) to bypass usearch's C++ file I/O on Windows. The on-disk format is
    /// identical — `save_to_buffer` and `restore_from_buffer` produce/consume
    /// the same byte stream as the native file-based methods.
    pub fn load_or_create(path: &Path, dims: usize) -> Result<Self, Error> {
        // Ensure the file exists so we can open + lock it.
        if !path.exists() {
            // Create a zero-byte placeholder; usearch will overwrite on save.
            std::fs::write(path, []).map_err(|e| Error::embed("create usearch file", e))?;
        }

        let lock_file = acquire_file_lock(path)?;

        let mut idx = if path.metadata().map_or(true, |m| m.len() == 0) {
            Self::new(dims)?
        } else {
            Self::load(path)?
        };
        idx.path = Some(path.to_path_buf());
        idx._lock_file = Some(lock_file);
        Ok(idx)
    }

    /// Load an existing index from disk.
    ///
    /// Uses buffer-based deserialization (#684): reads the file into memory via
    /// Rust's `std::fs::read()`, then deserializes via `restore_from_buffer()`.
    /// This bypasses usearch's C++ `fopen("rb")` + `mmap()` which causes
    /// "Permission denied" errors on Windows (#684).
    pub fn load(path: &Path) -> Result<Self, Error> {
        let buffer = std::fs::read(path).map_err(|e| Error::embed("read usearch file", e))?;
        let index = Index::restore_from_buffer(&buffer)
            .map_err(|e| Error::embed("load vector index", e))?;

        let _size = index.size();

        // Rebuild key mappings from the sidecar file
        let mut key_to_id = HashMap::new();
        let mut id_to_key = HashMap::new();
        let mut next_key = 0u64;

        let mapping_path = path.with_extension("keys");
        if mapping_path.exists() {
            let data = std::fs::read_to_string(&mapping_path)
                .map_err(|e| Error::embed("read key mapping", e))?;
            for line in data.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some((key_str, id)) = line.split_once('\t') {
                    if let Ok(key) = key_str.parse::<u64>() {
                        key_to_id.insert(key, id.to_string());
                        id_to_key.insert(id.to_string(), key);
                        next_key = next_key.max(key + 1);
                    }
                }
            }
        }

        Ok(Self {
            index,
            key_to_id,
            id_to_key,
            next_key,
            path: Some(path.to_path_buf()),
            dirty: false,
            _lock_file: None, // Set by caller (load_or_create)
        })
    }

    /// Save index and key mappings to disk.
    ///
    /// Uses buffer-based serialization (#647): serializes the usearch index
    /// into an in-memory buffer via `save_to_buffer`, then writes the buffer
    /// to disk using Rust's `std::fs` with atomic write (temp file + rename).
    ///
    /// This bypasses usearch's C++ `fopen("wb")` file I/O which has known
    /// issues on Windows:
    /// - `fopen` fails silently on paths > 260 chars (MAX_PATH)
    /// - `fopen("wb")` exclusive access conflicts with `fs2` exclusive lock
    /// - Windows Defender can intercept `fwrite` calls
    ///
    /// The in-memory buffer approach is safe because:
    /// - The index data is already fully in RAM (usearch always loads fully)
    /// - Buffer size = `serialized_length()`, same data as file-based save
    /// - Atomic write prevents corruption on crash
    pub fn save(&mut self) -> Result<(), Error> {
        if let Some(ref path) = self.path {
            // Serialize index to in-memory buffer, bypassing C++ file I/O (#647)
            let buf_len = self.index.serialized_length();
            let mut buffer = vec![0u8; buf_len];
            self.index
                .save_to_buffer(&mut buffer)
                .map_err(|e| Error::embed("save vector index to buffer", e))?;

            // Write buffer to disk via atomic write (temp file + rename)
            let tmp_path = path.with_extension(format!("{USEARCH_EXT}.tmp"));
            std::fs::write(&tmp_path, &buffer)
                .map_err(|e| Error::embed("write temp usearch index", e))?;
            std::fs::rename(&tmp_path, path)
                .map_err(|e| Error::embed("rename temp to final usearch index", e))?;

            // Save key→id mapping as sidecar file using atomic write
            let mapping_path = path.with_extension("keys");
            let mut lines = Vec::new();
            for (&key, id) in &self.key_to_id {
                lines.push(format!("{key}\t{id}"));
            }
            atomic_write(&mapping_path, lines.join("\n").as_bytes())?;

            self.dirty = false;
        }
        Ok(())
    }

    /// Build the index from a list of (id, embedding) pairs.
    /// Used for migration from old HNSW or full rebuild.
    pub fn build(&mut self, items: &[(String, Vec<f32>)]) -> Result<(), Error> {
        // Reset
        let dims = if items.is_empty() {
            DEFAULT_DIMS
        } else {
            items[0].1.len()
        };
        self.index = match Self::create_index(dims) {
            Ok(idx) => idx,
            Err(e) => {
                return Err(e);
            }
        };
        self.key_to_id.clear();
        self.id_to_key.clear();
        self.next_key = 0;

        // Pre-reserve capacity for bulk insert
        if !items.is_empty() {
            if let Err(e) = self.index.reserve(items.len()) {
                tracing::error!("Failed to reserve usearch capacity: {e}");
            }
        }

        for (id, embedding) in items {
            self.insert(id, embedding)?;
        }
        Ok(())
    }

    /// Insert a single item into the index.
    /// If the ID already exists, removes the old entry first to prevent duplicates.
    /// Returns error if the underlying usearch operation fails.
    pub fn insert(&mut self, id: &str, embedding: &[f32]) -> Result<(), Error> {
        // Guard: remove old entry if ID already exists (prevents duplicate + stale slot)
        if let Some(old_key) = self.id_to_key.get(id) {
            let old_key = *old_key;
            self.key_to_id.remove(&old_key);
            self.index.remove(old_key).map_err(|e| {
                Error::embed_msg(format!(
                    "Failed to remove old entry for duplicate ID {id}: {e}"
                ))
            })?;
        }

        let key = self.next_key;
        self.next_key += 1;

        self.key_to_id.insert(key, id.to_string());
        self.id_to_key.insert(id.to_string(), key);

        // Auto-reserve if at capacity
        if self.index.size() >= self.index.capacity() {
            let new_cap = (self.index.capacity() + 1024).max(1024);
            self.index.reserve(new_cap).map_err(|e| {
                Error::embed_msg(format!("Failed to reserve usearch capacity: {e}"))
            })?;
        }

        self.index
            .add(key, embedding)
            .map_err(|e| Error::embed_msg(format!("Failed to insert into usearch index: {e}")))?;

        self.dirty = true;
        Ok(())
    }

    /// Remove an item by memory ID. Incremental — no rebuild.
    pub fn remove(&mut self, id: &str) -> bool {
        if let Some(key) = self.id_to_key.remove(id) {
            self.key_to_id.remove(&key);
            if let Err(e) = self.index.remove(key) {
                tracing::error!("Failed to remove from usearch index: {e}");
            }
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Search for the k nearest neighbors of the query vector.
    /// Returns (memory_id, distance_f32) pairs, sorted by distance ascending.
    /// Search for k nearest neighbors.
    /// Note: `ef` parameter is accepted for API compatibility but not passed to
    /// usearch v2.25.3 (Rust bindings don't expose `ef` in `search()`).
    pub fn search(&self, query: &[f32], k: usize, _ef: usize) -> Vec<(String, f32)> {
        if self.index.size() == 0 {
            return Vec::new();
        }

        let count = k.max(1);
        let results = match self.index.search(query, count) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("usearch search failed: {e}");
                return Vec::new();
            }
        };

        results
            .keys
            .iter()
            .zip(results.distances.iter())
            .filter_map(|(key, dist)| self.key_to_id.get(key).map(|id| (id.clone(), *dist)))
            .collect()
    }

    /// Number of items in the index.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.index.size()
    }

    /// Embedding dimensionality of this index.
    ///
    /// Used by backend dispatch to detect dim mismatch when the user swaps
    /// embedding backends on an existing store (#337).
    pub fn dims(&self) -> usize {
        self.index.dimensions()
    }

    /// Check if the index is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.index.size() == 0
    }

    /// Whether the index has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn create_index(dims: usize) -> Result<Index, Error> {
        let options = IndexOptions {
            dimensions: dims,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };

        Index::new(&options).map_err(|e| {
            Error::embed_msg(format!(
                "Failed to create usearch index (dims={dims}): {e}. This is likely an out-of-memory condition."
            ))
        })
    }
}

impl Default for VectorIndex {
    fn default() -> Self {
        Self::new(DEFAULT_DIMS).expect("Failed to create default vector index")
    }
}

/// Convert cosine distance (0..2) to cosine similarity (0..1).
/// usearch with MetricKind::Cos returns cosine *distance* (1 - similarity).
pub fn cosine_distance_to_similarity(distance: f32) -> f32 {
    // usearch cosine distance = 1 - cosine_similarity
    let sim = 1.0 - distance;
    sim.clamp(0.0, 1.0)
}

/// Acquire an exclusive file lock on the usearch index file (#543).
///
/// Blocks until the lock is available. This serializes concurrent access from
/// separate CLI processes (e.g., `xargs -P5 uteke remember`).
fn acquire_file_lock(path: &Path) -> Result<File, Error> {
    let file = File::options()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| Error::embed_msg(format!("Failed to open usearch file for locking: {e}")))?;

    if file.try_lock_exclusive().is_ok() {
        tracing::debug!("usearch file lock acquired: {}", path.display());
    } else {
        tracing::debug!("usearch file lock busy on {}, waiting...", path.display());
        file.lock_exclusive()
            .map_err(|e| Error::embed("acquire exclusive file lock on usearch", e))?;
        tracing::debug!(
            "usearch file lock acquired (after wait): {}",
            path.display()
        );
    }

    Ok(file)
}

/// Atomic file write: write to temp file then rename.
/// Prevents corruption if process crashes mid-write.
/// POSIX guarantees rename() is atomic on the same filesystem.
fn atomic_write(path: &std::path::Path, data: &[u8]) -> Result<(), Error> {
    let tmp_path = path.with_extension("keys.tmp");
    std::fs::write(&tmp_path, data).map_err(|e| Error::embed("write temp key mapping", e))?;
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

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.usearch");

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
            ".usearch file must not be 0 bytes"
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
        let path = dir.path().join("roundtrip.usearch");

        let mut idx = VectorIndex::new(32).unwrap();
        idx.path = Some(path.clone());

        let v: Vec<f32> = {
            let mut v = vec![0.0f32; 32];
            v[5] = 1.0;
            v
        };
        idx.insert("round-1", &v).unwrap();
        idx.save().unwrap();

        // Verify the saved file can be loaded by usearch's restore_from_buffer
        let buffer = std::fs::read(&path).unwrap();
        let raw_index = usearch::Index::restore_from_buffer(&buffer);
        assert!(
            raw_index.is_ok(),
            "Buffer-saved index must be loadable by usearch restore_from_buffer"
        );
        assert_eq!(raw_index.unwrap().size(), 1);
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
}
