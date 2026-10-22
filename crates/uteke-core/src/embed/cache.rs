//! Persistent embedding cache backed by SQLite.
//!
//! Caches query embeddings by SHA256(text + model_name) to avoid re-loading
//! the 188MB ONNX model on every CLI invocation (#896).
//!
//! - Cache hit: ~1ms (SQLite read + bincode deserialize)
//! - Cache miss: falls through to inner embedder (~2s ONNX load + ~50ms inference)
//!
//! The cache lives in a dedicated SQLite file (`embed_cache.db`) inside the
//! uteke home directory, separate from the main `uteke.db` to avoid schema
//! coupling and lock contention.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::Error;
use crate::embed::Embedder;

/// Persistent SQLite cache for embedding vectors.
///
/// Stores text→vector mappings keyed by SHA256(model_name + text).
/// Thread-safe via internal `Mutex` on the `Connection`.
pub struct EmbeddingCache {
    conn: Mutex<Connection>,
}

impl EmbeddingCache {
    /// Open (or create) the cache database at `cache_path`.
    pub fn open(cache_path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(cache_path)
            .map_err(|e| Error::db("open embedding cache database", e))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| Error::db("set cache WAL mode", e))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .map_err(|e| Error::db("set cache synchronous mode", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS embedding_cache (
                text_hash   TEXT PRIMARY KEY,
                model_name  TEXT NOT NULL,
                embedding   BLOB NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_cache_model ON embedding_cache(model_name);",
        )
        .map_err(|e| Error::db("create embedding cache schema", e))?;

        // The cache used to be keyed by the embedder's constant name
        // ("openai", "embeddinggemma-q4"), which could serve the vector of
        // another model (#1536). Rows written that way can never be hit again
        // and may hold vectors of the wrong model: purge them once. A key now
        // is an identity (`name:...:dims`), so an old name has no colon.
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0);
        if version < 1 {
            conn.execute(
                "DELETE FROM embedding_cache WHERE model_name NOT LIKE '%:%'",
                [],
            )
            .map_err(|e| Error::db("purge embedding cache rows of the old key", e))?;
            conn.execute_batch("PRAGMA user_version = 1;")
                .map_err(|e| Error::db("mark the embedding cache as migrated", e))?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Compute the cache key: SHA256(model_name + "\0" + text).
    fn cache_key(model_name: &str, text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(model_name.as_bytes());
        hasher.update(b"\0");
        hasher.update(text.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Look up a cached embedding. Returns `None` on miss, and for a stored
    /// vector that does not have `dims` elements (it came from something else).
    fn lookup(&self, model_name: &str, text: &str, dims: usize) -> Option<Vec<f32>> {
        let key = Self::cache_key(model_name, text);
        let conn = self.conn.lock().ok()?;

        let blob: Vec<u8> = conn
            .query_row(
                "SELECT embedding FROM embedding_cache WHERE text_hash = ?1 AND model_name = ?2",
                rusqlite::params![&key, model_name],
                |row| row.get(0),
            )
            .ok()?;

        deserialize_embedding(&blob).filter(|v| v.len() == dims)
    }

    /// Store an embedding in the cache.
    fn store(&self, model_name: &str, text: &str, embedding: &[f32]) {
        // An empty vector is "no embedding", not something to remember.
        if embedding.is_empty() {
            return;
        }
        let key = Self::cache_key(model_name, text);
        let blob = serialize_embedding(embedding);

        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO embedding_cache (text_hash, model_name, embedding)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![&key, model_name, &blob],
            );
        }
    }

    /// Clear all cached embeddings of a backend or model: the rows whose
    /// identity is `model_name` or starts with `model_name:` (an identity is
    /// `name:...:dims`). Called when the model is upgraded or changed.
    #[allow(dead_code)] // Public API for future cache invalidation
    pub fn invalidate_model(&self, model_name: &str) -> Result<(), Error> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| Error::lock("embedding cache during invalidation"))?;
        // `substr` compare, not LIKE: a name may contain `%` or `_`.
        conn.execute(
            "DELETE FROM embedding_cache
             WHERE model_name = ?1 OR substr(model_name, 1, length(?1) + 1) = ?1 || ':'",
            rusqlite::params![model_name],
        )
        .map_err(|e| Error::db("invalidate embedding cache", e))?;
        Ok(())
    }
}

