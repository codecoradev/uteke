//! Agent initialization commands (pi, claude, cursor).

use crate::cli::Cli;
use crate::cli::Commands;

/// Handle the Init command from the CLI.
pub(crate) fn run_init_command(cli: &Cli) -> Result<(), String> {
    if let Commands::Init {
        agent,
        memory_provider,
        project,
    } = &cli.command
    {
        if *project {
            return init_project(cli.json);
        }
        return run_init(agent, *memory_provider, cli.json);
    }
    Ok(())
}

/// Initialize a repo-local uteke store for per-project code memory.
///
/// Writes `<repo>/.uteke/uteke.toml` (store path relative to repo root so the
/// DB lives inside the repo) and appends the DB glob to `<repo>/.gitignore`.
/// Idempotent: existing files are preserved, entries not duplicated.
fn init_project(json: bool) -> Result<(), String> {
    let outcome = init_project_store(None)?;
    if json {
        let out = serde_json::json!({
            "root": outcome.root.display().to_string(),
            "namespace": outcome.namespace,
            "config": outcome.config_path.display().to_string(),
            "wrote_config": outcome.wrote_config,
            "updated_gitignore": outcome.updated_gitignore,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Initialized project store at {}", outcome.uteke_dir().display());
        println!("  namespace: {}", outcome.namespace);
        if !outcome.wrote_config {
            println!("  (config already existed, left unchanged)");
        }
        println!("Next: run `uteke index` to index this repo's source.");
    }
    Ok(())
}

/// Result of initializing a per-project store.
pub(crate) struct ProjectInit {
    pub root: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub namespace: String,
    pub wrote_config: bool,
    pub updated_gitignore: bool,
}

impl ProjectInit {
    pub fn uteke_dir(&self) -> std::path::PathBuf {
        self.root.join(".uteke")
    }
}

/// Initialize a repo-local uteke store, optionally pinning an embedding
/// `backend` in the generated config. Idempotent: an existing config is left
/// unchanged (backend not overwritten). Reused by `uteke init --project` and
/// the coding path of `uteke onboard`.
pub(crate) fn init_project_store(backend: Option<&str>) -> Result<ProjectInit, String> {
    let root = crate::config::find_project_root()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "Cannot determine project root".to_string())?;

    let uteke_dir = root.join(".uteke");
    std::fs::create_dir_all(&uteke_dir)
        .map_err(|e| format!("Failed to create {}: {e}", uteke_dir.display()))?;

    // Derive a namespace from the repo directory name.
    let ns = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string());

    let config_path = uteke_dir.join("uteke.toml");
    let mut wrote_config = false;
    if !config_path.exists() {
        // Include an [embedding] section only when a backend was chosen.
        let embedding_section = match backend {
            Some(b) => format!("\n[embedding]\nbackend = \"{b}\"\n"),
            None => String::new(),
        };
        let body = format!(
            "# Uteke project store (DB-per-repo)\n\
             # DB lives at ./.uteke and is git-ignored; models stay global (~/.uteke/models).\n\
             [store]\n\
             path = \".uteke\"\n\
             namespace = \"{ns}\"\n{embedding_section}"
        );
        std::fs::write(&config_path, body)
            .map_err(|e| format!("Failed to write {}: {e}", config_path.display()))?;
        wrote_config = true;
    }

    // Add DB artifacts to .gitignore (keep the toml tracked).
    let gitignore = root.join(".gitignore");
    let ignore_entries = [".uteke/*.db", ".uteke/*.db-*", ".uteke/*.keys"];
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
    let mut to_add: Vec<&str> = Vec::new();
    for e in ignore_entries {
        if !existing.lines().any(|l| l.trim() == e) {
            to_add.push(e);
        }
    }
    let mut added_gitignore = false;
    if !to_add.is_empty() {
        let mut out = existing.clone();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n# uteke project store\n");
        for e in &to_add {
            out.push_str(e);
            out.push('\n');
        }
        std::fs::write(&gitignore, out)
            .map_err(|e| format!("Failed to write {}: {e}", gitignore.display()))?;
        added_gitignore = true;
    }

    Ok(ProjectInit {
        root,
        config_path,
        namespace: ns,
        wrote_config,
        updated_gitignore: added_gitignore,
    })
}

/// Dispatch init to the appropriate agent type.
pub(crate) fn run_init(agent: &str, memory_provider: bool, json: bool) -> Result<(), String> {
    match agent {
        "pi" => {
            if memory_provider {
                init_pi_memory_provider(json)
            } else {
                init_pi(json)
            }
        }
        "claude" => {
            if memory_provider {
                init_claude_memory_provider(json)
            } else {
                init_claude(json)
            }
        }
        "cursor" => {
            if memory_provider {
                init_cursor_memory_provider(json)
            } else {
                init_cursor(json)
            }
        }
        "opencode" => {
            if memory_provider {
                init_opencode_memory_provider(json)
            } else {
                init_opencode(json)
            }
        }
        "hermes" => {
            if memory_provider {
                init_hermes_memory_provider(json)
            } else {
                init_hermes(json)
            }
        }
        _ => Err(format!(
            "Unknown agent: {agent}. Supported: pi, claude, cursor, opencode, hermes"
        )),
    }
}

