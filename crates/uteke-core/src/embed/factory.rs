//! Constructing an embedder from a resolved backend (#1539).
//!
//! `Uteke::ensure_embedder` used to build the primary embedder, check its
//! dimensions, wrap the cache and wrap the fallback in one function that held
//! the embedder lock throughout. Here each concern is its own function: one
//! arm per backend, the dimension check, and the two decorators. [`build`]
//! applies them in the one order that matters (primary, dimension check,
//! cache, fallback: the cache sits inside the fallback so vectors from the
//! cloud are never cached under the primary's identity, #1536). Nothing here
//! touches the lock or the cached init error: the caller swaps the result in.

use std::path::Path;

use super::Embedder;
use crate::{EmbeddingSettings, Error, FallbackSettings};

/// A construction failure, and whether `ensure_embedder` should remember it.
pub(crate) struct BuildError {
    pub error: Error,
    /// `Some(permanent)`: cache the failure (#822). Permanent for a local
    /// cause (the ONNX runtime is missing), transient for a network-facing
    /// backend. `None`: report it every time (a bad request, not an
    /// environment problem).
    pub cache: Option<bool>,
}

impl BuildError {
    fn uncached(error: Error) -> Self {
        Self { error, cache: None }
    }
    fn cached(error: Error, permanent: bool) -> Self {
        Self {
            error,
            cache: Some(permanent),
        }
    }
}

/// Everything the construction needs, resolved by the caller.
pub(crate) struct Plan<'a> {
    pub backend: &'a str,
    /// Settings with the environment already applied.
    pub settings: &'a EmbeddingSettings,
    /// Where the persistent embedding cache lives; `None` = no cache.
    pub cache_path: Option<&'a Path>,
    pub fallback: &'a FallbackSettings,
    /// Dimensions of the vector index the embedder will feed.
    pub index_dims: usize,
    /// `UTEKE_ALLOW_DIM_MISMATCH=1`: let a store be opened with another
    /// backend so `uteke repair` can rebuild its vectors.
    pub allow_dim_mismatch: bool,
}

/// Build the embedder for `plan`: primary, dimension check, cache, fallback.
pub(crate) fn build(plan: &Plan<'_>) -> Result<Box<dyn Embedder>, BuildError> {
    let primary = build_primary(plan.backend, plan.settings)?;
    check_dims(
        primary.as_ref(),
        plan.index_dims,
        plan.backend,
        plan.allow_dim_mismatch,
    )
    .map_err(BuildError::uncached)?;
    let dims = primary.dims();
    let cached = with_cache(primary, plan.cache_path);
    with_fallback(cached, plan.fallback, dims).map_err(BuildError::uncached)
}

/// The primary embedder of `backend`. One arm per backend, each independent.
pub(crate) fn build_primary(
    backend: &str,
    settings: &EmbeddingSettings,
) -> Result<Box<dyn Embedder>, BuildError> {
    match backend {
        "onnx" | "" => onnx(),
        "custom" => Err(BuildError::uncached(Error::generic(
            "Custom embedder backend set but no embedder was provided",
        ))),
        "openai" => openai(settings),
        "ollama" => ollama(settings),
        other => Err(BuildError::uncached(Error::Validation(format!(
            "Unknown embedding backend: '{other}'. Supported: onnx, openai, ollama."
        )))),
    }
}

#[cfg(feature = "onnx")]
fn onnx() -> Result<Box<dyn Embedder>, BuildError> {
    super::OnnxEmbedder::new()
        .map(|e| Box::new(e) as Box<dyn Embedder>)
        .map_err(|e| BuildError::cached(e, true))
}

#[cfg(not(feature = "onnx"))]
fn onnx() -> Result<Box<dyn Embedder>, BuildError> {
    Err(BuildError::uncached(Error::embed_msg(
        "ONNX embedding backend requested but 'onnx' feature is disabled at compile time. \
         Rebuild with --features onnx or use openai/ollama backend.",
    )))
}

fn or_default(value: &str, default: &str) -> String {
    if value.is_empty() {
        default.to_string()
    } else {
        value.to_string()
    }
}

