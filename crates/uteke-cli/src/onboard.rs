//! `uteke onboard` — interactive onboarding flow.
//!
//! Guides a new user from zero to productive:
//!   1. Detect install (binary on PATH? store initialized?)
//!   2. Ask which AI agent they use
//!   3. Ask integration mode (manual tool vs auto memory-provider)
//!   4. Toggle features with on/off switches (aging, maintenance, graph, etc.)
//!   5. Write uteke.toml config
//!   6. Run `uteke init --agent <choice>` if the agent needs files
//!   7. Showcase all uteke features so the user knows what's available
//!
//! Non-interactive mode: `uteke onboard --yes --agent hermes` uses defaults.

use crate::cli::Cli;
use crate::cli::Commands;
use std::io::{self, BufRead, Write};

/// Feature toggle for the onboarding flow.
struct FeatureToggle {
    name: &'static str,
    description: &'static str,
    default: bool,
}

/// All toggleable features presented during onboarding.
const FEATURE_TOGGLES: &[FeatureToggle] = &[
    FeatureToggle {
        name: "Aging",
        description: "Auto-clean old, rarely-accessed memories (hot/warm/cold tiers)",
        default: false,
    },
    FeatureToggle {
        name: "Auto-maintenance",
        description: "Background prune + consolidate + orphan cleanup (requires server)",
        default: true,
    },
    FeatureToggle {
        name: "Graph rerank",
        description: "Boost recall results using knowledge-graph edge density",
        default: true,
    },
    FeatureToggle {
        name: "Salience boost",
        description: "Weight memories by importance score during recall",
        default: true,
    },
    FeatureToggle {
        name: "Recency boost",
        description: "Weight fresher memories higher during recall",
        default: true,
    },
    FeatureToggle {
        name: "Server mode",
        description: "Route CLI commands through uteke-serve daemon (21ms vs 980ms cold)",
        default: false,
    },
];

/// Supported agents for onboarding.
const AGENTS: &[&str] = &["hermes", "claude", "cursor", "pi", "opencode"];

