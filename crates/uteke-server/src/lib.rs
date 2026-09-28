//! Uteke HTTP Server library exports.
//!
//! The binary (`uteke-serve`) lives in `main.rs`.
//! This lib exists so that `crates/docgen` can access request/response types
//! and the API route registry for documentation generation.

pub mod api_registry;

pub mod types;
