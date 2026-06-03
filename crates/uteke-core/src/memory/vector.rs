//! Persistent vector index using usearch (HNSW with disk persistence).

use crate::Error;
use std::path::{Path, PathBuf};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Default dimensions for EmbeddingGemma Q4 (768d).
const DEFAULT_DIMS: usize = 768;

/// Persistent vector index backed by usearch.
///
/// - **Startup**: loads from disk (~5ms), no rebuild needed
/// - **Insert**: incremental, no rebuild
/// - **Delete**: incremental, no rebuild
/// - **Save**: persists to disk after mutations
pub struct VectorIndex {
    index: Index,
    /// Maps integer key (u64) → memory UUID string.
    key_to_id: Vec<String>,
    /// Maps memory UUID → integer key.
    id_to_key: std::collections::HashMap<String, u64>,
    /// Next available integer key.
    next_key: u64,
    /// Path to the usearch index file.
    path: Option<PathBuf>,
    /// Whether the index has unsaved changes.
    dirty: bool,
}

impl VectorIndex {
    /// Create a new empty vector index.
    pub fn new(dims: usize) -> Self {
        let index = Self::create_index(dims);
        Self {
            index,
            key_to_id: Vec::new(),
            id_to_key: std::collections::HashMap::new(),
            next_key: 0,
            path: None,
            dirty: false,
        }
    }

    /// Load index from disk, or create empty if file doesn't exist.
    /// `path` is the path to the `.usearch` file.
    pub fn load_or_create(path: &Path, dims: usize) -> Result<Self, Error> {
        if path.exists() {
            Self::load(path)
        } else {
            let mut idx = Self::new(dims);
            idx.path = Some(path.to_path_buf());
            Ok(idx)
        }
    }

    /// Load an existing index from disk.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let path_str = path.to_string_lossy().to_string();
        let index = Index::restore(&path_str)
            .map_err(|e| Error::embed("load vector index", e))?;

        let _size = index.size();

        // Rebuild key mappings from the loaded index
        let mut key_to_id = Vec::new();
        let mut id_to_key = std::collections::HashMap::new();
        let mut next_key = 0u64;

        // We store key→id mapping in a sidecar file
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
                        if key as usize >= key_to_id.len() {
                            key_to_id.resize(key as usize + 1, String::new());
                        }
                        key_to_id[key as usize] = id.to_string();
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
        })
    }

    /// Save index and key mappings to disk.
    pub fn save(&mut self) -> Result<(), Error> {
        if let Some(ref path) = self.path {
            let path_str = path.to_string_lossy().to_string();
            self.index
                .save(&path_str)
                .map_err(|e| Error::embed("save vector index", e))?;

            // Save key→id mapping as sidecar file
            let mapping_path = path.with_extension("keys");
            let mut lines = Vec::new();
            for (key, id) in self.key_to_id.iter().enumerate() {
                if !id.is_empty() {
                    lines.push(format!("{key}\t{id}"));
                }
            }
            std::fs::write(&mapping_path, lines.join("\n"))
                .map_err(|e| Error::embed("save key mapping", e))?;

            self.dirty = false;
        }
        Ok(())
    }

    /// Build the index from a list of (id, embedding) pairs.
    /// Used for migration from old HNSW or full rebuild.
    pub fn build(&mut self, items: &[(String, Vec<f32>)]) {
        // Reset
        let dims = if items.is_empty() {
            DEFAULT_DIMS
        } else {
            items[0].1.len()
        };
        self.index = Self::create_index(dims);
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
            self.insert(id, embedding);
        }
    }

    /// Insert a single item into the index.
    pub fn insert(&mut self, id: &str, embedding: &[f32]) {
        let key = self.next_key;
        self.next_key += 1;

        // Ensure key_to_id is large enough
        if key as usize >= self.key_to_id.len() {
            self.key_to_id.resize(key as usize + 1, String::new());
        }
        self.key_to_id[key as usize] = id.to_string();
        self.id_to_key.insert(id.to_string(), key);

        // Auto-reserve if at capacity
        if self.index.size() >= self.index.capacity() {
            let new_cap = (self.index.capacity() + 1024).max(1024);
            if let Err(e) = self.index.reserve(new_cap) {
                tracing::error!("Failed to reserve usearch capacity: {e}");
            }
        }

        if let Err(e) = self.index.add(key, embedding) {
            tracing::error!("Failed to insert into usearch index: {e}");
        }

        self.dirty = true;
    }

    /// Remove an item by memory ID. Incremental — no rebuild.
    pub fn remove(&mut self, id: &str) -> bool {
        if let Some(key) = self.id_to_key.remove(id) {
            if let Err(e) = self.index.remove(key) {
                tracing::error!("Failed to remove from usearch index: {e}");
            }
            if (key as usize) < self.key_to_id.len() {
                self.key_to_id[key as usize] = String::new();
            }
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Search for the k nearest neighbors of the query vector.
    /// Returns (memory_id, distance_f32) pairs, sorted by distance ascending.
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
            .filter(|(key, _)| {
                let k = **key as usize;
                k < self.key_to_id.len() && !self.key_to_id[k].is_empty()
            })
            .map(|(key, dist)| {
                let id = self.key_to_id[*key as usize].clone();
                (id, *dist)
            })
            .collect()
    }

    /// Number of items in the index.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.index.size()
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

    fn create_index(dims: usize) -> Index {
        let options = IndexOptions {
            dimensions: dims,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };

        match Index::new(&options) {
            Ok(idx) => idx,
            Err(e) => panic!("FATAL: Failed to create usearch index (dims={dims}): {e}. This is likely an out-of-memory condition."),
        }
    }
}