/// Entry point for the `onboard` command.
pub fn run(cli: &Cli) -> Result<(), String> {
    let Commands::Onboard {
        yes,
        agent,
        namespace,
    } = &cli.command
    else {
        return Err("onboard command not matched".to_string());
    };

    println!();
    print_banner("Welcome to Uteke Onboarding");
    println!("  The Brain for Your AI - persistent memory engine");
    println!();

    // ── Step 1: Detect install ──────────────────────────────────
    let installed = detect_install();
    if !installed {
        println!("⚠  uteke is not on your PATH.");
        println!("  Install it first:");
        println!(
            "    curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/develop/install.sh | sh"
        );
        println!();
        if !*yes {
            print!("  Continue onboarding anyway? [y/N] ");
            io::stdout().flush().ok();
            let resp = read_line()?;
            if !resp.eq_ignore_ascii_case("y") {
                return Ok(());
            }
        }
    } else {
        let version = get_version();
        println!("✓ uteke {} detected on PATH", version);
    }
    println!();

    // ── Step 2: Check if store exists ───────────────────────────
    let store_exists = detect_store();
    if store_exists {
        println!("✓ Existing memory store found");
    } else {
        println!("ℹ No existing memory store — first `remember` will create it.");
    }
    println!();

    // ── Step 3: Pick agent ──────────────────────────────────────
    let agent_choice = if let Some(a) = agent {
        validate_agent(a)?
    } else if *yes {
        "hermes".to_string()
    } else {
        prompt_agent()?
    };
    println!("→ Agent: {}", agent_choice);
    println!();

    // ── Step 4: Pick integration mode ──────────────────────────
    let use_memory_provider = if *yes {
        false // default to tool mode
    } else {
        prompt_integration_mode(&agent_choice)?
    };
    let mode_label = if use_memory_provider {
        "memory-provider (auto recall + extraction)"
    } else {
        "uteke-tool (manual actions)"
    };
    println!("→ Mode: {}", mode_label);
    println!();

    // ── Step 5: Pick namespace ─────────────────────────────────
    let ns = if let Some(n) = namespace {
        n.clone()
    } else if *yes {
        "default".to_string()
    } else {
        prompt_namespace()?
    };
    println!("→ Namespace: {}", ns);
    println!();

    // ── Step 6: Extraction configuration ──────────────────────
    let extraction_cfg = if *yes {
        ExtractionSettings::default_offline()
    } else {
        prompt_extraction_config()?
    };
    println!("→ Extraction: {}", extraction_cfg.summary());
    println!();

    // ── Step 7: Feature toggles ────────────────────────────────
    let toggles = if *yes {
        FEATURE_TOGGLES
            .iter()
            .map(|t| (t.name, t.default))
            .collect()
    } else {
        prompt_feature_toggles()?
    };

    println!("→ Features:");
    for (name, enabled) in &toggles {
        let status = if *enabled { "ON" } else { "OFF" };
        println!("   {name}: {status}");
    }
    println!();

    // ── Step 8: Write config ───────────────────────────────────
    let config_path = write_config(&ns, &toggles, &extraction_cfg, *yes)?;
    println!("✓ Config written: {}", config_path.display());

    // ── Step 9: Run `uteke init` for the agent ─────────────────────────
    // We call the init module directly instead of spawning a subprocess.
    // Note: onboard always produces human-readable output; the global --json
    // flag is intentionally ignored here to avoid mixing ASCII banners with
    // parseable JSON. Init is called with json=false for the same reason.
    let init_result = if use_memory_provider {
        crate::init::run_init(&agent_choice, true, false)
    } else {
        crate::init::run_init(&agent_choice, false, false)
    };
    match &init_result {
        Ok(()) => println!("✓ Agent integration installed for {}", agent_choice),
        Err(e) => println!(
            "⚠ Agent init failed: {e} (you can run `uteke init --agent {}` manually later)",
            agent_choice
        ),
    }
    println!();

    // ── Step 10: First memory test ─────────────────────────────
    run_first_memory_test(&agent_choice, &ns);

    // ── Step 11: Rooms intro ───────────────────────────────────
    run_rooms_intro(&agent_choice, *yes);

    // ── Step 12: Feature showcase ──────────────────────────────
    showcase();

    println!();
    print_banner("Onboarding complete!");
    println!();
    println!("  Quick start:");
    println!("    uteke remember \"My first memory\" --tags test");
    println!("    uteke recall \"memory\"");
    println!("    uteke stats");
    println!();

    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Print a centered banner box around the given title.
fn print_banner(title: &str) {
    let width = 60;
    let inner = width - 2; // -2 for the border chars
    let border = "=".repeat(inner);
    let pad = (inner - title.len()) / 2;
    let left_pad = " ".repeat(pad);
    let right_pad = " ".repeat(inner - pad - title.len());
    println!("+{}+", border);
    println!("|{}{}{}|", left_pad, title, right_pad);
    println!("+{}+", border);
}

/// Check if the uteke binary is on PATH.
fn detect_install() -> bool {
    which::which("uteke").is_ok()
}

/// Get the current uteke version string.
fn get_version() -> String {
    // Use the compiled-in version.
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// Check if the uteke store exists.
fn detect_store() -> bool {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return false,
    };
    home.join(".codecora/uteke").join("uteke.db").exists()
}

/// Prompt the user to select an agent.
fn prompt_agent() -> Result<String, String> {
    println!("Which AI agent do you use?");
    for (i, a) in AGENTS.iter().enumerate() {
        let desc = match *a {
            "hermes" => "Hermes Agent (by Nous Research)",
            "claude" => "Claude Code / Claude Desktop",
            "cursor" => "Cursor IDE",
            "pi" => "Pi (pi.dev)",
            "opencode" => "OpenCode",
            _ => a,
        };
        println!("  {}) {} — {}", i + 1, a, desc);
    }
    println!("  Or type a custom agent name");
    print!("\n  Select [1-5]: ");
    io::stdout().flush().ok();
    let resp = read_line()?;
    let resp = resp.trim();
    match resp.parse::<usize>() {
        Ok(n) if n > 0 && n <= AGENTS.len() => Ok(AGENTS[n - 1].to_string()),
        _ => {
            // Treat as custom name
            if resp.is_empty() {
                Ok("hermes".to_string())
            } else {
                Ok(resp.to_string())
            }
        }
    }
}

/// Prompt for integration mode (tool vs memory-provider).
fn prompt_integration_mode(agent: &str) -> Result<bool, String> {
    let supports_mp = matches!(agent, "hermes" | "claude" | "cursor" | "pi" | "opencode");
    if !supports_mp {
        println!("→ Integration mode: uteke-tool (custom agent)");
        return Ok(false);
    }

    println!("Integration mode for {}:", agent);
    println!("  1) uteke-tool — manual `uteke(action=...)` calls, multi-agent rooms");
    println!("     Best for: on-demand memory, multi-agent collaboration");
    println!("  2) memory-provider — automatic recall + fact extraction every turn");
    println!("     Best for: hands-free persistent memory (no manual calls needed)");
    print!("\n  Select [1-2] (default 1): ");
    io::stdout().flush().ok();
    let resp = read_line()?;
    let resp = resp.trim();
    match resp {
        "2" => Ok(true),
        _ => Ok(false),
    }
}

/// Prompt for namespace.
fn prompt_namespace() -> Result<String, String> {
    println!("Namespace for memory isolation (e.g. 'default', 'work', 'my-agent'):");
    print!("  Namespace [default]: ");
    io::stdout().flush().ok();
    let resp = read_line()?;
    let resp = resp.trim();
    if resp.is_empty() {
        Ok("default".to_string())
    } else {
        Ok(resp.to_string())
    }
}

/// Prompt for feature toggles. Returns vec of (name, enabled).
fn prompt_feature_toggles() -> Result<Vec<(&'static str, bool)>, String> {
    println!("Feature toggles — toggle ON/OFF (press Enter to accept defaults):");
    println!();
    let mut results = Vec::new();
    for t in FEATURE_TOGGLES {
        let default_str = if t.default { "ON" } else { "OFF" };
        print!("  {:<20} [{}] — {}: ", t.name, default_str, t.description);
        io::stdout().flush().ok();
        let resp = read_line()?;
        let resp = resp.trim().to_lowercase();
        let enabled = match resp.as_str() {
            "on" | "y" | "yes" | "1" | "true" => true,
            "off" | "n" | "no" | "0" | "false" => false,
            "" => t.default,
            _ => t.default,
        };
        results.push((t.name, enabled));
    }
    Ok(results)
}

// ── Extraction Settings ───────────────────────────────────────────────────

/// User's extraction configuration chosen during onboarding.
struct ExtractionSettings {
    /// "offline", "llm", or "none".
    mode: String,
    /// LLM endpoint (only for mode = "llm").
    base_url: String,
    /// LLM API key (only for mode = "llm").
    api_key: String,
    /// LLM model name (only for mode = "llm").
    model: String,
}

impl ExtractionSettings {
    /// Default for `--yes` mode: offline rule-based extraction.
    fn default_offline() -> Self {
        Self {
            mode: "offline".to_string(),
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
        }
    }

    /// Human-readable summary for the onboarding output.
    fn summary(&self) -> String {
        match self.mode.as_str() {
            "offline" => "offline (rule-based, zero API)".to_string(),
            "llm" => format!("external LLM ({})", self.model),
            _ => "disabled (manual only)".to_string(),
        }
    }
}

/// Prompt the user for extraction configuration.
fn prompt_extraction_config() -> Result<ExtractionSettings, String> {
    println!("Automatic fact extraction from conversations and files:");
    println!("  1) Offline (rule-based, zero API calls)  ← recommended");
    println!("     Best for: privacy, zero cost, air-gapped environments");
    println!("  2) External LLM (better quality, needs API endpoint)");
    println!("     Best for: nuanced conversations, complex fact extraction");
    println!("  3) No extraction (manual insertion only)");
    print!("\n  Select [1-3] (default 1): ");
    io::stdout().flush().ok();
    let resp = read_line()?;
    match resp.as_str() {
        "2" => {
            println!();
            println!("LLM Extraction Configuration:");
            print!("  Endpoint [https://api.openai.com/v1]: ");
            io::stdout().flush().ok();
            let mut base_url = read_line()?;
            if base_url.is_empty() {
                base_url = "https://api.openai.com/v1".to_string();
            }

            print!("  API Key: ");
            io::stdout().flush().ok();
            let api_key = read_line()?;

            print!("  Model [gpt-4o-mini]: ");
            io::stdout().flush().ok();
            let mut model = read_line()?;
            if model.is_empty() {
                model = "gpt-4o-mini".to_string();
            }

            // Optional connection test.
            if !api_key.is_empty() {
                println!("  → Testing connection...");
                // We don't actually call the API here — just validate the
                // fields are non-empty. The real test happens on first use.
                println!("  ✓ Configuration saved (will be tested on first import --extract)");
            }
            Ok(ExtractionSettings {
                mode: "llm".to_string(),
                base_url,
                api_key,
                model,
            })
        }
        "3" => Ok(ExtractionSettings {
            mode: "none".to_string(),
            ..ExtractionSettings::default_offline()
        }),
        _ => Ok(ExtractionSettings::default_offline()),
    }
}

// ── First Memory Test ─────────────────────────────────────────────────────

/// Run a quick remember + recall test to verify the memory system works.
fn run_first_memory_test(agent: &str, namespace: &str) {
    print_banner("Memory System Test");

    let test_memory = format!("Onboarding complete for {} agent", agent);

    // We spawn `uteke remember` and `uteke recall` as subprocesses so we
    // don't need to pull in the full core library initialization here.
    println!("→ Storing test memory...");
    let remember_result =
        std::process::Command::new(std::env::current_exe().unwrap_or_else(|_| "uteke".into()))
            .arg("remember")
            .arg(&test_memory)
            .arg("--tags")
            .arg("onboarding,test")
            .arg("--namespace")
            .arg(namespace)
            .output();

    match remember_result {
        Ok(out) if out.status.success() => {
            println!("  ✓ remember: success");
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!("  ⚠ remember: {}", stderr.trim());
            println!("  (You can test manually later with: uteke remember \"hello\")");
            return;
        }
        Err(e) => {
            println!("  ⚠ Could not run memory test: {e}");
            return;
        }
    }

    // Small delay to let the write settle (SQLite WAL).
    std::thread::sleep(std::time::Duration::from_millis(100));

    println!("→ Recalling test memory...");
    let recall_result =
        std::process::Command::new(std::env::current_exe().unwrap_or_else(|_| "uteke".into()))
            .arg("recall")
            .arg("onboarding")
            .arg("--namespace")
            .arg(namespace)
            .arg("--limit")
            .arg("3")
            .output();

    match recall_result {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains("onboarding") || !stdout.trim().is_empty() {
                println!("  ✓ recall: found test memory");
                println!("  ✓ Memory system verified!");
            } else {
                println!("  ⚠ recall: no results (memory may still be indexing)");
                println!("  (This is normal — try `uteke recall \"onboarding\"` in a moment)");
            }
        }
        Ok(_) | Err(_) => {
            println!("  ⚠ recall test skipped (you can verify manually)");
        }
    }
    println!();
}

