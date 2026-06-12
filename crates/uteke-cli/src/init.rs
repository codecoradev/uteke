//! Agent initialization commands (pi, claude, cursor).

use crate::cli::Cli;
use crate::cli::Commands;

/// Handle the Init command from the CLI.
pub(crate) fn run_init_command(cli: &Cli) -> Result<(), String> {
    if let Commands::Init { agent } = &cli.command {
        return run_init(agent, cli.json);
    }
    Ok(())
}

/// Dispatch init to the appropriate agent type.
pub(crate) fn run_init(agent: &str, json: bool) -> Result<(), String> {
    match agent {
        "pi" => init_pi(json),
        "claude" => init_claude(json),
        "cursor" => init_cursor(json),
        _ => Err(format!(
            "Unknown agent: {agent}. Supported: pi, claude, cursor"
        )),
    }
}

/// Initialize uteke integration for Pi agents.
fn init_pi(json: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let skill_dir = cwd.join(".agents").join("skills").join("uteke-memory");
    std::fs::create_dir_all(&skill_dir).map_err(|e| format!("Failed to create skill dir: {e}"))?;

    let skill_content = r#"# Uteke Memory Skill

Provides persistent memory for AI agents via the `uteke` CLI.

## Commands

- `uteke remember "<text>" --tags <tags>` — Store a memory
- `uteke recall "<query>" --limit <n>` — Semantic search
- `uteke search "<keywords>"` — Keyword search
- `uteke list --tag <tag>` — List memories
- `uteke get <id>` — Get a memory by ID
- `uteke forget <id>` — Delete a memory
- `uteke stats` — Show statistics
- `uteke export [file]` — Export memories to JSONL
- `uteke import [file]` — Import memories from JSONL

## Usage Patterns

### Store important context
```bash
uteke remember "Database uses WAL mode for concurrent reads" --tags architecture,db
```

### Recall relevant context
```bash
uteke recall "how does the database work?"
```

### Project-specific store
```bash
uteke --store .uteke remember "Uses React Server Components" --tags frontend
```

## When to Use
- Before starting work: `uteke recall "<project context>"`
- After making decisions: `uteke remember "<decision>" --tags <tags>`
- Before closing session: `uteke remember "<session state>" --tags session`
"#;

    std::fs::write(skill_dir.join("SKILL.md"), skill_content)
        .map_err(|e| format!("Failed to write skill: {e}"))?;

    if json {
        let obj = serde_json::json!({
            "agent": "pi",
            "skill": skill_dir.to_string_lossy(),
            "status": "installed"
        });
        println!("{obj}");
    } else {
        println!("✓ Pi skill installed: {}", skill_dir.display());
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