impl Default for VectorIndex {
    fn default() -> Self {
        Self::new(DEFAULT_DIMS)
    }
}

/// Convert cosine distance (0..2) to cosine similarity (0..1).
/// usearch with MetricKind::Cos returns cosine *distance* (1 - similarity).
pub fn euclidean_to_cosine(distance: f32) -> f32 {
    // usearch cosine distance = 1 - cosine_similarity
    let sim = 1.0 - distance;
    sim.clamp(0.0, 1.0)
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
        let idx = VectorIndex::new(768);
        assert!(idx.is_empty());
        let results = idx.search(&[0.0; 768], 5, 50);
        assert!(results.is_empty());
    }

    #[test]
    fn test_insert_and_search() {
        let mut idx = VectorIndex::new(768);

        let v1 = make_vec(768, 0);
        let v2 = make_vec(768, 1);
        let mut v3 = vec![0.0f32; 768];
        v3[0] = 0.9;
        v3[1] = 0.1;
        let norm = v3.iter().map(|x| x * x).sum::<f32>().sqrt();
        v3.iter_mut().for_each(|x| *x /= norm);

        idx.insert("m1", &v1);
        idx.insert("m2", &v2);
        idx.insert("m3", &v3);

        assert_eq!(idx.len(), 3);

        let results = idx.search(&v1, 3, 50);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "m1");
    }

    #[test]
    fn test_remove() {
        let mut idx = VectorIndex::new(768);

        let v1 = make_vec(768, 0);
        let v2 = make_vec(768, 1);

        idx.insert("m1", &v1);
        idx.insert("m2", &v2);

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
        let mut idx = VectorIndex::new(64);
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

        idx.insert("mem-1", &v1);
        idx.insert("mem-2", &v2);
        idx.save().unwrap();

        // Load from disk
        let loaded = VectorIndex::load(&path).unwrap();
        assert_eq!(loaded.len(), 2);

        // Search on loaded index
        let results = loaded.search(&v1, 5, 50);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "mem-1");
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

        let mut idx = VectorIndex::new(768);
        idx.build(&items);
        assert_eq!(idx.len(), 10);

        let query = make_vec(768, 0);
        let results = idx.search(&query, 3, 50);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "item-0");
    }
}