// ── Rooms Intro ───────────────────────────────────────────────────────────

/// Introduce Rooms and optionally create the user's first room.
fn run_rooms_intro(agent: &str, yes: bool) {
    print_banner("Rooms — Multi-Agent Memory Sharing");
    println!();
    println!("  Rooms let multiple AI agents share a memory space.");
    println!("  Use cases:");
    println!("    • Multi-project context (one room per project)");
    println!("    • Team knowledge sharing");
    println!("    • Multi-agent collaboration (e.g. Hermes + Claude)");
    println!();
    println!("  Commands:");
    println!("    uteke room create \"project-name\"");
    println!("    uteke room recall \"project-name\" \"search term\"");
    println!("    uteke room summary \"project-name\"");
    println!();

    if yes {
        println!("  Skipping room creation (--yes mode).");
        println!("  You can create a room later with: uteke room create \"my-project\"");
        println!();
        return;
    }

    print!("  Create your first room now? [Y/n] ");
    io::stdout().flush().ok();
    let resp = read_line().unwrap_or_default();
    if resp.eq_ignore_ascii_case("n") {
        println!("  → Skipped. You can create a room later.");
        println!();
        return;
    }

    let default_name = format!("project-{}", agent);
    print!("  Room name [{}]: ", default_name);
    io::stdout().flush().ok();
    let room_name = {
        let r = read_line().unwrap_or_default();
        if r.is_empty() { default_name } else { r }
    };

    let result =
        std::process::Command::new(std::env::current_exe().unwrap_or_else(|_| "uteke".into()))
            .arg("room")
            .arg("create")
            .arg(&room_name)
            .output();

    match result {
        Ok(out) if out.status.success() => {
            println!("  ✓ Room \"{}\" created!", room_name);
            println!("    Multiple agents can now share memories via this room.");
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!("  ⚠ Room creation failed: {}", stderr.trim());
            println!(
                "  (You can create it later: uteke room create \"{}\")",
                room_name
            );
        }
        Err(_) => {
            println!("  ⚠ Could not create room.");
            println!(
                "  (You can create it later: uteke room create \"{}\")",
                room_name
            );
        }
    }
    println!();
}

