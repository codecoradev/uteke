//! Fallback embedding backend — tries local ONNX first, falls back to cloud.
//!
//! Zero-config by default: only activates when `[embed_fallback]` is configured
//! in uteke.toml. Uses the existing [`OpenAiEmbedder`] as the cloud backend
//! (any OpenAI-compatible API — Modal, Together, etc.).
//!
//! Dim validation: if primary and fallback have different dims, the fallback
//! is rejected at construction time to prevent silent index corruption.

use crate::embed::Embedder;
use crate::Error;

/// Fallback embedder that tries a primary backend first, then a cloud fallback.
///
/// Always wraps exactly one primary embedder (required). The fallback embedder
/// is optional — if `None`, failures from primary propagate directly.
pub struct FallbackEmbedder {
    primary: Box<dyn Embedder>,
    fallback: Option<Box<dyn Embedder>>,
}

impl std::fmt::Debug for FallbackEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackEmbedder")
            .field("primary", &self.primary.name())
            .field("fallback", &self.fallback.as_ref().map(|fb| fb.name()))
            .finish()
    }
}

impl FallbackEmbedder {
    /// Create a new fallback embedder.
    ///
    /// `primary` is required (e.g., local ONNX).
    /// `fallback` is optional (e.g., OpenAI-compatible cloud API).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Validation`] if both embedders have different dimensions,
    /// which would corrupt the vector index.
    pub fn new(
        primary: Box<dyn Embedder>,
        fallback: Option<Box<dyn Embedder>>,
    ) -> Result<Self, Error> {
        // Validate dimension compatibility
        if let Some(ref fb) = fallback {
            let p_dims = primary.dims();
            let f_dims = fb.dims();
            if p_dims != f_dims {
                return Err(Error::Validation(format!(
                    "Embed fallback dimension mismatch: primary produces {p_dims}d but fallback produces {f_dims}d. \
                     Both must match for index compatibility."
                )));
            }
        }

        Ok(Self { primary, fallback })
    }
}

impl Embedder for FallbackEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        match self.primary.embed(text) {
            Ok(vec) => Ok(vec),
            Err(primary_err) => {
                match &self.fallback {
                    Some(cloud) => {
                        tracing::info!(
                            "Primary embed failed: {primary_err}. Falling back to cloud embed."
                        );
                        match cloud.embed(text) {
                            Ok(vec) => Ok(vec),
                            Err(cloud_err) => {
                                tracing::error!(
                                    "Both primary and fallback embed failed. Primary: {primary_err}, Fallback: {cloud_err}"
                                );
                                // Return the original primary error — it's the more
                                // actionable one (OOM, model missing, etc.)
                                Err(primary_err)
                            }
                        }
                    }
                    None => Err(primary_err),
                }
            }
        }
    }

    fn dims(&self) -> usize {
        self.primary.dims()
    }

    fn max_seq_len(&self) -> usize {
        // Use the primary's max_seq_len — it's the tighter constraint
        self.primary.max_seq_len()
    }

    fn name(&self) -> &str {
        match &self.fallback {
            Some(_) => "fallback(onnx→cloud)",
            None => self.primary.name(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A mock embedder for testing.
    struct MockEmbedder {
        name: &'static str,
        dims: usize,
        fail_count: AtomicUsize,
        max_calls: usize, // fail this many times before succeeding
    }

    impl MockEmbedder {
        fn new(name: &'static str, dims: usize, fail_count: usize) -> Self {
            Self {
                name,
                dims,
                fail_count: AtomicUsize::new(0),
                max_calls: fail_count,
            }
        }
    }

    impl Embedder for MockEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, Error> {
            let count = self.fail_count.fetch_add(1, Ordering::SeqCst);
            if count < self.max_calls {
                Err(Error::embed_msg(format!("{} embed failed", self.name)))
            } else {
                Ok(vec![0.1; self.dims])
            }
        }

        fn dims(&self) -> usize {
            self.dims
        }

        fn max_seq_len(&self) -> usize {
            512
        }

        fn name(&self) -> &str {
            self.name
        }
    }

    #[test]
    fn primary_succeeds_no_fallback_needed() {
        let primary = Box::new(MockEmbedder::new("onnx", 768, 0));
        let fb = FallbackEmbedder::new(primary, None).unwrap();
        let result = fb.embed("hello").unwrap();
        assert_eq!(result.len(), 768);
    }

    #[test]
    fn primary_succeeds_even_with_fallback_configured() {
        let primary = Box::new(MockEmbedder::new("onnx", 768, 0));
        let fallback = Box::new(MockEmbedder::new("cloud", 768, 0));
        let fb = FallbackEmbedder::new(primary, Some(fallback)).unwrap();
        let result = fb.embed("hello").unwrap();
        assert_eq!(result.len(), 768);
        assert_eq!(fb.name(), "fallback(onnx→cloud)");
    }

    #[test]
    fn fallback_kicks_in_on_primary_failure() {
        let primary = Box::new(MockEmbedder::new("onnx", 768, 999)); // always fail
        let fallback = Box::new(MockEmbedder::new("cloud", 768, 0)); // always succeed
        let fb = FallbackEmbedder::new(primary, Some(fallback)).unwrap();
        let result = fb.embed("hello").unwrap();
        assert_eq!(result.len(), 768);
    }

    #[test]
    fn both_fail_returns_primary_error() {
        let primary = Box::new(MockEmbedder::new("onnx", 768, 999));
        let fallback = Box::new(MockEmbedder::new("cloud", 768, 999));
        let fb = FallbackEmbedder::new(primary, Some(fallback)).unwrap();
        let err = fb.embed("hello").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("onnx embed failed"), "got: {msg}");
    }

    #[test]
    fn no_fallback_propagates_error() {
        let primary = Box::new(MockEmbedder::new("onnx", 768, 999));
        let fb = FallbackEmbedder::new(primary, None).unwrap();
        let err = fb.embed("hello").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("onnx embed failed"), "got: {msg}");
    }

    #[test]
    fn dim_mismatch_rejected() {
        let primary = Box::new(MockEmbedder::new("onnx", 768, 0));
        let fallback = Box::new(MockEmbedder::new("cloud", 1536, 0));
        let err = FallbackEmbedder::new(primary, Some(fallback)).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("dimension mismatch"), "got: {msg}");
        assert!(msg.contains("768"), "missing primary dims: {msg}");
        assert!(msg.contains("1536"), "missing fallback dims: {msg}");
    }

    #[test]
    fn dims_and_seq_len_from_primary() {
        let primary = Box::new(MockEmbedder::new("onnx", 768, 0));
        let fallback = Box::new(MockEmbedder::new("cloud", 768, 0));
        let fb = FallbackEmbedder::new(primary, Some(fallback)).unwrap();
        assert_eq!(fb.dims(), 768);
        assert_eq!(fb.max_seq_len(), 512);
    }
}
