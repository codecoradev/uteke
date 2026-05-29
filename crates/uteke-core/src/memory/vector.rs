//! HNSW vector index for approximate nearest neighbor search.

use hnsw::{Hnsw, Params, Searcher};
use rand_pcg::Pcg64;
use space::Metric;

/// HNSW const generics: M=16 connections per node (non-zero layer), M0=32 (zero layer).
const M: usize = 16;
const M0: usize = 32;

/// Euclidean distance metric for Vec<f32>.
/// Stores distance as u32 (f32 bit representation) for the HNSW crate.
#[derive(Clone, Default)]
struct Euclidean;

impl Metric<Vec<f32>> for Euclidean {
    type Unit = u32;

    fn distance(&self, a: &Vec<f32>, b: &Vec<f32>) -> Self::Unit {
        let dist_sq: f32 = a.iter().zip(b.iter()).map(|(&x, &y)| (x - y).powi(2)).sum();
        dist_sq.sqrt().to_bits()
    }
}

/// In-memory HNSW index mapping internal indices to memory IDs.
pub struct VectorIndex {
    /// The HNSW graph.
    hnsw: Hnsw<Euclidean, Vec<f32>, Pcg64, M, M0>,
    /// Maps HNSW internal index → memory ID.
    id_map: Vec<String>,
}

impl VectorIndex {
    /// Create a new empty vector index.
    pub fn new() -> Self {
        Self {
            hnsw: Hnsw::new_params(Euclidean, Params::new().ef_construction(200)),
            id_map: Vec::new(),
        }
    }

    /// Build the index from a list of (id, embedding) pairs.
    pub fn build(&mut self, items: &[(String, Vec<f32>)]) {
        // Reset
        self.hnsw = Hnsw::new_params(Euclidean, Params::new().ef_construction(200));
        self.id_map.clear();

        for (id, embedding) in items {
            self.insert(id, embedding);
        }
    }

    /// Insert a single item into the index.
    pub fn insert(&mut self, id: &str, embedding: &[f32]) {
        let mut searcher = Searcher::default();
        let idx = self.hnsw.insert(embedding.to_vec(), &mut searcher);
        // Ensure id_map is large enough
        if idx >= self.id_map.len() {
            self.id_map.resize(idx + 1, String::new());
        }
        self.id_map[idx] = id.to_string();
    }

    /// Search for the k nearest neighbors of the query vector.
    /// Returns (memory_id, distance_f32) pairs, sorted by distance ascending.
    pub fn search(&mut self, query: &[f32], k: usize, ef: usize) -> Vec<(String, f32)> {
        if self.hnsw.is_empty() {
            return Vec::new();
        }

        let count = k.max(1);
        let mut searcher = Searcher::default();
        let mut dest = vec![
            space::Neighbor {
                index: !0,
                distance: !0,
            };
            count
        ];

        let found = self
            .hnsw
            .nearest(&query.to_vec(), ef, &mut searcher, &mut dest);

        found
            .iter()
            .filter(|n| n.index != !0 && n.index < self.id_map.len())
            .map(|n| {
                let distance_f32 = f32::from_bits(n.distance);
                let id = self.id_map[n.index].clone();
                (id, distance_f32)
            })
            .collect()
    }

    /// Number of items in the index.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.hnsw.len()
    }

    /// Check if the index is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.hnsw.is_empty()
    }
}

impl Default for VectorIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert Euclidean distance to cosine similarity approximation.
/// For normalized vectors, cos_sim ≈ 1 - dist²/2.
pub fn euclidean_to_cosine(distance: f32) -> f32 {
    let sim = 1.0 - (distance * distance) / 2.0;
    sim.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_index() {
        let mut idx = VectorIndex::new();
        assert!(idx.is_empty());
        let results = idx.search(&[0.0; 384], 5, 50);
        assert!(results.is_empty());
    }

    #[test]
    fn test_insert_and_search() {
        let mut idx = VectorIndex::new();

        // Create normalized vectors
        let v1: Vec<f32> = {
            let mut v = vec![0.0; 384];
            v[0] = 1.0;
            v
        };
        let v2: Vec<f32> = {
            let mut v = vec![0.0; 384];
            v[1] = 1.0;
            v
        };
        let v3: Vec<f32> = {
            let mut v = vec![0.0; 384];
            v[0] = 0.9;
            v[1] = 0.1;
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.iter_mut().for_each(|x| *x /= norm);
            v
        };

        idx.insert("m1", &v1);
        idx.insert("m2", &v2);
        idx.insert("m3", &v3);

        assert_eq!(idx.len(), 3);

        // Query close to v1 should find m1 first
        let results = idx.search(&v1, 3, 50);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "m1");
    }

    #[test]
    fn test_build_from_items() {
        let items: Vec<(String, Vec<f32>)> = (0..10)
            .map(|i| {
                let mut v = vec![0.0f32; 384];
                v[i] = 1.0;
                (format!("item-{i}"), v)
            })
            .collect();

        let mut idx = VectorIndex::new();
        idx.build(&items);
        assert_eq!(idx.len(), 10);

        let query = {
            let mut v = vec![0.0f32; 384];
            v[0] = 1.0;
            v
        };
        let results = idx.search(&query, 3, 50);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "item-0");
    }
}
