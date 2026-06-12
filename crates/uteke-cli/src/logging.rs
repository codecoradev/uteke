//! Logging setup: console + daily rotating file.
//!
//! Console shows WARN by default (DEBUG with --verbose).
//! File always captures DEBUG to `~/.uteke/uteke.log` with daily rotation.

use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

/// Initialize logging. Returns a guard that must stay alive for the
/// file writer to continue flushing.
pub fn init(verbose: bool) -> Box<dyn std::any::Any + Send> {
    let log_dir = dirs::home_dir()
        .map(|h| h.join(".uteke"))
        .filter(|d| d.is_dir() || std::fs::create_dir_all(d).is_ok())
        .unwrap_or_else(|| {
            // Fallback to temp dir instead of cwd
            std::env::temp_dir().join("uteke")
        });

    let file_appender = tracing_appender::rolling::daily(&log_dir, "uteke.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_filter(EnvFilter::from_default_env().add_directive(Level::DEBUG.into()));

    let console_level = if verbose { Level::DEBUG } else { Level::WARN };
    let console_layer = tracing_subscriber::fmt::layer()
        .with_filter(EnvFilter::from_default_env().add_directive(console_level.into()));

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    Box::new(guard)
}
