//! Uteke Core — persistent memory library for AI agents.
//!
//! # Example
//! ```no_run
//! use uteke_core::Uteke;
//!
//! let mut uteke = Uteke::open("~/.uteke")?;
//! uteke.remember("important context", &["tag1"])?;
//! let results = uteke.recall("query", 5)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

/// Uteke — AI agent memory engine.
pub struct Uteke {
    _path: std::path::PathBuf,
}

impl Uteke {
    /// Open or create a Uteke memory store at the given path.
    pub fn open(_path: impl AsRef<std::path::std::Path>) -> Result<Self, Error> {
        let path = _path.as_ref().to_path_buf();
        Ok(Self { _path: path })
    }
}

/// Uteke error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Embedding error: {0}")]
    Embedding(String),
}