/// Decorator that wraps any [`Embedder`] with a persistent [`EmbeddingCache`].
///
/// On cache hit (~1ms SQLite read), skips the inner embedder entirely —
/// avoiding the ~2s ONNX model load on repeated CLI invocations (#896).
pub struct CachingEmbedder {
    inner: Box<dyn Embedder>,
    cache: EmbeddingCache,
}

impl CachingEmbedder {
    /// Wrap `inner` with a pre-opened `EmbeddingCache`.
    pub fn with_cache(inner: Box<dyn Embedder>, cache: EmbeddingCache) -> Self {
        Self { inner, cache }
    }
}

impl Embedder for CachingEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        // Keyed by what produced the vectors, not by the backend's name (#1536).
        let model_name = self.inner.identity();
        let model_name = model_name.as_str();

        // Try cache first
        if let Some(cached) = self.cache.lookup(model_name, text, self.inner.dims()) {
            tracing::debug!(text_len = text.len(), "embedding cache hit");
            return Ok(cached);
        }

        // Cache miss — compute embedding
        tracing::debug!(text_len = text.len(), "embedding cache miss");
        let embedding = self.inner.embed(text)?;

        // Store in cache (best-effort, don't fail if cache write errors)
        self.cache.store(model_name, text, &embedding);

        Ok(embedding)
    }

    fn dims(&self) -> usize {
        self.inner.dims()
    }

    fn max_seq_len(&self) -> usize {
        self.inner.max_seq_len()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn identity(&self) -> String {
        self.inner.identity()
    }
}

// ── Serialization ──────────────────────────────────────────────────────────

