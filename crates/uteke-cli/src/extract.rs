//! Re-export extraction from uteke-core for CLI use.
//!
//! The Extractor and all supporting types now live in uteke-core so both
//! the CLI and the server can share them without circular dependencies.

#[allow(unused_imports)] // re-exports for crate::extract::* consumers
pub use uteke_core::extraction::{
    DEFAULT_BASE_URL, DEFAULT_ENDPOINT_PATH, DEFAULT_MAX_FACTS, DEFAULT_MODEL, ExtractionConfig,
    Extractor,
};