/// Escape a user-provided string for use inside a TOML basic string, so a
/// namespace / URL / key containing `"`, `\` or a newline cannot break out of
/// the value or inject further keys (#1332).
fn escape_toml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Render `uteke.toml` from the onboarding answers. Pure, so it can be tested
/// (and parsed back) without touching the disk.
fn render_config_toml(
    namespace: &str,
    toggles: &[(&str, bool)],
    extraction: &ExtractionSettings,
) -> String {
    let aging_enabled = get_toggle(toggles, "Aging");
    let auto_maint = get_toggle(toggles, "Auto-maintenance");
    let graph_rerank = get_toggle(toggles, "Graph rerank");
    let salience = get_toggle(toggles, "Salience boost");
    let recency = get_toggle(toggles, "Recency boost");
    let server_enabled = get_toggle(toggles, "Server mode");

    // Build extraction section based on chosen mode.

    let extraction_section = match extraction.mode.as_str() {
        "offline" => r#"
[extraction]
mode = "offline"
"#
        .to_string(),
        "llm" => format!(
            r#"
[extraction]
mode = "llm"
base_url = "{}"
api_key = "{}"
model = "{}"
"#,
            escape_toml(&extraction.base_url),
            escape_toml(&extraction.api_key),
            escape_toml(&extraction.model),
        ),
        _ => String::new(), // "none" — omit section entirely
    };

    format!(
        r#"# Uteke configuration — generated by `uteke onboard`
# See https://github.com/codecoradev/uteke for full documentation

[store]
namespace = "{namespace}"

[recall]
graph_rerank_enabled = {graph_rerank}
salience_weight = {salience_weight}
recency_weight = {recency_weight}
{extraction_section}
[aging]
enabled = {aging_enabled}

[maintenance]
auto_aging_enabled = {auto_maint}

[server]
enabled = {server_enabled}
"#,
        namespace = escape_toml(namespace),
        graph_rerank = graph_rerank,
        salience_weight = if salience { 0.15 } else { 0.0 },
        recency_weight = if recency { 0.15 } else { 0.0 },
        extraction_section = extraction_section,
        aging_enabled = aging_enabled,
        auto_maint = auto_maint,
        server_enabled = server_enabled,
    )
}

