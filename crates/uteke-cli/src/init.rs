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
        "hermes" => init_hermes(json),
        _ => Err(format!(
            "Unknown agent: {agent}. Supported: pi, claude, cursor, hermes"
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
    let plugin_yaml = "name: uteke-tool\ndescription: Semantic memory recall and storage via uteke — includes room operations\nversion: 0.2.1\nauthor: CodeCoraDev\nactions:\n  - uteke\n";
    std::fs::write(plugin_dir.join("plugin.yaml"), plugin_yaml)
        .map_err(|e| format!("Failed to write plugin.yaml: {e}"))?;

    // tool.py — Hermes plugin entry point (#395: includes room operations).
    // Uses only stdlib (urllib) so no pip install needed.
    let tool_py = "#!/usr/bin/env python3\n\"\"\"uteke-tool: Semantic memory plugin for Hermes.\n\nActions:\n  Memory: remember, recall, search, list, forget, stats\n  Room:   room_create, room_recall, room_list, room_summary, room_stats, room_delete\n\"\"\"\nimport json\nimport os\nimport urllib.request\nimport urllib.error\n\nUTEKE_URL = os.environ.get(\"UTEKE_SERVER_URL\", \"http://127.0.0.1:8767\")\n\n\ndef _request(method, path, data=None):\n    \"\"\"Make an HTTP request to the uteke server.\"\"\"\n    url = f\"{UTEKE_URL}{path}\"\n    body = json.dumps(data).encode() if data else None\n    req = urllib.request.Request(url, data=body, method=method)\n    req.add_header(\"Content-Type\", \"application/json\")\n    try:\n        with urllib.request.urlopen(req) as resp:\n            return json.loads(resp.read().decode())\n    except urllib.error.HTTPError as e:\n        return {\"error\": e.read().decode(), \"status\": e.code}\n    except urllib.error.URLError:\n        return {\"error\": \"uteke-serve not running. Start it: uteke-serve --port 8767\"}\n\n\ndef uteke(action=\"recall\", **kwargs):\n    \"\"\"Call uteke for memory and room operations.\n\n    Memory actions:\n        uteke(action=\"remember\", content=\"...\", tags=\"t1,t2\", namespace=\"hermes\")\n        uteke(action=\"recall\", content=\"query\", namespace=\"hermes\", limit=5)\n        uteke(action=\"search\", content=\"query\", namespace=\"hermes\")\n        uteke(action=\"list\", namespace=\"hermes\", limit=20)\n        uteke(action=\"forget\", id=\"memory-id\")\n        uteke(action=\"stats\", namespace=\"hermes\")\n\n    Room actions (#395):\n        uteke(action=\"room_create\", room_id=\"planning\", title=\"Sprint Planning\")\n        uteke(action=\"room_recall\", room_id=\"planning\", content=\"deadline\")\n        uteke(action=\"room_list\")\n        uteke(action=\"room_summary\", room_id=\"planning\")\n        uteke(action=\"room_stats\", room_id=\"planning\")\n        uteke(action=\"room_delete\", room_id=\"planning\")\n    \"\"\"\n    content = kwargs.get(\"content\", \"\")\n    namespace = kwargs.get(\"namespace\", \"hermes\")\n    tags = kwargs.get(\"tags\", \"\")\n    limit = kwargs.get(\"limit\", 5)\n\n    # -- Memory actions ----\n    if action == \"remember\":\n        result = _request(\"POST\", \"/remember\", {\n            \"content\": content,\n            \"tags\": tags.split(\",\") if tags else [],\n            \"namespace\": namespace,\n        })\n        if \"error\" not in result:\n            return f\"\\u2713 Stored: {content[:80]}\"\n        return result\n\n    elif action == \"recall\":\n        result = _request(\"POST\", \"/recall\", {\n            \"query\": content,\n            \"limit\": limit,\n            \"namespace\": namespace,\n        })\n        if isinstance(result, list) and result:\n            lines = []\n            for m in result:\n                score = m.get(\"score\", 0)\n                text = m.get(\"memory\", {}).get(\"content\", \"?\")\n                lines.append(f\"[{score:.2f}] {text}\")\n            return \"\\n\".join(lines)\n        return \"No memories found.\"\n\n    elif action == \"search\":\n        result = _request(\"POST\", \"/search\", {\n            \"query\": content,\n            \"limit\": limit,\n            \"namespace\": namespace,\n        })\n        if isinstance(result, list) and result:\n            lines = []\n            for m in result:\n                score = m.get(\"score\", 0)\n                text = m.get(\"content\", \"?\")\n                lines.append(f\"[{score:.2f}] {text}\")\n            return \"\\n\".join(lines)\n        return \"No memories found.\"\n\n    elif action == \"list\":\n        result = _request(\"POST\", \"/list\", {\n            \"limit\": limit,\n            \"namespace\": namespace,\n        })\n        if isinstance(result, list) and result:\n            lines = []\n            for m in result:\n                mid = m.get(\"id\", \"?\")[:8]\n                text = m.get(\"content\", \"?\")[:60]\n                lines.append(f\"[{mid}] {text}\")\n            return \"\\n\".join(lines)\n        return \"No memories found.\"\n\n    elif action == \"forget\":\n        mid = kwargs.get(\"id\", \"\")\n        result = _request(\"DELETE\", f\"/forget?id={mid}\")\n        return f\"\\u2713 Deleted memory: {mid}\" if \"error\" not in result else result\n\n    elif action == \"stats\":\n        result = _request(\"GET\", f\"/stats?namespace={namespace}\")\n        return json.dumps(result, indent=2)\n\n    # -- Room actions (#395) ----\n    elif action == \"room_create\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        title = kwargs.get(\"title\")\n        result = _request(\"POST\", \"/room/create\", {\n            \"room_id\": room_id,\n            \"title\": title,\n            \"namespace\": namespace,\n        })\n        if \"error\" not in result:\n            return f\"\\u2713 Room '{room_id}' created\"\n        return result\n\n    elif action == \"room_recall\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        result = _request(\"POST\", \"/room/recall\", {\n            \"room_id\": room_id,\n            \"query\": content,\n            \"limit\": limit,\n        })\n        if isinstance(result, list) and result:\n            lines = []\n            for m in result:\n                score = m.get(\"score\", 0)\n                text = m.get(\"memory\", {}).get(\"content\", \"?\")\n                lines.append(f\"[{score:.2f}] {text}\")\n            return \"\\n\".join(lines)\n        return \"No memories found in room.\"\n\n    elif action == \"room_list\":\n        result = _request(\"GET\", \"/room/list\")\n        if isinstance(result, list) and result:\n            lines = []\n            for r in result:\n                rid = r.get(\"id\", \"?\")\n                title = r.get(\"title\", \"(untitled)\")\n                ns = r.get(\"namespace\", \"?\")\n                lines.append(f\"  {rid}  {title}  [{ns}]\")\n            return \"\\n\".join(lines)\n        return \"No rooms found.\"\n\n    elif action == \"room_summary\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        result = _request(\"POST\", \"/room/summary\", {\"room_id\": room_id})\n        return json.dumps(result, indent=2) if result else \"Room not found.\"\n\n    elif action == \"room_stats\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        result = _request(\"POST\", \"/room/stats\", {\"room_id\": room_id})\n        return json.dumps(result, indent=2) if result else \"Room not found.\"\n\n    elif action == \"room_delete\":\n        room_id = kwargs.get(\"room_id\", \"\")\n        result = _request(\"DELETE\", \"/room/delete\", {\"room_id\": room_id})\n        if \"error\" not in result:\n            return f\"\\u2713 Room '{room_id}' deleted (memories preserved)\"\n        return result\n\n    return f\"Unknown action: {action}\"\n";
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
        println!("  Room actions:   room_create, room_recall, room_list, room_summary, room_stats, room_delete");
        println!("\n  Make sure uteke-serve is running: uteke-serve --port 8767");
        println!("  Or use MCP: hermes mcp add uteke --command uteke-mcp");
    }
    Ok(())
}
