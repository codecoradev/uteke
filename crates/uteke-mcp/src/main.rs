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

    // Open uteke store
    let store_path = dirs::home_dir()
        .map(|h| h.join(".uteke"))
        .expect("Cannot determine home directory");

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