/// Copy the bundled SKILL.md to a target `.agents/skills/uteke-memory/` directory.
/// Returns the path to the written file.
fn install_skill_md(cwd: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let skill_dir = cwd.join(".agents").join("skills").join("uteke-memory");
    std::fs::create_dir_all(&skill_dir).map_err(|e| format!("Failed to create skill dir: {e}"))?;
    let dest = skill_dir.join("SKILL.md");
    // Bundled SKILL.md is in .agents/skills/uteke-memory/ relative to repo root.
    // For `cargo install` builds it's embedded; for `cargo run` it lives in the
    // crate's CWD.  We embed the content at compile time to always have it.
    let bundled = include_str!("../../../.agents/skills/uteke-memory/SKILL.md");
    std::fs::write(&dest, bundled).map_err(|e| format!("Failed to write SKILL.md: {e}"))?;
    Ok(dest)
}

/// Initialize uteke integration for Pi agents.
/// Writes the bundled SKILL.md to `.agents/skills/uteke-memory/SKILL.md`.
fn init_pi(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let dest = install_skill_md(&cwd)?;

    if json {
        let obj = serde_json::json!({
            "agent": "pi",
            "skill": dest.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!("✓ Pi skill installed: {}", dest.display());
        println!("  Restart your agent to activate.");
    }
    Ok(())
}

/// Initialize uteke integration for Claude.
fn init_claude(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let md_path = cwd.join("UTEKE.md");

    let md_content = r#"# Uteke Memory Integration

## Commands
- `uteke remember "<text>" --tags <tags>` — Store a memory
- `uteke recall "<query>" --limit <n>` — Semantic search
- `uteke search "<keywords>"` — Keyword search
- `uteke list --tag <tag>` — List memories
- `uteke get <id>` — Get by ID
- `uteke forget <id>` — Delete
- `uteke stats` — Statistics
- `uteke export [file]` — Export to JSONL
- `uteke import [file]` — Import from JSONL

## Usage Guidelines
1. Before starting work: recall relevant context
2. After making decisions: store them with tags
3. Before closing session: store session state
4. Use project-specific stores with `--store .uteke`

## Example
```bash
uteke recall "how does auth work?"
uteke remember "Auth uses JWT with 24h expiry" --tags auth,security
```
"#;

    std::fs::write(&md_path, md_content).map_err(|e| format!("Failed to write UTEKE.md: {e}"))?;

    // Try to add reference to CLAUDE.md
    let claude_md = cwd.join("CLAUDE.md");
    if claude_md.exists() {
        let existing = std::fs::read_to_string(&claude_md)
            .map_err(|e| format!("Failed to read CLAUDE.md: {e}"))?;
        if !existing.contains("UTEKE.md") {
            let updated = format!("{existing}\n\n## Uteke Memory\n\nSee [UTEKE.md](UTEKE.md) for uteke memory commands.\n");
            std::fs::write(&claude_md, updated)
                .map_err(|e| format!("Failed to update CLAUDE.md: {e}"))?;
        }
    }

    if json {
        let obj = serde_json::json!({
            "agent": "claude",
            "file": md_path.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!("✓ Claude integration installed: {}", md_path.display());
        if claude_md.exists() {
            println!("  Reference added to CLAUDE.md");
        } else {
            println!("  Tip: Create CLAUDE.md and add a reference to UTEKE.md");
        }
    }
    Ok(())
}

/// Initialize uteke integration for Cursor.
fn init_cursor(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let rules_dir = cwd.join(".cursor").join("rules");
    std::fs::create_dir_all(&rules_dir).map_err(|e| format!("Failed to create rules dir: {e}"))?;

    let rules_content = r#"# Uteke Memory Integration

## Commands
- `uteke remember "<text>" --tags <tags>` — Store a memory
- `uteke recall "<query>" --limit <n>` — Semantic search
- `uteke search "<keywords>"` — Keyword search
- `uteke list --tag <tag>` — List memories
- `uteke get <id>` — Get by ID
- `uteke forget <id>` — Delete
- `uteke stats` — Statistics

## Guidelines
1. Before starting work: recall relevant context
2. After making decisions: store them with tags
3. Before closing session: store session state
4. Use project-specific stores with `--store .uteke`
"#;

    let rules_path = rules_dir.join("uteke.mdc");
    std::fs::write(&rules_path, rules_content)
        .map_err(|e| format!("Failed to write rules: {e}"))?;

    if json {
        let obj = serde_json::json!({
            "agent": "cursor",
            "file": rules_path.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!("✓ Cursor rules installed: {}", rules_path.display());
    }
    Ok(())
}

/// Initialize uteke memory-provider integration for Pi (#575).
///
/// Installs the pi TypeScript extension that hooks `before_agent_start` to
/// automatically inject relevant memories into every agent turn — mirroring
/// the Hermes memory-provider experience. No manual `uteke recall` needed.
///
/// Extension template lives in `extensions/pi-memory-provider/`.
fn init_pi_memory_provider(json: bool) -> Result<(), String> {
    let ext_dir = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| {
            let mut p = std::path::PathBuf::from(h);
            p.push(".pi");
            p.push("agent");
            p.push("extensions");
            p.push("uteke-memory-provider");
            p
        })
        .or_else(|| {
            std::env::current_dir().ok().map(|d| {
                d.join(".pi")
                    .join("extensions")
                    .join("uteke-memory-provider")
            })
        })
        .ok_or_else(|| "Cannot determine pi extension install directory".to_string())?;

    let installed_to_home = ext_dir.components().any(|c| c.as_os_str() == ".pi");

    std::fs::create_dir_all(&ext_dir)
        .map_err(|e| format!("Failed to create extension dir: {e}"))?;

    // Templates embedded from extensions/pi-memory-provider/ at build time.
    let index_ts = include_str!("../../../extensions/pi-memory-provider/index.ts");
    let package_json = include_str!("../../../extensions/pi-memory-provider/package.json");

    std::fs::write(ext_dir.join("index.ts"), index_ts)
        .map_err(|e| format!("Failed to write index.ts: {e}"))?;
    std::fs::write(ext_dir.join("package.json"), package_json)
        .map_err(|e| format!("Failed to write package.json: {e}"))?;

    if json {
        let obj = serde_json::json!({
            "agent": "pi",
            "plugin": "memory-provider",
            "directory": ext_dir.to_string_lossy(),
            "files": ["index.ts", "package.json"],
            "status": "installed",
            "auto_registered": installed_to_home
        });
        println!("{obj}");
    } else {
        println!(
            "✓ Pi memory-provider extension installed: {}/",
            ext_dir.display()
        );
        if installed_to_home {
            println!("  Location: ~/.pi/agent/extensions/uteke-memory-provider/");
        } else {
            println!("  Location: {}/", ext_dir.display());
        }
        println!("  Restart pi to activate — memories are auto-recalled every turn.");
    }
    Ok(())
}

/// Initialize uteke memory-provider integration for Claude Code (#575).
///
/// Claude Code has no lifecycle hooks, so the approach is:
/// 1. Generate an enhanced UTEKE.md with stronger auto-recall rules
/// 2. Wire the MCP server (uteke-mcp) config snippet for claude_desktop_config.json
///
/// The MCP server provides `uteke_recall`/`uteke_remember` as first-class tools,
/// and the rules instruct Claude to recall at task start.
fn init_claude_memory_provider(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let md_path = cwd.join("UTEKE.md");

    // Enhanced rules that include MCP wiring instructions
    let md_content = r#"# Uteke Memory Integration (Auto Mode)

## MCP Server
Your uteke MCP server provides these tools: `uteke_recall`, `uteke_remember`, `uteke_search`, `uteke_list`, `uteke_forget`, `uteke_stats`, `uteke_context`.

## Auto-Recall Rule
**At the start of every task**, before asking the user, run:
```
uteke_recall(query: "<task summary>", limit: 5)
```
Inject any relevant memories into your working context.

## Store Proactively
After each meaningful decision or discovery:
```
uteke_remember(content: "<fact>", tags: ["relevant", "tags"])
```

## Commands
- `uteke remember "<text>" --tags <tags>` — Store a memory
- `uteke recall "<query>" --limit <n>` — Semantic search
- `uteke search "<keywords>"` — Keyword search
- `uteke stats` — Statistics

## MCP Config (add to claude_desktop_config.json or .claude.json)
```json
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp",
      "args": []
    }
  }
}
```
"#;

    std::fs::write(&md_path, md_content).map_err(|e| format!("Failed to write UTEKE.md: {e}"))?;

    // Enhance CLAUDE.md with auto-recall reference
    let claude_md = cwd.join("CLAUDE.md");
    if claude_md.exists() {
        let existing = std::fs::read_to_string(&claude_md)
            .map_err(|e| format!("Failed to read CLAUDE.md: {e}"))?;
        if !existing.contains("uteke_recall") {
            let updated = format!(
                "{existing}\n\n## Uteke Auto-Memory\n\nBefore every task, call `uteke_recall` with the task summary. See UTEKE.md for full integration.\n"
            );
            std::fs::write(&claude_md, updated)
                .map_err(|e| format!("Failed to update CLAUDE.md: {e}"))?;
        }
    }

    if json {
        let obj = serde_json::json!({
            "agent": "claude",
            "plugin": "memory-provider",
            "file": md_path.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!("✓ Claude memory-provider installed: {}", md_path.display());
        println!("  UTEKE.md includes auto-recall rules + MCP config snippet.");
        if claude_md.exists() {
            println!("  Auto-recall reference added to CLAUDE.md");
        }
        println!();
        println!("  Add MCP server to your Claude config:");
        println!("    claude mcp add uteke -- uteke-mcp");
    }
    Ok(())
}

/// Initialize uteke memory-provider integration for Cursor (#575).
///
/// Cursor has no lifecycle hooks. Strategy:
/// 1. Generate enhanced cursor rules (.cursor/rules/uteke.mdc) with auto-recall rules
/// 2. Include MCP server config snippet
fn init_cursor_memory_provider(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let rules_dir = cwd.join(".cursor").join("rules");
    std::fs::create_dir_all(&rules_dir).map_err(|e| format!("Failed to create rules dir: {e}"))?;

    let rules_content = r#"---
description: Uteke auto-memory — recall before every task, store after decisions
globs: 
alwaysApply: true
---

# Uteke Memory (Auto Mode)

## MCP Tools Available
`uteke_recall`, `uteke_remember`, `uteke_search`, `uteke_list`, `uteke_forget`, `uteke_stats`, `uteke_context`

## Auto-Recall Rule
**Before starting any task**, call `uteke_recall` with the task summary as the query. Use the results to restore context from prior sessions.

## Store Proactively
After meaningful decisions, discoveries, or progress: call `uteke_remember` with relevant tags.

## MCP Config (add to .cursor/mcp.json)
```json
{
  "mcpServers": {
    "uteke": {
      "command": "uteke-mcp",
      "args": []
    }
  }
}
```
"#;

    let rules_path = rules_dir.join("uteke.mdc");
    std::fs::write(&rules_path, rules_content)
        .map_err(|e| format!("Failed to write rules: {e}"))?;

    if json {
        let obj = serde_json::json!({
            "agent": "cursor",
            "plugin": "memory-provider",
            "file": rules_path.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!(
            "✓ Cursor memory-provider installed: {}",
            rules_path.display()
        );
        println!("  Rules include auto-recall + MCP config snippet.");
        println!();
        println!("  Add MCP server to .cursor/mcp.json:");
        println!("    {{\"mcpServers\":{{\"uteke\":{{\"command\":\"uteke-mcp\"}}}}}}");
    }
    Ok(())
}

/// Initialize uteke integration for OpenCode.
/// Writes AGENTS.md (or appends to existing) with uteke instructions.
fn init_opencode(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let agents_path = cwd.join("AGENTS.md");

    let md_content = r#"# Uteke Memory Integration

## Commands
- `uteke remember "<text>" --tags <tags>` — Store a memory
- `uteke recall "<query>" --limit <n>` — Semantic search
- `uteke search "<keywords>"` — Keyword search
- `uteke list --tag <tag>` — List memories
- `uteke get <id>` — Get by ID
- `uteke forget <id>` — Delete
- `uteke stats` — Statistics
- `uteke export [file]` — Export to JSONL
- `uteke import [file]` — Import from JSONL

## Guidelines
1. Before starting work: recall relevant context
2. After making decisions: store them with tags
3. Before closing session: store session state
4. Use project-specific stores with `--store .uteke`
"#;

    if agents_path.exists() {
        let existing = std::fs::read_to_string(&agents_path)
            .map_err(|e| format!("Failed to read AGENTS.md: {e}"))?;
        if !existing.contains("Uteke Memory") {
            let updated = format!("{existing}\n\n{md_content}");
            std::fs::write(&agents_path, updated)
                .map_err(|e| format!("Failed to update AGENTS.md: {e}"))?;
        }
    } else {
        std::fs::write(&agents_path, md_content)
            .map_err(|e| format!("Failed to write AGENTS.md: {e}"))?;
    }

    // Also install the bundled SKILL.md for agents that read .agents/skills/
    let _skill = install_skill_md(&cwd);

    if json {
        let obj = serde_json::json!({
            "agent": "opencode",
            "file": agents_path.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!(
            "✓ OpenCode integration installed: {}",
            agents_path.display()
        );
        println!("  Restart OpenCode to activate.");
    }
    Ok(())
}

/// Initialize uteke memory-provider integration for OpenCode.
///
/// OpenCode has no lifecycle hooks. Strategy:
/// 1. Generate enhanced AGENTS.md with auto-recall rules
/// 2. Include MCP server config snippet
fn init_opencode_memory_provider(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let agents_path = cwd.join("AGENTS.md");

    let md_content = r#"# Uteke Memory Integration (Auto Mode)

## MCP Tools Available
`uteke_recall`, `uteke_remember`, `uteke_search`, `uteke_list`, `uteke_forget`, `uteke_stats`, `uteke_context`

## Auto-Recall Rule
**Before starting any task**, call `uteke_recall` with the task summary as the query. Use the results to restore context from prior sessions.

## Store Proactively
After meaningful decisions, discoveries, or progress: call `uteke_remember` with relevant tags.

## MCP Config (add to opencode.json)
```json
{
  "mcp": {
    "uteke": {
      "command": "uteke-mcp",
      "args": []
    }
  }
}
```
"#;

    if agents_path.exists() {
        let existing = std::fs::read_to_string(&agents_path)
            .map_err(|e| format!("Failed to read AGENTS.md: {e}"))?;
        if !existing.contains("uteke_recall") {
            let updated = format!("{existing}\n\n{md_content}");
            std::fs::write(&agents_path, updated)
                .map_err(|e| format!("Failed to update AGENTS.md: {e}"))?;
        }
    } else {
        std::fs::write(&agents_path, md_content)
            .map_err(|e| format!("Failed to write AGENTS.md: {e}"))?;
    }

    // Also install the bundled SKILL.md
    let _skill = install_skill_md(&cwd);

    if json {
        let obj = serde_json::json!({
            "agent": "opencode",
            "plugin": "memory-provider",
            "file": agents_path.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!(
            "✓ OpenCode memory-provider installed: {}",
            agents_path.display()
        );
        println!("  AGENTS.md includes auto-recall + MCP config snippet.");
        println!();
        println!("  Add MCP server to your opencode.json:");
        println!("    {{\"mcp\":{{\"uteke\":{{\"command\":\"uteke-mcp\"}}}}}}");
    }
    Ok(())
}

/// Initialize uteke integration for Hermes (#384).
/// Generates a uteke-tool plugin in the Hermes plugins directory.
fn init_hermes(json: bool) -> Result<(), String> {
    // #385: Auto-install to ~/.hermes/plugins/uteke-tool/ when possible.
    // Fall back to CWD if the home directory isn't available.
    // Check HOME (Unix) then USERPROFILE (Windows).
    let plugin_dir = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| {
            let mut p = std::path::PathBuf::from(h);
            // Use platform-native separators.
            p.push(".hermes");
            p.push("plugins");
            p.push("uteke-tool");
            p
        })
        .or_else(|| std::env::current_dir().ok().map(|d| d.join("uteke-tool")))
        .ok_or_else(|| "Cannot determine plugin install directory".to_string())?;

    let installed_to_home = plugin_dir.components().any(|c| c.as_os_str() == ".hermes");

    std::fs::create_dir_all(&plugin_dir)
        .map_err(|e| format!("Failed to create plugin dir: {e}"))?;

    // __init__.py — required by Hermes plugin loader (#402).
    // Without this, Hermes logs "No __init__.py" and skips the plugin.
    let init_py = "\"\"\"uteke-tool plugin package marker.\n\nHermes requires __init__.py in every plugin directory to load it\nas a Python package. This file marks the directory as importable.\n\"\"\"\n";
    std::fs::write(plugin_dir.join("__init__.py"), init_py)
        .map_err(|e| format!("Failed to write __init__.py: {e}"))?;

    // plugin.yaml — Hermes plugin manifest.
    let plugin_yaml = "name: uteke-tool\ndescription: Semantic memory recall and storage via uteke — includes room operations\nversion: 0.3.1\nauthor: CodeCoraDev\nactions:\n  - uteke\n";
    std::fs::write(plugin_dir.join("plugin.yaml"), plugin_yaml)
        .map_err(|e| format!("Failed to write plugin.yaml: {e}"))?;

    // tool.py — Hermes plugin entry point (#395: includes room operations).
    // Uses only stdlib (urllib) so no pip install needed.
    let tool_py = "#!/usr/bin/env python3\n\"\"\"uteke-tool: Semantic memory plugin for Hermes.\n\nActions:\n  Memory: remember, recall, search, list, forget, stats\n  Room:   room_create, room_remember, room_recall, room_list, room_summary, room_stats, room_delete\n\"\"\"\nimport json\nimport os\nimport urllib.request\nimport urllib.error\n\nUTEKE_URL = os.environ.get(\"UTEKE_SERVER_URL\", \"http://127.0.0.1:8767\")\n\n\ndef _request(method, path, data=None):\n    \"\"\"Make an HTTP request to the uteke server.\"\"\"\n    url = f\"{UTEKE_URL}{path}\"\n    body = json.dumps(data).encode() if data else None\n    req = urllib.request.Request(url, data=body, method=method)\n    req.add_header(\"Content-Type\", \"application/json\")\n    try:\n        with urllib.request.urlopen(req) as resp:\n            return json.loads(resp.read().decode())\n    except urllib.error.HTTPError as e:\n        return {\"error\": e.read().decode(), \"status\": e.code}\n    except urllib.error.URLError:\n        return {\"error\": \"uteke-serve not running. Start it: uteke-serve --port 8767\"}\n\n\ndef uteke(action=\"recall\", **kwargs):\n    \"\"\"Call uteke for memory and room operations.\n\n    Memory actions:\n        uteke(action=\"remember\", content=\"...\", tags=\"t1,t2\", namespace=\"hermes\")\n        uteke(action=\"recall\", content=\"query\", namespace=\"hermes\", limit=5)\n        uteke(action=\"search\", content=\"query\", namespace=\"hermes\")\n        uteke(action=\"list\", namespace=\"hermes\", limit=20)\n        uteke(action=\"forget\", id=\"memory-id\")\n        uteke(action=\"stats\", namespace=\"hermes\")\n\n    Room actions (#395):\n        uteke(action=\"room_create\", room_id=\"planning\", title=\"Sprint Planning\")\n        uteke(action=\"room_recall\", room_id=\"planning\", content=\"deadline\")\n        uteke(action=\"room_list\")\n        uteke(action=\"room_summary\", room_id=\"planning\")\n        uteke(action=\"room_stats\", room_id=\"planning\")\n        uteke(action=\"room_delete\", room_id=\"planning\")\n    \"\"\"\n    content = kwargs.get(\"content\", \"\")\n    namespace = kwargs.get(\"namespace\", \"hermes\")\n    tags = kwargs.get(\"tags\", \"\")\n    limit = kwargs.get(\"limit\", 5)\n\n    # -- Memory actions ----\n    if action == \"remember\":\n        result = _request(\"POST\", \"/remember\", {\n            \"content\": content,\n            \"tags\": tags.split(\",\") if tags else [],\n            \"namespace\": namespace,\n        })\n        if \"error\" not in result:\n            return f\"\\u2713 Stored: {content[:80]}\"\n        return result\n\n    elif action == \"recall\":\n        result = _request(\"POST\", \"/recall\", {\n            \"query\": content,\n            \"limit\": limit,\n            \"namespace\": namespace,\n        })\n        if isinstance(result, list) and result:\n            lines = []\n            for m in result:\n                score = m.get(\"score\", 0)\n                text = m.get(\"memory\", {}).get(\"content\", \"?\")\n                lines.append(f\"[{score:.2f}] {text}\")\n            return \"\\n\".join(lines)\n        return \"No memories found.\"\n\n    elif action == \"search\":\n        result = _request(\"POST\", \"/search\", {\n            \"query\": content,\n            \"limit\": limit,\n            \"namespace\": namespace,\n        })\n        if isinstance(result, list) and result:\n            lines = []\n            for m in result:\n                score = m.get(\"score\", 0)\n                text = m.get(\"content\", \"?\")\n                lines.append(f\"[{score:.2f}] {text}\")\n            return \"\\n\".join(lines)\n        return \"No memories found.\"\n\n    elif action == \"list\":\n        result = _request(\"POST\", \"/list\", {\n            \"limit\": limit,\n            \"namespace\": namespace,\n        })\n        if isinstance(result, list) and result:\n            lines = []\n            for m in result:\n                mid = m.get(\"id\", \"?\")[:8]\n                text = m.get(\"content\", \"?\")[:60]\n                lines.append(f\"[{mid}] {text}\")\n            return \"\\n\".join(lines)\n        return \"No memories found.\"\n\n    elif action == \"forget\":\n        mid = kwargs.get(\"id\", \"\")\n        result = _request(\"DELETE\", f\"/forget?id={mid}\")\n        return f\"\\u2713 Deleted memory: {mid}\" if \"error\" not in result else result\n\n    elif action == \"stats\":\n        result = _request(\"GET\", f\"/stats?namespace={namespace}\")\n        return json.dumps(result, indent=2)\n\n    # -- Room actions (#395) ----\n    elif action == \"room_create\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        title = kwargs.get(\"title\")\n        result = _request(\"POST\", \"/room/create\", {\n            \"room_id\": room_id,\n            \"title\": title,\n            \"namespace\": namespace,\n        })\n        if \"error\" not in result:\n            return f\"\\u2713 Room '{room_id}' created\"\n        return result\n\n    elif action == \"room_remember\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        author = kwargs.get(\"author\", \"agent\")\n        result = _request(\"POST\", \"/remember\", {\n            \"content\": content,\n            \"tags\": tags.split(\",\") if tags else [],\n            \"namespace\": namespace,\n            \"room\": room_id,\n            \"author\": author,\n        })\n        if \"error\" not in result:\n            return f\"\\u2713 Stored in room \'{room_id}\': {content[:80]}\"\n        return result\n\n    elif action == \"room_recall\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        result = _request(\"POST\", \"/room/recall\", {\n            \"room_id\": room_id,\n            \"query\": content,\n            \"limit\": limit,\n        })\n        if isinstance(result, list) and result:\n            lines = []\n            for m in result:\n                score = m.get(\"score\", 0)\n                text = m.get(\"memory\", {}).get(\"content\", \"?\")\n                lines.append(f\"[{score:.2f}] {text}\")\n            return \"\\n\".join(lines)\n        return \"No memories found in room.\"\n\n    elif action == \"room_list\":\n        result = _request(\"GET\", \"/room/list\")\n        if isinstance(result, list) and result:\n            lines = []\n            for r in result:\n                rid = r.get(\"id\", \"?\")\n                title = r.get(\"title\", \"(untitled)\")\n                ns = r.get(\"namespace\", \"?\")\n                lines.append(f\"  {rid}  {title}  [{ns}]\")\n            return \"\\n\".join(lines)\n        return \"No rooms found.\"\n\n    elif action == \"room_summary\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        result = _request(\"POST\", \"/room/summary\", {\"room_id\": room_id})\n        return json.dumps(result, indent=2) if result else \"Room not found.\"\n\n    elif action == \"room_stats\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        result = _request(\"POST\", \"/room/stats\", {\"room_id\": room_id})\n        return json.dumps(result, indent=2) if result else \"Room not found.\"\n\n    elif action == \"room_delete\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        result = _request(\"DELETE\", \"/room/delete\", {\"room_id\": room_id})\n        if \"error\" not in result:\n            return f\"\\u2713 Room '{room_id}' deleted (memories preserved)\"\n        return result\n\n    return f\"Unknown action: {action}\"\n";
    std::fs::write(plugin_dir.join("tool.py"), tool_py)
        .map_err(|e| format!("Failed to write tool.py: {e}"))?;

    // README.md — quick start guide.
    let readme = "# uteke-tool\n\nSemantic memory plugin for Hermes via [uteke](https://github.com/codecoradev/uteke).\n\n## Setup\n\n1. Install uteke: `curl -fsSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh`\n2. Start daemon: `uteke-serve --port 8767`\n3. This plugin is installed in `~/.hermes/plugins/uteke-tool/`\n4. Start a new Hermes session (plugin loads automatically)\n\n### MCP Server (alternative)\n\nFor MCP-compatible agents, use the uteke MCP server instead:\n\n```bash\nhermes mcp add uteke --command uteke-mcp\n```\n\n## Usage\n\n### Memory Operations\n\n```python\nuteke(action=\"remember\", content=\"User prefers dark mode\", tags=\"preference,ui\")\nuteke(action=\"recall\", content=\"user preferences\")\nuteke(action=\"search\", content=\"dark mode\")\nuteke(action=\"list\", limit=10)\nuteke(action=\"stats\")\nuteke(action=\"forget\", id=\"abc12345\")\n```\n\n### Room Operations (multi-agent collaboration)\n\n```python\n# Create a shared room\nuteke(action=\"room_create\", room_id=\"sprint-planning\", title=\"Sprint Planning\")\n\n# Recall from a room\nuteke(action=\"room_recall\", room_id=\"sprint-planning\", content=\"deadline\")\n\n# List all rooms\nuteke(action=\"room_list\")\n\n# Room analytics\nuteke(action=\"room_stats\", room_id=\"sprint-planning\")\nuteke(action=\"room_summary\", room_id=\"sprint-planning\")\n```\n\n## Configuration\n\n| Environment Variable | Default | Description |\n|---------------------|---------|-------------|\n| `UTEKE_SERVER_URL`  | `http://127.0.0.1:8767` | uteke server URL |\n";
    std::fs::write(plugin_dir.join("README.md"), readme)
        .map_err(|e| format!("Failed to write README.md: {e}"))?;

    if json {
        let obj = serde_json::json!({
            "agent": "hermes",
            "directory": plugin_dir.to_string_lossy(),
            "files": ["__init__.py", "plugin.yaml", "tool.py", "README.md"],
            "status": "installed",
            "auto_registered": installed_to_home
        });
        println!("{obj}");
    } else {
        println!("✓ Hermes plugin installed: {}/", plugin_dir.display());
        if installed_to_home {
            println!("  Location: ~/.hermes/plugins/uteke-tool/");
            println!("  Start a new Hermes session to activate.");
        } else {
            println!("  Location: {}/", plugin_dir.display());
            println!("  Copy to your Hermes plugins directory to activate.");
        }
        println!("\n  Memory actions: remember, recall, search, list, forget, stats");
        println!("  Room actions:   room_create, room_remember, room_recall, room_list, room_summary, room_stats, room_delete");
        println!("\n  Make sure uteke-serve is running: uteke-serve --port 8767");
        println!("  Or use MCP: hermes mcp add uteke --command uteke-mcp");
    }
    Ok(())
}

/// Initialize uteke as Hermes's memory provider (automatic recall + extraction).
///
/// Unlike [`init_hermes`] (the `uteke-tool` plugin: manual `uteke(action=...)`
/// calls over the HTTP daemon), this installs a `MemoryProvider` plugin that
/// makes uteke Hermes's default long-term memory:
/// - recall is prefetched and injected into the prompt every turn,
/// - the transcript is distilled into atomic facts on session end / pre-compress
///   via the opt-in `import --extract` path,
/// - it talks to the `uteke` binary directly (no `uteke-serve` daemon).
///
/// Templates live in `extensions/hermes-memory-provider/` and are embedded at
/// build time so the generated plugin always matches the installed binary.
fn init_hermes_memory_provider(json: bool) -> Result<(), String> {
    // Install to ~/.hermes/plugins/uteke/ (memory providers are keyed by name).
    let plugin_dir = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| {
            let mut p = std::path::PathBuf::from(h);
            p.push(".hermes");
            p.push("plugins");
            p.push("uteke");
            p
        })
        .or_else(|| std::env::current_dir().ok().map(|d| d.join("uteke")))
        .ok_or_else(|| "Cannot determine plugin install directory".to_string())?;

    let installed_to_home = plugin_dir.components().any(|c| c.as_os_str() == ".hermes");

    std::fs::create_dir_all(&plugin_dir)
        .map_err(|e| format!("Failed to create plugin dir: {e}"))?;

    // Templates embedded from extensions/hermes-memory-provider/ at build time.
    let init_py = include_str!("../../../extensions/hermes-memory-provider/__init__.py.tmpl");
    let plugin_yaml = include_str!("../../../extensions/hermes-memory-provider/plugin.yaml.tmpl");

    std::fs::write(plugin_dir.join("__init__.py"), init_py)
        .map_err(|e| format!("Failed to write __init__.py: {e}"))?;
    std::fs::write(plugin_dir.join("plugin.yaml"), plugin_yaml)
        .map_err(|e| format!("Failed to write plugin.yaml: {e}"))?;

    if json {
        let obj = serde_json::json!({
            "agent": "hermes",
            "plugin": "memory-provider",
            "directory": plugin_dir.to_string_lossy(),
            "files": ["__init__.py", "plugin.yaml"],
            "status": "installed",
            "auto_registered": installed_to_home
        });
        println!("{obj}");
    } else {
        println!(
            "✓ Hermes memory-provider plugin installed: {}/",
            plugin_dir.display()
        );
        if installed_to_home {
            println!("  Location: ~/.hermes/plugins/uteke/");
        } else {
            println!("  Location: {}/", plugin_dir.display());
            println!("  Copy to your Hermes plugins directory to activate.");
        }
        println!("\n  Activate it as the default memory provider in ~/.hermes/config.yaml:");
        println!("\n    memory:");
        println!("      provider: uteke");
        println!("\n  Then start a new Hermes session. Recall is automatic; no tool call needed.");
        println!("  Optional: enable LLM fact extraction by configuring extract_* in");
        println!("  ~/.hermes/uteke.json (see docs/integrations/hermes.md).");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The Hermes memory-provider templates are embedded at build time. Guard
    /// against them going missing or losing their entry points.
    const INIT_PY: &str =
        include_str!("../../../extensions/hermes-memory-provider/__init__.py.tmpl");
    const PLUGIN_YAML: &str =
        include_str!("../../../extensions/hermes-memory-provider/plugin.yaml.tmpl");

    #[test]
    fn memory_provider_template_has_register_entrypoint() {
        assert!(
            INIT_PY.contains("def register("),
            "plugin __init__.py must expose a register() entry point"
        );
        assert!(
            INIT_PY.contains("MemoryProvider"),
            "plugin must implement the MemoryProvider interface"
        );
    }

    #[test]
    fn memory_provider_manifest_declares_hooks() {
        assert!(PLUGIN_YAML.contains("name: uteke"));
        assert!(
            PLUGIN_YAML.contains("on_session_end"),
            "manifest must declare the on_session_end hook for extraction"
        );
        assert!(
            PLUGIN_YAML.contains("on_pre_compress"),
            "manifest must declare the on_pre_compress hook"
        );
    }

    // Pi memory-provider template guards (#575).
    const PI_INDEX_TS: &str = include_str!("../../../extensions/pi-memory-provider/index.ts");
    const PI_PACKAGE_JSON: &str =
        include_str!("../../../extensions/pi-memory-provider/package.json");

    #[test]
    fn pi_memory_provider_has_before_agent_start_hook() {
        assert!(
            PI_INDEX_TS.contains("before_agent_start"),
            "pi extension must hook before_agent_start for auto-recall"
        );
        assert!(
            PI_INDEX_TS.contains("recallMemories"),
            "pi extension must have recallMemories function"
        );
        assert!(
            PI_INDEX_TS.contains("uteke recall"),
            "pi extension must invoke uteke recall CLI"
        );
    }

    #[test]
    fn pi_memory_provider_manifest_valid() {
        assert!(PI_PACKAGE_JSON.contains("uteke-memory-provider"));
        assert!(PI_PACKAGE_JSON.contains("Apache-2.0"));
    }
}
