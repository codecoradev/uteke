---
title: Multi-Agent Isolation
---

# Multi-Agent Isolation

Uteke provides first-class namespace support for running multiple AI agents, each with fully isolated memory.

## Namespaces are deprecated

Namespaces are deprecated as an isolation mechanism and will be removed in a future 0.x release. The direction is one brain per database: organise memories inside it with **rooms** (projects, research) linked to documents, **tags** for fine-grained filtering, and **author** / **source** for attribution. Namespaces ended up doing four jobs at once (agent identity, machine, project, topic), and implicit creation plus the `default` fallback means a memory written to one namespace is silently missing from a recall that omits the parameter.

What changes now (phase 1, the deprecation signal): nothing breaks. The `namespace` parameter keeps working on HTTP, MCP and the CLI, and reads and storage are untouched. A **write** to a non-default namespace (omitting it, or passing `default`, is not deprecated) now carries a warning:

- HTTP `POST /remember` and `POST /room/remember`: the response `warning` field carries it (merged with any embedding warning). `POST /extract`, `POST /import`, `POST /room/create` and `PUT /memory` (when it moves a memory with `namespace`) gain an optional `warning` field, present only for a non-default namespace.
- MCP `uteke_remember`, `uteke_update` (namespace move) and `uteke_room_create`: a trailing warning line in the tool text. Every tool with a `namespace` argument says it is deprecated in its description.
- CLI: `uteke remember`, `uteke import` and `uteke room create` print one `Warning:` line on stderr. stdout and `--json` output are unchanged.
- The server logs the warning at WARN level, once per distinct namespace per process.

Phase plan (each phase ships in a later 0.x minor):

1. **Deprecation signal** (this change): warning on writes, docs and tool descriptions steer to rooms, tags and author.
2. **Attribution**: `author` (the agent that wrote the memory) accepted with or without a room.
3. **Migration tool**: `uteke namespace migrate --map <file>` with per-namespace targets (`room:<id>`, `tag:<x>`, `author:<x>`, `source:<x>`, or `drop`); dry-run by default, reversible via export.
4. **Single-namespace mode**: every write goes to `default`, a non-default value is recorded as the tag `ns:<x>`, reads ignore the parameter and cover everything.
5. **Cleanup**: remove the namespace `rename`/`delete` commands once the migration tool has shipped for at least one minor.

Until then, prefer a room per project, `project:<name>` tags, and (once phase 2 lands) `author` for the writing agent. The sections below describe the current behaviour.

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
