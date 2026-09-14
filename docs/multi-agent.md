---
title: Multi-Agent Isolation
---

# Multi-Agent Isolation

Uteke provides first-class namespace support for running multiple AI agents, each with fully isolated memory.

## How Namespaces Work

Every memory belongs to exactly one namespace. Namespaces are fully isolated: a recall in one namespace never returns results from another.

- **`default`**: Used when no `--namespace` flag is provided. Backward compatible with v0.0.1 databases.
- **`hermes`**: Example: a planning agent that remembers architecture decisions.
- **`pi-agent`**: Example: a coding agent that remembers project-specific context.

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
- ✓ Zero data loss: your memories are preserved

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

## Managing Namespaces (#1181)

Namespaces are a derived view of where memories live, and v0.17.0 adds sanctioned
ops to manage them directly:

```bash
# Move one memory to another namespace (plain column update, no re-embed)
uteke namespace move <memory-id> other-ns

# Rename a namespace. If the target already exists, this MERGES into it
# (all memories move, the old name vanishes)
uteke namespace rename old-ns new-ns

# Delete a namespace with an explicit strategy for its memories:
#   refuse    (default) refuses with 409 while any memory references the name
#   merge     moves all memories to --target, the name vanishes
#   deprecate soft-deletes them; restorable via promote, never hard-deleted
uteke namespace delete temp-ns --strategy merge --target archive --confirm
uteke namespace delete ghost-ns --strategy deprecate --confirm
```

The same operations are available over HTTP (`PUT /memory` with a `namespace`
field, `POST /namespaces/rename`, `POST /namespaces/delete`) and MCP
(`uteke_namespace_rename`, `uteke_namespace_delete`, plus the `namespace` field
on `uteke_update`). `GET /namespaces?with_counts=true` reports `active` and
`deprecated` breakdowns alongside the total count. Deleting a namespace never
hard-deletes memories: `refuse` is the safe default, and `deprecate` keeps them
recoverable.

## All Commands Are Scoped

The `--namespace` flag works on every command:

| Command | Scoped Behavior |
|---------|-----------------|
| `remember` | Store in namespace |
| `recall` | Search within namespace |
| `search` | Text search within namespace |
| `list` | List memories in namespace |
| `room` | Room operations in namespace |
| `tags list` | Tags for namespace |
| `tags rename` | Rename tag in namespace |
| `tags delete` | Delete tag in namespace |
| `aging status` | Aging breakdown for namespace |
| `aging cleanup` | Cleanup in namespace |
| `stats` | Statistics for namespace |
| `export` | Export from namespace |
| `import` | Import to namespace |

## Documents Are Global

Unlike memories, **documents are global** (v0.7.0, #614/#615). Documents use unique slugs across all namespaces: no namespace isolation. This means:

- Documents created in any namespace are visible in all namespaces via `uteke doc list`
- Document commands (`create`, `get`, `update`, `search`, `list`, `delete`) do not accept `--namespace`
- Slugs must be globally unique: a duplicate slug across namespaces will be auto-migrated on upgrade (schema v12→v13)

This is intentional: documents represent shared knowledge (wiki, architecture guides, runbooks) that should be accessible to all agents regardless of their memory namespace.

## Best Practices

- **One namespace per agent role**: Use descriptive names like "planner", "coder", "reviewer" instead of "agent-1", "agent-2".
- **Use config files for defaults**: Set `default_namespace` in `uteke.toml` per project so agents don't need `--namespace` on every call.
- **Shell hooks for project isolation**: Install shell hooks (`uteke hook install`) to auto-discover `.uteke/` in parent directories. Each project gets isolated memory.
- **Export for backup**: `uteke export --namespace my-agent > backup.jsonl`. Backup per-agent memory independently.
