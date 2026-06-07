---
title: Multi-Agent Isolation
---

# Multi-Agent Isolation

Uteke provides first-class namespace support for running multiple AI agents, each with fully isolated memory.

## How Namespaces Work

Every memory belongs to exactly one namespace. Namespaces are fully isolated — a recall in one namespace never returns results from another.

- **`default`** — Used when no `--namespace` flag is provided. Backward compatible with v0.0.1 databases.
- **`hermes`** — Example: a planning agent that remembers architecture decisions.
- **`pi-agent`** — Example: a coding agent that remembers project-specific context.

## Usage

```bash
# Agent "architect" stores its context
uteke --namespace architect remember "We chose PostgreSQL for ACID compliance" --tags db,decision

# Agent "dev" has its own separate memory
uteke --namespace dev remember "Database connection string: postgres://localhost:5432/app" --tags db,config

# Each only sees its own memories
uteke --namespace architect recall "database"
# → Finds "We chose PostgreSQL for ACID compliance"

uteke --namespace dev recall "database"
# → Finds "Database connection string: postgres://localhost:5432/app"

# Without --namespace, uses "default"
uteke remember "General knowledge" --tags misc
```

## Auto-Migration

Existing databases from v0.0.1 are automatically migrated on first run:

- ✓ `namespace` column added to SQLite
- ✓ All existing memories assigned to `"default"` namespace
- ✓ Zero data loss — your memories are preserved

## Namespace Switching

Switch the default namespace permanently, so you don't need `--namespace` on every call:

```bash
# List all namespaces
uteke namespace list

# See stats for a namespace
uteke namespace stats my-agent

# Switch default (saves to uteke.toml)
uteke namespace switch my-agent

# Now all commands use my-agent by default
uteke remember "Project context" --tags ctx
uteke recall "context"
```

Resolution order: `--namespace` flag → `UTEKE_NAMESPACE` env → `uteke.toml` → `"default"`

## All Commands Are Scoped

The `--namespace` flag works on every command:

| Command | Scoped Behavior |
|---------|-----------------|
| `remember` | Store in namespace |
| `recall` | Search within namespace |
| `search` | Text search within namespace |
| `list` | List memories in namespace |
| `tags list` | Tags for namespace |
| `tags rename` | Rename tag in namespace |
| `tags delete` | Delete tag in namespace |
| `aging status` | Aging breakdown for namespace |
| `aging cleanup` | Cleanup in namespace |
| `stats` | Statistics for namespace |
| `export` | Export from namespace |
| `import` | Import to namespace |

## Best Practices

- **One namespace per agent role** — Use descriptive names like "planner", "coder", "reviewer" instead of "agent-1", "agent-2".
- **Use config files for defaults** — Set `default_namespace` in `uteke.toml` per project so agents don't need `--namespace` on every call.
- **Shell hooks for project isolation** — Install shell hooks (`uteke hook install`) to auto-discover `.uteke/` in parent directories — each project gets isolated memory.
- **Export for backup** — `uteke export --namespace my-agent > backup.jsonl` — backup per-agent memory independently.