fn openai(cfg: &EmbeddingSettings) -> Result<Box<dyn Embedder>, BuildError> {
    let dims = if cfg.dims == 0 {
        super::openai::DEFAULT_DIMS
    } else {
        cfg.dims
    };
    super::OpenAiEmbedder::new(
        &cfg.api_key,
        &or_default(&cfg.model, super::openai::DEFAULT_MODEL),
        &or_default(&cfg.base_url, super::openai::DEFAULT_BASE_URL),
        &cfg.endpoint_path,
        dims,
    )
    .map(|e| Box::new(e) as Box<dyn Embedder>)
    .map_err(|e| BuildError::cached(e, false))
}

fn ollama(cfg: &EmbeddingSettings) -> Result<Box<dyn Embedder>, BuildError> {
    let dims = if cfg.dims == 0 {
        super::ollama::DEFAULT_DIMS
    } else {
        cfg.dims
    };
    super::OllamaEmbedder::new(
        &or_default(&cfg.base_url, super::ollama::DEFAULT_BASE_URL),
        &or_default(&cfg.model, super::ollama::DEFAULT_MODEL),
        dims,
    )
    .map(|e| Box::new(e) as Box<dyn Embedder>)
    .map_err(|e| BuildError::cached(e, false))
}

/// Refuse to silently mix vectors of different dimensions in one index (#337).
/// `allow_mismatch` is the escape hatch: it lets the store be opened with a
/// different backend to run `uteke repair`, which rebuilds the vectors.
pub(crate) fn check_dims(
    embedder: &dyn Embedder,
    index_dims: usize,
    backend: &str,
    allow_mismatch: bool,
) -> Result<(), Error> {
    let backend_dims = embedder.dims();
    if index_dims != backend_dims && !allow_mismatch {
        return Err(Error::Validation(format!(
            "Embedding dimension mismatch: index has {index_dims}d vectors but backend '{backend}' produces {backend_dims}d. \
             Rebuild the index (`UTEKE_ALLOW_DIM_MISMATCH=1 uteke repair`) or switch backend."
        )));
    }
    Ok(())
}

/// Serve repeated texts from the persistent cache (#896). A cache that cannot
/// be opened is not fatal: the embedder is returned as it was.
pub(crate) fn with_cache(inner: Box<dyn Embedder>, path: Option<&Path>) -> Box<dyn Embedder> {
    let Some(path) = path else {
        return inner;
    };
    match super::EmbeddingCache::open(path) {
        Ok(cache) => {
            tracing::debug!(cache_path = %path.display(), "Persistent embedding cache enabled");
            Box::new(super::CachingEmbedder::with_cache(inner, cache))
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to init embedding cache, using uncached embedder");
            inner
        }
    }
}

