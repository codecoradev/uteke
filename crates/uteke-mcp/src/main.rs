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

    // Open uteke store. Must go through uteke_home() — the canonical resolver the CLI
    // uses (UTEKE_HOME override, ~/.codecora/uteke, legacy ~/.uteke auto-migration).
    // Hardcoding ~/.uteke here pinned the MCP server to the pre-move legacy path, so it
    // silently opened an empty second store and never saw anything the CLI had written.
    let store_path = uteke_core::uteke_home().unwrap_or_else(|e| {
        eprintln!("Cannot determine uteke store path: {e}");
        std::process::exit(1);
    });

    let uteke = match Uteke::open(&store_path) {
        Ok(mut u) => {
            // #1464: secret filter on the write path ([privacy] in uteke.toml)
            u.set_privacy_config(uteke_core::privacy::load_config());
            u
        }
        Err(e) => {
            eprintln!("Failed to open uteke store: {e}");
            std::process::exit(1);
        }
    };

    // Background update check — writes to stderr so it doesn't corrupt the
    // JSON-RPC stdout channel. MCP clients capture stderr for logging.
    // Cache is 24h so most runs skip network entirely.
    // Detached thread — main loop blocks on stdin for hours, so join is impractical.
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(|| {
            if let Some(info) = uteke_core::update_check::check()
                .filter(uteke_core::update_check::UpdateInfo::is_update_available)
            {
                eprintln!("\n{}\n", info.banner());
            }
        });
    });

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
            // Detect broken pipe: if the parent process has disconnected,
            // writes to stdout will fail. Exit cleanly instead of hanging (#843).
            if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
                eprintln!("stdout write failed — parent process disconnected. Exiting.");
                break;
            }
        }
    }
}
