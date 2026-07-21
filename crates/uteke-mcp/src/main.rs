//! Uteke MCP Server — Model Context Protocol interface for AI agents.
//!
//! Communicates via JSON-RPC over stdin/stdout (stdio transport).
//! Exposes uteke memory operations as MCP tools that AI coding agents
//! (Claude Code, Cursor, Copilot, etc.) can call directly.
//!
//! ## Usage
//!
//! Add to your MCP client config:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "uteke": {
//!       "command": "uteke-mcp",
//!       "args": []
//!     }
//!   }
//! }
//! ```

use std::io::{self, BufRead, Write};
use uteke_core::Uteke;

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    // Open uteke store. Prefer a repo-local store when the MCP server is
    // launched inside a project that ran `uteke init --project`: walk up from
    // cwd for `.uteke/uteke.toml` and use that repo-local `.uteke` directory.
    // Otherwise fall back to the global `~/.uteke` store.
    let store_path = resolve_store_path();

    let uteke = match Uteke::open(&store_path) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Failed to open uteke store: {e}");
            std::process::exit(1);
        }
    };

    // JSON-RPC over stdin/stdout (MCP stdio transport)
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Read error: {e}");
                break;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        // Delegate to the shared handler (#381).
        // None = notification (no response per JSON-RPC 2.0 §4.1).
        if let Some(response) = uteke_mcp::handle_jsonrpc(&uteke, &line) {
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
}

/// Resolve the store directory for the MCP server.
///
/// Walks up from the current directory looking for a repo-local
/// `.uteke/uteke.toml` (written by `uteke init --project`). When found, the
/// repo-local `.uteke` directory is used so the MCP server shares the same
/// per-project code index the CLI writes. Falls back to the global
/// `~/.uteke` store otherwise. `UTEKE_HOME` still overrides via `Uteke::open`.
fn resolve_store_path() -> std::path::PathBuf {
    if std::env::var_os("UTEKE_HOME").is_none() {
        if let Ok(cwd) = std::env::current_dir() {
            let mut dir: Option<&std::path::Path> = Some(cwd.as_path());
            while let Some(d) = dir {
                if d.join(".uteke").join("uteke.toml").is_file() {
                    return d.join(".uteke");
                }
                if d.join(".git").exists() {
                    break; // repo root without a project store -> use global
                }
                dir = d.parent();
            }
        }
    }
    dirs::home_dir()
        .map(|h| h.join(".uteke"))
        .expect("Cannot determine home directory")
}