/// Add the cloud fallback when it is fully configured; `primary_dims` are the
/// dimensions the primary was validated against.
pub(crate) fn with_fallback(
    primary: Box<dyn Embedder>,
    fb: &FallbackSettings,
    primary_dims: usize,
) -> Result<Box<dyn Embedder>, Error> {
    if !fb.is_configured() {
        return Ok(primary);
    }
    tracing::info!("Embedding fallback configured — wrapping primary with cloud backup");
    // The fallback is a backup: one that cannot be built is a warning and the
    // primary runs alone, like a cache that cannot be opened. Failing here
    // would drop the primary and rebuild it (a model load) on every call.
    let cloud = match super::OpenAiEmbedder::new(
        &fb.api_key,
        &fb.model,
        &fb.base_url,
        &fb.endpoint_path,
        primary_dims,
    ) {
        Ok(cloud) => cloud,
        Err(e) => {
            tracing::warn!(error = %e, "Embedding fallback could not be built, using the primary alone");
            return Ok(primary);
        }
    };
    Ok(Box::new(super::FallbackEmbedder::new(
        primary,
        Some(Box::new(cloud)),
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(usize);
    impl Embedder for Fixed {
        fn embed(&self, _: &str) -> Result<Vec<f32>, Error> {
            Ok(vec![0.0; self.0])
        }
        fn dims(&self) -> usize {
            self.0
        }
        fn max_seq_len(&self) -> usize {
            8
        }
        fn name(&self) -> &str {
            "fixed"
        }
    }

    fn settings(model: &str, dims: usize) -> EmbeddingSettings {
        EmbeddingSettings {
            api_key: "k".into(),
            base_url: "http://localhost:1/v1".into(),
            endpoint_path: String::new(),
            model: model.into(),
            dims,
        }
    }

    #[test]
    fn each_backend_arm_builds_on_its_own() {
        let o = build_primary("openai", &settings("", 0)).ok().unwrap();
        assert_eq!(o.name(), "openai");
        assert_eq!(o.dims(), super::super::openai::DEFAULT_DIMS);
        let o = build_primary("openai", &settings("m", 99)).ok().unwrap();
        assert_eq!(o.dims(), 99);
        let o = build_primary("ollama", &settings("", 0)).ok().unwrap();
        assert_eq!(o.name(), "ollama");
        assert_eq!(o.dims(), super::super::ollama::DEFAULT_DIMS);
    }

    #[test]
    fn an_openai_arm_without_a_key_fails_transiently() {
        let e = build_primary("openai", &EmbeddingSettings::default())
            .err()
            .unwrap();
        assert!(e.error.to_string().contains("requires an API key"));
        assert_eq!(e.cache, Some(false), "cached as a transient failure");
    }

    #[test]
    fn a_bad_backend_is_reported_every_time() {
        let e = build_primary("nope", &EmbeddingSettings::default())
            .err()
            .unwrap();
        assert!(
            e.error
                .to_string()
                .contains("Unknown embedding backend: 'nope'")
        );
        assert_eq!(e.cache, None);
        let e = build_primary("custom", &EmbeddingSettings::default())
            .err()
            .unwrap();
        assert_eq!(e.cache, None);
    }

    #[test]
    fn the_dimension_check_refuses_unless_the_escape_hatch_is_set() {
        assert!(check_dims(&Fixed(768), 768, "x", false).is_ok());
        let err = check_dims(&Fixed(1536), 768, "openai", false).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("index has 768d") && msg.contains("1536d"),
            "{msg}"
        );
        assert!(check_dims(&Fixed(1536), 768, "openai", true).is_ok());
    }

    #[test]
    fn the_cache_decorator_is_applied_only_with_a_path() {
        let plain = with_cache(Box::new(Fixed(4)), None);
        assert_eq!(plain.name(), "fixed");
        let dir = tempfile::tempdir().unwrap();
        let cached = with_cache(Box::new(Fixed(4)), Some(&dir.path().join("c.db")));
        assert_eq!(cached.dims(), 4);
        assert_eq!(cached.embed("a").unwrap().len(), 4);
        // An unopenable path leaves the embedder as it was.
        let bad = with_cache(
            Box::new(Fixed(4)),
            Some(Path::new("/nonexistent-dir/x/c.db")),
        );
        assert_eq!(bad.name(), "fixed");
    }

    #[test]
    fn the_fallback_decorator_needs_a_complete_configuration() {
        let none = with_fallback(Box::new(Fixed(4)), &FallbackSettings::default(), 4).unwrap();
        assert_eq!(none.name(), "fixed");
        let fb = FallbackSettings {
            api_key: "k".into(),
            base_url: "http://localhost:1/v1".into(),
            endpoint_path: String::new(),
            model: "m".into(),
        };
        let wrapped = with_fallback(Box::new(Fixed(4)), &fb, 4).unwrap();
        assert_eq!(wrapped.dims(), 4);
        assert_ne!(wrapped.name(), "fixed");
        // A fallback that cannot be built leaves the primary in place.
        let bad = FallbackSettings {
            base_url: "not a url".into(),
            ..fb
        };
        let alone = with_fallback(Box::new(Fixed(4)), &bad, 4).unwrap();
        assert_eq!(alone.name(), "fixed");
    }

    #[test]
    fn build_applies_primary_then_dims_then_decorators() {
        let fb = FallbackSettings::default();
        let s = settings("", 0);
        let plan = Plan {
            backend: "openai",
            settings: &s,
            cache_path: None,
            fallback: &fb,
            index_dims: super::super::openai::DEFAULT_DIMS,
            allow_dim_mismatch: false,
        };
        assert!(build(&plan).is_ok());
        let wrong = Plan {
            index_dims: 3,
            ..plan
        };
        let err = build(&wrong).err().unwrap();
        assert!(err.error.to_string().contains("dimension mismatch"));
        assert_eq!(err.cache, None, "a dimension error is not an init error");
    }
}
