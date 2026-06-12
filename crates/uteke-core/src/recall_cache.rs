//! Recall cache for avoiding redundant embedding computation.
//!
//! TTL-based cache keyed by (query_hash, namespace, limit, strategy) with FIFO eviction.
//! Embedding a query takes ~50ms; cache hit returns in <1μs.
//!
//! Note: min_score is NOT part of the cache key. Cached results are stored
//! WITHOUT min_score filtering. The caller re-applies min_score after cache
//! lookup, ensuring correctness regardless of threshold differences between
//! calls. This works because cache stores the full result set (limit * multiplier)
//! and the caller truncates after filtering.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::memory::types::{RecallStrategy, SearchResult};

/// Cache key: query hash + namespace + limit + strategy.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    query_hash: u64,
    namespace: String,
    limit: usize,
    /// Strategy affects which search path is used (vector, fts5, hybrid).
    strategy: &'static str,
}

/// Cache entry with TTL.
struct CacheEntry {
    results: Vec<SearchResult>,
    inserted_at: Instant,
}

/// Recall cache configuration.
#[derive(Debug, Clone, Copy)]
pub struct RecallCacheConfig {
    /// Maximum number of cached queries.
    pub max_entries: usize,
    /// Time-to-live in seconds for cached entries.
    pub ttl_secs: u64,
}

impl Default for RecallCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 256,
            ttl_secs: 300, // 5 minutes
        }
    }
}

/// Thread-safe recall cache with FIFO eviction and TTL expiry.
pub struct RecallCache {
    entries: Mutex<HashMap<CacheKey, CacheEntry>>,
    config: RecallCacheConfig,
    /// Total number of cache hits (for metrics).
    hits: Mutex<u64>,
    /// Total number of cache misses (for metrics).
    misses: Mutex<u64>,
}

impl RecallCache {
    pub fn new(config: RecallCacheConfig) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            config,
            hits: Mutex::new(0),
            misses: Mutex::new(0),
        }
    }

    /// Look up cached recall results.
    /// Returns None on miss or expired entry.
    pub fn get(
        &self,
        query: &str,
        namespace: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        strategy: RecallStrategy,
    ) -> Option<Vec<SearchResult>> {
        let key = self.make_key(query, namespace, limit, tags_filter, strategy);
        let mut entries = self.entries.lock().ok()?;

        if let Some(entry) = entries.get(&key) {
            if entry.inserted_at.elapsed().as_secs() < self.config.ttl_secs {
                if let Ok(mut hits) = self.hits.lock() {
                    *hits += 1;
                }
                return Some(entry.results.clone());
            }
            // Expired — remove
            entries.remove(&key);
        }

        if let Ok(mut misses) = self.misses.lock() {
            *misses += 1;
        }
        None
    }

    /// Store recall results in cache.
    pub fn put(
        &self,
        query: &str,
        namespace: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        strategy: RecallStrategy,
        results: Vec<SearchResult>,
    ) {
        let key = self.make_key(query, namespace, limit, tags_filter, strategy);
        if let Ok(mut entries) = self.entries.lock() {
            // Evict expired entries first
            let now = Instant::now();
            entries
                .retain(|_, v| now.duration_since(v.inserted_at).as_secs() < self.config.ttl_secs);

            // FIFO eviction if still over capacity (oldest insert time first)
            if entries.len() >= self.config.max_entries {
                if let Some(oldest_key) = entries
                    .iter()
                    .min_by_key(|(_, v)| v.inserted_at)
                    .map(|(k, _)| k.clone())
                {
                    entries.remove(&oldest_key);
                }
            }

            entries.insert(
                key,
                CacheEntry {
                    results,
                    inserted_at: now,
                },
            );
        }
    }

    /// Invalidate all cached entries for a namespace (e.g., after remember/forget).
    pub fn invalidate_namespace(&self, namespace: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|k, _| k.namespace != namespace);
        }
    }

    /// Clear all cached entries.
    #[allow(dead_code)] // Public API — kept for library consumers
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    /// Get cache hit rate metrics.
    pub fn metrics(&self) -> (u64, u64) {
        let hits = self.hits.lock().map(|h| *h).unwrap_or(0);
        let misses = self.misses.lock().map(|m| *m).unwrap_or(0);
        (hits, misses)
    }

    fn make_key(
        &self,
        query: &str,
        namespace: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        strategy: RecallStrategy,
    ) -> CacheKey {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        if let Some(tags) = tags_filter {
            tags.len().hash(&mut hasher); // sentinel: prevents ["a","bc"] vs ["ab","c"] collision
            for t in tags {
                t.hash(&mut hasher);
                0u8.hash(&mut hasher); // separator: terminates each tag in the hash stream
            }
        }
        CacheKey {
            query_hash: hasher.finish(),
            namespace: namespace.to_string(),
            limit,
            strategy: strategy.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_put_get() {
        let cache = RecallCache::new(RecallCacheConfig {
            max_entries: 10,
            ttl_secs: 60,
        });

        let results = vec![SearchResult {
            memory: crate::Memory {
                id: "test-id".to_string(),
                content: "test content".to_string(),
                embedding: vec![],
                tags: vec!["test".to_string()],
                metadata: serde_json::Value::Null,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                namespace: "default".to_string(),
                access_count: 0,
                last_accessed: None,
                deprecated: false,
                valid_from: None,
                valid_until: None,
                memory_type: "fact".to_string(),
                importance: 0.5,
                pinned: false,
            },
            score: 0.95,
        }];

        // Miss first
        assert!(cache
            .get("test query", "default", 5, None, RecallStrategy::Vector)
            .is_none());

        // Put then hit (same strategy)
        cache.put(
            "test query",
            "default",
            5,
            None,
            RecallStrategy::Vector,
            results.clone(),
        );
        let cached = cache.get("test query", "default", 5, None, RecallStrategy::Vector);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().len(), 1);

        // Different strategy = miss
        assert!(cache
            .get("test query", "default", 5, None, RecallStrategy::Hybrid)
            .is_none());

        let (hits, misses) = cache.metrics();
        assert_eq!(hits, 1);
        assert_eq!(misses, 2); // first miss + different strategy miss
    }

    #[test]
    fn test_cache_invalidate_namespace() {
        let cache = RecallCache::new(RecallCacheConfig {
            max_entries: 10,
            ttl_secs: 60,
        });

        let results = vec![];
        cache.put(
            "query",
            "ns1",
            5,
            None,
            RecallStrategy::Vector,
            results.clone(),
        );
        cache.put(
            "query",
            "ns2",
            5,
            None,
            RecallStrategy::Vector,
            results.clone(),
        );

        cache.invalidate_namespace("ns1");
        assert!(cache
            .get("query", "ns1", 5, None, RecallStrategy::Vector)
            .is_none());
        assert!(cache
            .get("query", "ns2", 5, None, RecallStrategy::Vector)
            .is_some());
    }

    #[test]
    fn test_cache_clear() {
        let cache = RecallCache::new(RecallCacheConfig {
            max_entries: 10,
            ttl_secs: 60,
        });

        cache.put("query", "default", 5, None, RecallStrategy::Vector, vec![]);
        cache.clear();
        assert!(cache
            .get("query", "default", 5, None, RecallStrategy::Vector)
            .is_none());
    }
}