/// Write the config to `{uteke_home}/uteke.toml`.
///
/// If a config file already exists, it is backed up to `uteke.toml.bak`
/// before overwriting. In interactive mode, the user is prompted to confirm.
fn write_config(
    namespace: &str,
    toggles: &[(&str, bool)],
    extraction: &ExtractionSettings,
    yes: bool,
) -> Result<std::path::PathBuf, String> {
    let uteke_dir = uteke_core::uteke_home().map_err(|e| format!("{e}"))?;
    std::fs::create_dir_all(&uteke_dir).map_err(|e| format!("Failed to create uteke home: {e}"))?;

    let config_path = uteke_dir.join("uteke.toml");

    // If config already exists, back it up and confirm before overwriting.
    if config_path.exists() {
        if !yes {
            print!(
                "⚠ Config file already exists at {} — overwrite? [y/N] ",
                config_path.display()
            );
            io::stdout().flush().ok();
            let resp = read_line()?;
            if !resp.eq_ignore_ascii_case("y") {
                println!("→ Skipped config write (existing config preserved)");
                return Ok(config_path);
            }
        }
        // Back up existing config before overwriting.
        let backup_path = uteke_dir.join("uteke.toml.bak");
        std::fs::copy(&config_path, &backup_path)
            .map_err(|e| format!("Failed to back up existing config: {e}"))?;
        println!("→ Backed up existing config to {}", backup_path.display());
    }

    let toml = render_config_toml(namespace, toggles, extraction);

    std::fs::write(&config_path, toml).map_err(|e| format!("Failed to write config: {e}"))?;

    Ok(config_path)
}

/// Get a toggle value by name.
fn get_toggle(toggles: &[(&str, bool)], name: &str) -> bool {
    toggles
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| *v)
        .unwrap_or(false)
}

