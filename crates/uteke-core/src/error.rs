//! Error types and helpers for Uteke.

/// Uteke error type.
///
/// Error messages are designed to be user-friendly — internal details
/// (rusqlite codes, useark internals, ONNX session errors) are logged
/// via `tracing` and never exposed in the display message.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{message}")]
    Database { message: String },

    #[error("{message}")]
    Embedding { message: String },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Lock error: {context}")]
    Lock { context: String },
}

impl Error {
    // ── Convenience constructors with context ──────────────────────────

    /// Database error from rusqlite — strips internal details from message.
    pub fn db(context: &str, err: impl std::fmt::Display) -> Self {
        let message = Self::sanitize_db_error(context, &err.to_string());
        tracing::warn!(context, %err, "database error");
        Error::Database { message }
    }

    /// Database error with only a message (no source error).
    pub fn db_msg(message: impl Into<String>) -> Self {
        Error::Database {
            message: message.into(),
        }
    }

    /// Embedding error — strips ONNX/usearch internals.
    pub fn embed(context: &str, err: impl std::fmt::Display) -> Self {
        let message = Self::sanitize_embed_error(context, &err.to_string());
        tracing::warn!(context, %err, "embedding error");
        Error::Embedding { message }
    }

    /// Embedding error with only a message.
    pub fn embed_msg(message: impl Into<String>) -> Self {
        Error::Embedding {
            message: message.into(),
        }
    }

    /// Lock error — includes which lock and what operation.
    pub fn lock(context: impl Into<String>) -> Self {
        let context = context.into();
        tracing::warn!(%context, "lock error");
        Error::Lock { context }
    }

    // ── Sanitizers ─────────────────────────────────────────────────────

    /// Clean rusqlite error messages — keep constraint names, strip paths.
    fn sanitize_db_error(context: &str, err_str: &str) -> String {
        // Keep user-meaningful constraint info, strip internal details
        if err_str.contains("UNIQUE constraint") {
            format!("{context}: duplicate entry")
        } else if err_str.contains("NOT NULL constraint") {
            format!("{context}: missing required field")
        } else if err_str.contains("no such table") {
            format!("{context}: database schema error (run 'uteke doctor')")
        } else if err_str.contains("disk I/O error") || err_str.contains("IO_ERROR") {
            format!("{context}: storage error (check disk space and permissions)")
        } else if err_str.contains("database is locked") {
            format!("{context}: database busy — retry after a moment")
        } else if err_str.contains("corruption") || err_str.contains("malformed") {
            format!("{context}: database corruption (run 'uteke repair')")
        } else {
            format!("{context}: database operation failed")
        }
    }

    /// Clean embedding error messages — strip ONNX/useark internals.
    fn sanitize_embed_error(context: &str, err_str: &str) -> String {
        if err_str.contains("out of memory") || err_str.contains("OOM") {
            format!("{context}: out of memory (reduce batch size or use a smaller model)")
        } else if err_str.contains("model") || err_str.contains("session") {
            format!(
                "{context}: failed to load embedding model (check model files with 'uteke doctor')"
            )
        } else if err_str.contains("dimension") || err_str.contains("dims") {
            format!("{context}: vector dimension mismatch")
        } else {
            format!("{context}: embedding operation failed")
        }
    }
}

/// Helper to format bytes.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(1572864), "1.5 MB");
    }

    #[test]
    fn test_db_error_sanitization() {
        // UNIQUE constraint
        let e = Error::db(
            "insert",
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "UNIQUE constraint failed: memories.id",
            ),
        );
        let msg = e.to_string();
        assert!(msg.contains("duplicate entry"), "got: {msg}");
        assert!(
            !msg.contains("UNIQUE constraint failed"),
            "leaks internal: {msg}"
        );

        // Disk I/O
        let e = Error::db(
            "open",
            std::io::Error::new(std::io::ErrorKind::Other, "disk I/O error: /path/to/db"),
        );
        let msg = e.to_string();
        assert!(msg.contains("storage error"), "got: {msg}");
        assert!(!msg.contains("/path/to/db"), "leaks path: {msg}");

        // Generic
        let e = Error::db(
            "query",
            std::io::Error::new(std::io::ErrorKind::Other, "some random rusqlite error"),
        );
        let msg = e.to_string();
        assert!(msg.contains("database operation failed"), "got: {msg}");
        assert!(!msg.contains("rusqlite"), "leaks internal: {msg}");
    }

    #[test]
    fn test_embed_error_sanitization() {
        let e = Error::embed(
            "recall",
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to initialize ONNX session for model embeddinggemma",
            ),
        );
        let msg = e.to_string();
        assert!(msg.contains("embedding model"), "got: {msg}");
        assert!(!msg.contains("ONNX session"), "leaks internal: {msg}");
        assert!(!msg.contains("embeddinggemma"), "leaks model name: {msg}");

        let e = Error::embed(
            "embed",
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "out of memory allocating tensors",
            ),
        );
        let msg = e.to_string();
        assert!(msg.contains("out of memory"), "got: {msg}");
    }

    #[test]
    fn test_lock_error() {
        let e = Error::lock("index lock during insert");
        assert_eq!(e.to_string(), "Lock error: index lock during insert");
    }

    #[test]
    fn test_db_msg() {
        let e = Error::db_msg("Memory not found: abc-123");
        assert_eq!(e.to_string(), "Memory not found: abc-123");
    }

    #[test]
    fn test_embed_msg() {
        let e = Error::embed_msg("Model files not found");
        assert_eq!(e.to_string(), "Model files not found");
    }
}