/// Serialize f32 slice as little-endian bytes (4 bytes per element).
fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &v in embedding {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Deserialize little-endian bytes back to Vec<f32>.
fn deserialize_embedding(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() % 4 != 0 {
        return None;
    }
    let mut result = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        let bytes: [u8; 4] = chunk.try_into().ok()?;
        result.push(f32::from_le_bytes(bytes));
    }
    Some(result)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock embedder that counts calls.
    struct CountingEmbedder {
        dims: usize,
        call_count: Mutex<usize>,
    }

    impl Embedder for CountingEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            Ok(vec![0.5; self.dims])
        }

        fn dims(&self) -> usize {
            self.dims
        }

        fn max_seq_len(&self) -> usize {
            128
        }

        fn name(&self) -> &str {
            "mock-model"
        }
    }

    fn tmp_cache_path() -> std::path::PathBuf {
        // Unique per call: tests run in parallel, and two calls in the same
        // nanosecond used to share (and delete) one file.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!("uteke_test_cache_{}_{n}.db", std::process::id()))
    }

    #[test]
    fn test_cache_hit_skips_inner() {
        let cache_path = tmp_cache_path();
        let inner = Box::new(CountingEmbedder {
            dims: 4,
            call_count: Mutex::new(0),
        });
        let cache_db = EmbeddingCache::open(&cache_path).unwrap();
        let cache = CachingEmbedder::with_cache(inner, cache_db);

        // First call — miss, inner called
        let emb1 = cache.embed("hello world").unwrap();
        assert_eq!(emb1, vec![0.5; 4]);

        // Second call — hit, inner NOT called
        let emb2 = cache.embed("hello world").unwrap();
        assert_eq!(emb2, vec![0.5; 4]);

        // Verify inner was only called once
        let count = {
            // Access inner through trait — we stored call_count in the mock,
            // but can't access it directly through Box<dyn>. Instead, verify
            // correctness by checking the cache persisted to SQLite.
            let conn = Connection::open(&cache_path).unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
                .unwrap();
            count
        };
        assert_eq!(count, 1, "only one entry should be in the cache");

        let _ = std::fs::remove_file(&cache_path);
    }

    #[test]
    fn test_different_text_separate_entries() {
        let cache_path = tmp_cache_path();
        let inner = Box::new(CountingEmbedder {
            dims: 2,
            call_count: Mutex::new(0),
        });
        let cache_db = EmbeddingCache::open(&cache_path).unwrap();
        let cache = CachingEmbedder::with_cache(inner, cache_db);

        let _ = cache.embed("text one").unwrap();
        let _ = cache.embed("text two").unwrap();
        let _ = cache.embed("text one").unwrap(); // cache hit

        let conn = Connection::open(&cache_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2, "two unique texts = two cache entries");

        let _ = std::fs::remove_file(&cache_path);
    }

    #[test]
    fn test_cache_key_deterministic() {
        let key1 = EmbeddingCache::cache_key("model-a", "hello");
        let key2 = EmbeddingCache::cache_key("model-a", "hello");
        let key3 = EmbeddingCache::cache_key("model-b", "hello");

        assert_eq!(key1, key2, "same model + text = same key");
        assert_ne!(key1, key3, "different model = different key");
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let embedding = vec![0.1, 0.2, 0.3, -0.4, 1.5];
        let blob = serialize_embedding(&embedding);
        let restored = deserialize_embedding(&blob).unwrap();

        assert_eq!(embedding.len(), restored.len());
        for (a, b) in embedding.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn test_invalidate_model() {
        let cache_path = tmp_cache_path();
        let inner = Box::new(CountingEmbedder {
            dims: 2,
            call_count: Mutex::new(0),
        });
        let cache_db = EmbeddingCache::open(&cache_path).unwrap();
        let cache = CachingEmbedder::with_cache(inner, cache_db);

        cache.embed("text one").unwrap();
        cache.embed("text two").unwrap();

        // Verify 2 entries
        let conn_check = Connection::open(&cache_path).unwrap();
        let count: i64 = conn_check
            .query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);

        // Invalidate through the real function: it removes every identity of
        // the model (`mock-model:2`), and nothing of another one.
        cache.cache.invalidate_model("mock-model").unwrap();

        let count_after: i64 = conn_check
            .query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count_after, 0, "cache should be empty after invalidation");

        let _ = std::fs::remove_file(&cache_path);
    }

    #[test]
    fn test_persistence_across_connections() {
        let cache_path = tmp_cache_path();

        // Write with one CachingEmbedder
        {
            let inner = Box::new(CountingEmbedder {
                dims: 3,
                call_count: Mutex::new(0),
            });
            let cache_db = EmbeddingCache::open(&cache_path).unwrap();
            let cache = CachingEmbedder::with_cache(inner, cache_db);
            let emb = cache.embed("persistent text").unwrap();
            assert_eq!(emb, vec![0.5; 3]);
        }

        // Read with a new CachingEmbedder (simulates new CLI invocation)
        {
            let inner = Box::new(CountingEmbedder {
                dims: 3,
                call_count: Mutex::new(0),
            });
            let cache_db = EmbeddingCache::open(&cache_path).unwrap();
            let cache = CachingEmbedder::with_cache(inner, cache_db);
            let emb = cache.embed("persistent text").unwrap();
            assert_eq!(emb, vec![0.5; 3]);

            // Inner should NOT have been called (cache hit from previous write)
            let count = {
                let conn = Connection::open(&cache_path).unwrap();
                conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM embedding_cache", [], |row| {
                    row.get(0)
                })
                .unwrap()
            };
            assert_eq!(count, 1, "entry should persist from previous connection");
        }

        let _ = std::fs::remove_file(&cache_path);
    }

    // ── Identity (#1536) ─────────────────────────────────────────────────

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An embedder whose identity, size and value the test chooses, counting calls.
    struct Fake {
        identity: String,
        dims: usize,
        value: f32,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl Fake {
        fn new(identity: &str, dims: usize, value: f32) -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    identity: identity.to_string(),
                    dims,
                    value,
                    calls: calls.clone(),
                    fail: false,
                },
                calls,
            )
        }
    }

    impl Embedder for Fake {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(Error::embed_msg("fake failure"));
            }
            Ok(vec![self.value; self.dims])
        }
        fn dims(&self) -> usize {
            self.dims
        }
        fn max_seq_len(&self) -> usize {
            128
        }
        fn name(&self) -> &str {
            "fake"
        }
        fn identity(&self) -> String {
            self.identity.clone()
        }
    }

    /// The same cache file used by two embedders in turn, like two runs of the CLI.
    fn run_with(path: &std::path::Path, fake: Fake, text: &str) -> Vec<f32> {
        let cache = EmbeddingCache::open(path).unwrap();
        CachingEmbedder::with_cache(Box::new(fake), cache)
            .embed(text)
            .unwrap()
    }

    #[test]
    fn the_default_identity_is_the_name_and_the_dimensions() {
        let c = CountingEmbedder {
            dims: 4,
            call_count: Mutex::new(0),
        };
        assert_eq!(c.identity(), "mock-model:4");
    }

    #[test]
    fn a_different_model_size_or_endpoint_never_gets_the_vector_of_another() {
        let path = tmp_cache_path();
        let (a, a_calls) = Fake::new("openai:model-a:8:aaaa", 8, 1.0);
        assert_eq!(run_with(&path, a, "same text"), vec![1.0; 8]);
        assert_eq!(a_calls.load(Ordering::SeqCst), 1);

        // Another model, another size, another endpoint: each must be computed.
        for (identity, dims, value) in [
            ("openai:model-b:8:aaaa", 8, 2.0),
            ("openai:model-a:4:aaaa", 4, 3.0),
            ("openai:model-a:8:bbbb", 8, 4.0),
        ] {
            let (f, calls) = Fake::new(identity, dims, value);
            assert_eq!(
                run_with(&path, f, "same text"),
                vec![value; dims],
                "{identity}"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "{identity} must reach its own backend"
            );
        }

        // The same identity is a hit.
        let (again, again_calls) = Fake::new("openai:model-a:8:aaaa", 8, 99.0);
        assert_eq!(run_with(&path, again, "same text"), vec![1.0; 8]);
        assert_eq!(again_calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_stored_vector_of_the_wrong_length_is_ignored() {
        let path = tmp_cache_path();
        let (f, _) = Fake::new("x:8", 8, 1.0);
        run_with(&path, f, "text");
        // Corrupt the stored vector: four elements where the embedder makes eight.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                "UPDATE embedding_cache SET embedding = ?1",
                rusqlite::params![serialize_embedding(&[9.0; 4])],
            )
            .unwrap();
        }
        let (g, calls) = Fake::new("x:8", 8, 5.0);
        assert_eq!(run_with(&path, g, "text"), vec![5.0; 8]);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the bad row was a miss");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_vector_is_not_remembered() {
        let path = tmp_cache_path();
        let (f, _) = Fake::new("x:0", 0, 0.0);
        run_with(&path, f, "text");
        let conn = Connection::open(&path).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM embedding_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rows_written_under_the_old_key_are_purged_once() {
        let path = tmp_cache_path();
        // A cache as the previous version left it: keyed by a constant name,
        // user_version 0.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE embedding_cache (
                    text_hash TEXT PRIMARY KEY, model_name TEXT NOT NULL,
                    embedding BLOB NOT NULL, created_at TEXT NOT NULL DEFAULT '');",
            )
            .unwrap();
            for (hash, name) in [
                ("h1", "openai"),
                ("h2", "embeddinggemma-q4"),
                ("h3", "ollama"),
            ] {
                conn.execute(
                    "INSERT INTO embedding_cache (text_hash, model_name, embedding) VALUES (?1, ?2, x'00000000')",
                    rusqlite::params![hash, name],
                )
                .unwrap();
            }
        }
        let count = |path: &std::path::Path| -> i64 {
            Connection::open(path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM embedding_cache", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count(&path), 3);
        drop(EmbeddingCache::open(&path).unwrap());
        assert_eq!(
            count(&path),
            0,
            "the old rows are gone after the first open"
        );

        // A row of the new kind survives every later open.
        let (f, _) = Fake::new("openai:m:8:cccc", 8, 1.0);
        run_with(&path, f, "kept");
        drop(EmbeddingCache::open(&path).unwrap());
        assert_eq!(count(&path), 1, "the purge runs once, not at every open");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn what_the_cloud_fallback_answers_is_never_cached() {
        use crate::embed::FallbackEmbedder;
        let path = tmp_cache_path();
        let (mut primary, _) = Fake::new("onnx:8", 8, 1.0);
        primary.fail = true;
        let (cloud, cloud_calls) = Fake::new("openai:cloud:8:dddd", 8, 7.0);
        // The wrapping order of the core: the cache around the primary, the
        // fallback around the cached primary.
        let cached_primary =
            CachingEmbedder::with_cache(Box::new(primary), EmbeddingCache::open(&path).unwrap());
        let fb = FallbackEmbedder::new(Box::new(cached_primary), Some(Box::new(cloud))).unwrap();
        assert_eq!(fb.embed("text").unwrap(), vec![7.0; 8]);
        assert_eq!(fb.embed("text").unwrap(), vec![7.0; 8]);
        assert_eq!(
            cloud_calls.load(Ordering::SeqCst),
            2,
            "no cache served the cloud vector"
        );
        let conn = Connection::open(&path).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM embedding_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "nothing the cloud produced is in the cache");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_remote_embedders_identify_model_dimensions_and_endpoint_but_not_the_key() {
        use crate::embed::{OllamaEmbedder, OpenAiEmbedder};
        let o = |model: &str, dims: usize, url: &str, key: &str| {
            OpenAiEmbedder::new(key, model, url, "", dims)
                .unwrap()
                .identity()
        };
        let base = o("m", 8, "https://a.example/v1", "key-one");
        assert_eq!(
            base,
            o("m", 8, "https://a.example/v1", "key-two"),
            "the key is not part of it"
        );
        assert!(
            !base.contains("key-one") && !base.contains("a.example"),
            "{base}"
        );
        assert_ne!(base, o("m2", 8, "https://a.example/v1", "k"));
        assert_ne!(base, o("m", 16, "https://a.example/v1", "k"));
        assert_ne!(base, o("m", 8, "https://b.example/v1", "k"));

        let l = |model: &str, dims: usize, url: &str| {
            OllamaEmbedder::new(url, model, dims).unwrap().identity()
        };
        let ollama = l("nomic", 8, "http://localhost:11434");
        assert_ne!(ollama, l("other", 8, "http://localhost:11434"));
        assert_ne!(ollama, l("nomic", 4, "http://localhost:11434"));
        assert_ne!(ollama, l("nomic", 8, "http://other:11434"));
        assert_ne!(ollama, base, "different backends never collide");
    }

    #[test]
    fn invalidating_a_model_leaves_the_others_alone() {
        let path = tmp_cache_path();
        for identity in ["mock-model:8", "mock-model:4", "mock-model-2:8", "other:8"] {
            let (f, _) = Fake::new(identity, 8, 1.0);
            run_with(&path, f, "text");
        }
        EmbeddingCache::open(&path)
            .unwrap()
            .invalidate_model("mock-model")
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        let mut left: Vec<String> = conn
            .prepare("SELECT model_name FROM embedding_cache ORDER BY model_name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        left.sort();
        assert_eq!(left, vec!["mock-model-2:8", "other:8"]);
        let _ = std::fs::remove_file(&path);
    }
}