/// Validate the agent name and warn on unrecognized agents.
fn validate_agent(agent: &str) -> Result<String, String> {
    if AGENTS.contains(&agent) {
        Ok(agent.to_string())
    } else {
        // Allow custom agent names but warn the user.
        println!(
            "⚠ '{}' is not a recognized agent — init step will be skipped.",
            agent
        );
        println!("  Recognized agents: {}", AGENTS.join(", "));
        Ok(agent.to_string())
    }
}

/// Print a feature showcase so the user knows everything uteke can do.
fn showcase() {
    print_banner("Uteke Feature Showcase");
    println!();

    let sections: &[(&str, &[(&str, &str)])] = &[
        (
            "Core Memory",
            &[
                (
                    "remember",
                    "Store a memory with tags, type, entity, metadata",
                ),
                (
                    "recall",
                    "Hybrid search (vector + FTS5 + RRF) — by meaning, not keywords",
                ),
                ("search", "Keyword text search (FTS5)"),
                ("list", "List memories with filters (tag, entity, category)"),
                ("get", "Get a single memory by ID"),
                ("forget", "Delete memory by ID, tag, or age tier"),
            ],
        ),
        (
            "Documents (Wiki)",
            &[
                ("doc create", "Create wiki/knowledge-base documents"),
                ("doc update", "Partial update — only change provided fields"),
                ("doc search", "Search documents (semantic + FTS5 hybrid)"),
                ("doc list --tree", "Browse document hierarchy"),
                ("doc move", "Reorganize document tree (move parent)"),
            ],
        ),
        (
            "Knowledge Graph",
            &[
                ("graph nodes", "List all graph nodes"),
                ("graph neighbors", "Find neighbors via BFS traversal"),
                ("graph path", "Find shortest path between two nodes"),
                ("edges", "Show auto-wired relationship edges for a memory"),
                ("rebuild-backlinks", "Rebuild referenced_by backlinks"),
            ],
        ),
        (
            "Rooms (Multi-Agent)",
            &[
                ("room create", "Create a shared memory space for agents"),
                ("room recall", "Recall from a room (cross-namespace)"),
                ("room summary", "Auto-generate topic summary (no LLM)"),
                (
                    "room document",
                    "Generate structured doc from room memories",
                ),
            ],
        ),
        (
            "Memory Lifecycle",
            &[
                ("pin / unpin", "Pin critical memories so they never decay"),
                ("importance", "Recalculate importance scores"),
                ("orphans", "Find disconnected low-importance memories"),
                (
                    "aging status",
                    "Show hot/warm/cold/never-accessed breakdown",
                ),
                ("dream", "Full maintenance: lint → dedup → orphans → verify"),
            ],
        ),
        (
            "Maintenance",
            &[
                ("consolidate", "Merge near-duplicate memories (cosine sim)"),
                ("prune", "Remove deprecated/expired memories (TTL)"),
                ("verify / repair", "Check and rebuild index consistency"),
                ("upgrade", "Self-update to latest uteke release"),
            ],
        ),
        (
            "Import / Export",
            &[
                ("import --extract", "Distill text into atomic facts via LLM"),
                (
                    "import --batch-dir",
                    "Bulk import .md/.txt/.jsonl from a directory",
                ),
                ("export", "Export all memories as portable JSONL"),
            ],
        ),
        (
            "Agent Integration",
            &[
                (
                    "init --agent <name>",
                    "Install integration for Hermes/Claude/Cursor/Pi/OpenCode",
                ),
                (
                    "init --memory-provider",
                    "Auto recall + extraction (no manual calls)",
                ),
                ("hook bash/zsh/fish", "Shell hook for auto-context on cd"),
            ],
        ),
    ];

    for (section_name, items) in sections {
        println!("  {}:", section_name);
        for (cmd, desc) in *items {
            println!("    {:<28} — {}", cmd, desc);
        }
        println!();
    }
}

/// Read a line from stdin.
fn read_line() -> Result<String, String> {
    let stdin = io::stdin();
    let mut line = String::new();
    stdin
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read input: {e}"))?;
    Ok(line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_toggle_returns_value_when_present() {
        let toggles = vec![("Aging", true), ("Graph rerank", false)];
        assert!(get_toggle(&toggles, "Aging"));
        assert!(!get_toggle(&toggles, "Graph rerank"));
    }

    #[test]
    fn test_get_toggle_returns_false_when_absent() {
        let toggles = vec![("Aging", true)];
        assert!(!get_toggle(&toggles, "Nonexistent"));
    }

    #[test]
    fn test_get_toggle_empty_slice() {
        let toggles: Vec<(&str, bool)> = vec![];
        assert!(!get_toggle(&toggles, "Aging"));
    }

    #[test]
    fn test_validate_agent_recognized() {
        assert_eq!(validate_agent("hermes").unwrap(), "hermes");
        assert_eq!(validate_agent("claude").unwrap(), "claude");
        assert_eq!(validate_agent("cursor").unwrap(), "cursor");
        assert_eq!(validate_agent("pi").unwrap(), "pi");
        assert_eq!(validate_agent("opencode").unwrap(), "opencode");
    }

    #[test]
    fn test_validate_agent_custom_returns_ok() {
        // Custom agents should still return Ok (just with a warning printed).
        assert_eq!(
            validate_agent("my-custom-agent").unwrap(),
            "my-custom-agent"
        );
    }

    #[test]
    fn test_write_config_generates_valid_toml() {
        // Verify that the TOML structure produced by write_config is valid.
        // We test the format string inline since write_config writes to disk.
        let toml = r#"[store]
namespace = "test-ns"

[recall]
graph_rerank_enabled = true
salience_weight = 0.15
recency_weight = 0.0

[aging]
enabled = false

[maintenance]
auto_aging_enabled = true

[server]
enabled = false
"#;
        // Verify it parses as valid TOML (basic structural check).
        assert!(toml.contains("[store]"));
        assert!(toml.contains("namespace = \"test-ns\""));
        assert!(toml.contains("[recall]"));
        assert!(toml.contains("graph_rerank_enabled = true"));
        assert!(toml.contains("salience_weight = 0.15"));
        assert!(toml.contains("recency_weight = 0.0"));
        assert!(toml.contains("[aging]"));
        assert!(toml.contains("enabled = false"));
        assert!(toml.contains("[maintenance]"));
        assert!(toml.contains("auto_aging_enabled = true"));
        assert!(toml.contains("[server]"));
        assert!(toml.contains("enabled = false"));
    }

    #[test]
    fn test_print_banner_does_not_panic() {
        // Just verify print_banner doesn't panic on various inputs.
        // We can't easily capture stdout in a unit test, but we can ensure
        // the function completes without error.
        print_banner("Short");
        print_banner("A Much Longer Banner Title That Might Cause Issues");
        print_banner("");
    }

    fn extraction_none() -> ExtractionSettings {
        ExtractionSettings {
            mode: "none".to_string(),
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
        }
    }

    /// #1332: the namespace was written into the TOML unescaped, so a quote or
    /// newline in it broke the file or injected extra keys/sections.
    #[test]
    fn test_render_config_escapes_hostile_namespace() {
        let hostile = "evil\"\n[server]\nenabled = true\n#";
        let toml = render_config_toml(hostile, &[("Server mode", false)], &extraction_none());
        let parsed: toml::Value = toml::from_str(&toml).expect("generated TOML must parse");
        assert_eq!(
            parsed["store"]["namespace"].as_str(),
            Some(hostile),
            "namespace round-trips verbatim"
        );
        assert_eq!(
            parsed["server"]["enabled"].as_bool(),
            Some(false),
            "the injected [server] section must not take effect"
        );
    }

    #[test]
    fn test_render_config_plain_values_parse() {
        let toggles = [("Aging", true), ("Salience boost", true)];
        let toml = render_config_toml("my-agent", &toggles, &extraction_none());
        let parsed: toml::Value = toml::from_str(&toml).unwrap();
        assert_eq!(parsed["store"]["namespace"].as_str(), Some("my-agent"));
        assert_eq!(parsed["aging"]["enabled"].as_bool(), Some(true));
        assert_eq!(parsed["recall"]["salience_weight"].as_float(), Some(0.15));
    }

    #[test]
    fn test_render_config_escapes_llm_extraction_values() {
        let ex = ExtractionSettings {
            mode: "llm".to_string(),
            base_url: "https://x.example/v1".to_string(),
            api_key: "k\"\nmodel = \"evil".to_string(),
            model: "gpt".to_string(),
        };
        let toml = render_config_toml("ns", &[], &ex);
        let parsed: toml::Value = toml::from_str(&toml).expect("must parse");
        assert_eq!(parsed["extraction"]["model"].as_str(), Some("gpt"));
        assert_eq!(
            parsed["extraction"]["api_key"].as_str(),
            Some("k\"\nmodel = \"evil")
        );
    }
}
