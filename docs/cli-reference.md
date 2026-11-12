---
title: CLI Reference
---

# CLI Reference

Complete reference for all uteke commands. Version **0.13.0**.

## Global Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--store <path>` | Override store location | `~/.codecora/uteke` |
| `--namespace <name>` | Namespace (deprecated: prefer rooms, tags and author; writes to a non-default namespace print a warning on stderr, see [Namespaces are deprecated](/multi-agent#namespaces-are-deprecated)) | `default` |
| `--json` | Output as JSON | off |
| `--verbose` | Enable debug logging | off |

Config file path is auto-resolved (`~/.codecora/uteke/uteke.toml` + `.uteke/uteke.toml`); there is no `--config` flag.

## uteke onboard

Interactive onboarding wizard — guides new users from zero to productive. Detects install, picks agent, configures extraction mode, toggles features, writes config, installs agent integration, runs a memory test, and introduces Rooms.

```bash
# Interactive (recommended for first-time users)
uteke onboard

# Non-interactive (defaults: agent=hermes, namespace=default, tool mode, offline extraction)
uteke onboard --yes

# Pre-select agent and namespace
uteke onboard --yes --agent claude --namespace work
```

| Flag | Description |
|------|-------------|
| `--yes` | Skip interactive prompts, use defaults |
| `--agent <name>` | Pre-select agent (hermes, claude, cursor, pi, opencode) |
| `--namespace <name>` | Pre-select namespace |

The wizard steps:

1. **Install detection** — checks if `uteke` is on PATH and if a store exists
2. **Agent selection** — Hermes, Claude, Cursor, Pi, or OpenCode
3. **Integration mode** — manual tool vs memory-provider (auto recall + extraction)
4. **Namespace** — for multi-agent isolation
5. **Feature toggles** — toggle ON/OFF for: Aging, Auto-maintenance, Graph rerank, Salience boost, Recency boost, Server mode
6. **Config write** — generates `~/.codecora/uteke/uteke.toml`
7. **Agent init** — runs `uteke init` with your selections
8. **Feature showcase** — prints all commands grouped by category

## uteke upgrade

Check for updates and upgrade to the latest Uteke release.

```bash
uteke upgrade
uteke upgrade --yes
```

| Flag | Description |
|------|-------------|
| `--yes` | Skip confirmation prompt |

> **Note:** Before v0.7.0 this command was called `uteke update`; that name now edits a memory in place. Verifies the cosign (Sigstore, keyless) signature of `checksums-sha256.txt` against roots built into the binary and then the archive's SHA-256 by default; a missing or invalid signature aborts the upgrade, as does a release older than the installed version or a non-stable (pre-release) one; `uteke upgrade` installs only stable releases (see SECURITY.md). Only `uteke upgrade` checks the signature; `install.sh`, `install.ps1` and Homebrew do not. Set `UTEKE_UPGRADE_SKIP_CHECKSUM=1` to skip both checks (not recommended).

## uteke remember

Store a new memory with optional tags, metadata, and contradiction detection.

```bash
# Basic
uteke remember "Deploy v2.1 to staging Friday" --tags deploy,staging

# With metadata enrichment
uteke remember "Deploy staging to AWS us-east-1" \
  --tags deploy,aws \
  --entity staging-server \
  --category infrastructure \
  --meta "source:meeting-note,confidence:0.9"

# With contradiction detection
uteke remember "Server runs on port 8080" --tags config --detect-contradiction

# With memory type
uteke remember "API rate limit is 1000/min" --type fact

# With source attribution (#348)
uteke remember "Deploy at 3pm Friday" --tags deploy --source "https://slack.com/msg/123" --source-type url

# In a specific namespace
uteke remember "User prefers dark mode" --tags pref --namespace my-agent
```

| Flag | Description |
|------|-------------|
| `--tags <tags>` | Comma-separated tags |
| `--entity <name>` | Associate memory with an entity (e.g. "staging-server") |
| `--category <cat>` | Categorize the memory (e.g. "infrastructure") |
| `--meta <pairs>` | Key:value pairs, comma-separated. Auto-detects type (string/number/bool) |
| `--type <type>` | Memory type: fact, procedure, preference, decision, context, note, insight, reference, event |
| `--source <url-or-path>` | Source attribution (URL, file path, or description) |
| `--source-type <type>` | Source type: user, url, file, import, derived, system (default: user) |
| `--detect-contradiction` | Detect conflicting memories (default threshold: 0.65) |
| `--room <room_id>` | Link memory to a room (collaborative context) |
| `--author <name>` | Who wrote the memory (1 to 128 characters). Recorded on the memory with or without `--room`; with `--room` it is also the room author (default there: `anonymous`) |
| `--json` | Output stored memory as JSON |

## uteke recall

Unified search combining vector similarity with FTS5 full-text search, ranked by Reciprocal Rank Fusion (RRF). Default strategy since v0.16.0: **fusion** — a weighted RRF of the vector and hybrid rankings. Searches **both memories and documents** by default (unified mode). Hot memories (accessed within 7 days) get a score boost.

```bash
uteke recall "What framework does the API use?"
uteke recall "deployment" --limit 10
uteke recall "database config" --namespace hermes --json
uteke recall "server" --entity staging-server --json
uteke recall "config" --category infrastructure --limit 5
# Search documents only
uteke recall "API architecture" --type doc
# Search memories only (backward compatible)
uteke recall "deployment" --type memory
# Explain mode (#1160): show the ranking signals behind each result
uteke recall "database caching" --explain
# Budgeted context pack (#1281): bounded selection for prompt injection
uteke recall "database caching" --pack --budget 4000
uteke recall "database caching" --pack --exclude-ids abc123,def456
```

| Flag | Description |
|------|-------------|
| `--type <type>` | Search scope: `all` (default — memories + documents, unified via RRF), `memory` (memories only), `doc` (documents only) |
| `--limit <n>` | Max results (default: 10, at most 100) |
| `--entity <name>` | Filter results to a specific entity |
| `--category <cat>` | Filter results to a specific category |
| `--content-format <fmt>` | Content display: `auto` (detect), `text`, `json` (pretty-print JSON memories) |
| `--where <key=value>` | Filter by JSON field on structured memories (e.g. `--where role=CTO`) |
| `--explain` | Show the ranking signals behind each result (#1160): final score, strategy, vector similarity + rank, FTS rank, RRF score with per-channel fusion contributions, and jaccard/salience/recency/graph boost deltas. Memory-only — not available with `--type doc` |
| `--pack` | Return a budgeted context pack (#1281) instead of a bare ranked list: `{selected, skipped, budget_used, budget_chars}`. Rank-order preserving greedy fill of the character budget; deterministic and LLM-free |
| `--budget <chars>` | Character budget for `--pack` (default: 4000) |
| `--exclude-ids <ids>` | Memory IDs already injected this turn, comma-separated — excluded from the pack and reported as `skipped[reason=excluded]` |
| `--json` | Output as JSON array |

## uteke search

Keyword text search with tag filtering.

```bash
uteke search "monorepo"
uteke search "deploy" --tags staging,prod --limit 20
uteke search "api" --namespace backend --json
```

| Flag | Description |
|------|-------------|
| `--tags <tags>` | Filter by comma-separated tags |
| `--limit <n>` | Max results (default: 10) |
| `--json` | Output as JSON |

## uteke list

List memories with optional tag, entity, category filter and pagination.

```bash
uteke list --limit 20
uteke list --tag deploy --offset 10 --json
uteke list --entity staging-server --json
uteke list --category infrastructure --limit 10
uteke list --namespace hermes
uteke list --at 2026-06-01T12:00:00Z   # Time-travel: list at point in time
```

| Flag | Description |
|------|-------------|
| `--tag <tag>` | Filter by single tag |
| `--entity <name>` | Filter by entity name |
| `--category <cat>` | Filter by category |
| `--limit <n>` | Max results (default: 10, at most 1000) |
| `--offset <n>` | Skip first N results |
| `--min-trust <tier>` | Lowest provenance tier to return: `human`, `agent`, `external` or `untrusted` (lowercase). Filters the tier (author_type + source_type) before `--limit`. `author_type` is self-declared, so this is a convenience filter, not a security boundary; it is not a trust state (#1651). Each result carries `trust_tier` in `--json` |
| `--at <timestamp>` | Query memories at point in time (RFC3339), including those deprecated after it (#1598) |
| `--history` | Deprecated, no effect: `--at` always includes memories deprecated after it. Needs `--at` |
| `--json` | Output as JSON |

## uteke forget

Delete memories by ID, tag, tier, or all. Supports bulk operations.

```bash
# Delete single memory
uteke forget <uuid> --confirm

# Delete all with a tag
uteke forget --tag deploy --confirm

# Delete all cold-tier memories (>30 days old)
uteke forget --cold --confirm

# Delete everything in namespace
uteke forget --all --confirm

# Preview without deleting
uteke forget --tag stale --dry-run
```

| Flag | Description |
|------|-------------|
| `--tag <tag>` | Delete all memories with this tag |
| `--cold` | Delete all cold-tier memories (>30 days) |
| `--all` | Delete all memories in namespace |
| `--confirm` | Skip confirmation prompt |
| `--dry-run` | Preview what would be deleted |
| `--reason <text>` | Why the memory is forgotten (with an ID). `wrong` or `wrong: <note>` also rejects the value, see below |

### Rejecting a wrong value (`--reason wrong`)

Forgetting a memory does not stop the same text from being written again, by an
agent, an import or a later extraction. When a memory is forgotten because it is
**wrong**, say so:

```bash
uteke forget <id> --reason "wrong: the port is 8767" --confirm
uteke supersede <old> <new> --reason wrong          # the old value is rejected
```

This records a tombstone: a hash of the text after normalisation (Unicode NFKC,
letter case, whitespace and invisible characters do not matter). Every write
refuses that text from then on: `remember`, room remember, `update`, `import`
(the row is skipped and counted as `rejected_tombstone`), `import --full` and
consolidation. The HTTP API answers `409 {"error": "rejected_value", "reason": ...}`,
MCP a tool error starting with `rejected_value:`. The tombstone survives `prune`
and is carried by `export --full`. A paraphrase is not caught. Any other reason
(`--reason outdated`) is only kept on the forgotten memory.

**What survives a forget as wrong (#1669).** The tombstone keeps the hash, the id of
the memory, the date, the reason as you typed it and a preview (the first 120
characters of the text), and all of it stays after the memory is pruned and travels in
`export --full`. The reason and the preview pass the privacy filter first, so a secret
in `wrong: the token is ...` is stored masked (also in `strict` mode, where the forget
is never refused for it; with `[privacy] mode = "off"` they are kept as given). To keep
no text at all, set `tombstone_preview = false` under `[privacy]`: new tombstones then
hold the hash only (an empty preview), and the value is still refused. Tombstones
recorded before the setting keep their preview; `uteke tombstone remove` drops one.

## uteke capture / extract-queue

Queue conversation turns now, turn them into memories later (#1466). Capture makes
no LLM call and no network request (one SQLite insert per turn), so an agent hook
can call it at the end of every session. Extraction runs when you choose, on a
budget, one extractor call per session.

```bash
uteke capture --session sess-42 --role user "We moved billing to PostgreSQL"
echo "Noted." | uteke capture --session sess-42 --role assistant
uteke extract-queue status          # pending / extracting / done / failed
uteke extract-queue run --budget 5  # extract up to 5 sessions now
```

- Capture is **off by default**: set `enabled = true` under `[capture]` in the global
  `uteke.toml` (a project file cannot set it). See [Configuration](configuration.md#raw-capture-1466).
- Roles are `user`, `assistant` and `tool`; a tool result is kept as a summary (its
  first 2000 characters). The privacy filter runs before a turn is queued.
- Capturing the same text from the same role in the same session again is a no-op.
- A turn over `max_turn_bytes` (1 MiB) is refused, and so is any turn while the queue holds
  `max_queue_turns` / `max_queue_bytes`. `status` shows the waiting bytes and warns from 80%
  of a limit.
- `run` extracts with the offline rules (no network) unless `[extraction] mode = "llm"`
  is set: only that explicit mode sends the conversation turns to the LLM endpoint. An
  API key alone does not (it may be there for `import --extract`). `run`, `uteke connect`
  (when capture is on) and the `uteke-serve` schedule print or log one line saying which
  extractor receives the turns (offline, or the endpoint host, never the key). Facts are stored like any other write (duplicate
  check, rejected-value tombstones), tagged `capture`, with the source
  `capture:<session>:<batch>`. An extractor error (rate limit, provider down)
  stops the run and the session is retried next time; after `max_attempts` it is
  marked failed. A session left mid-run by a crash is requeued after the lease.
  A long session is extracted in chunks of `chunk_chars` (24000) characters, stored
  as one batch; a provider refusal of the input size (HTTP 413, or a 4xx naming the context length) fails that
  session only and the run goes on.
- Done turns are deleted after `retention_days` (30).
- `uteke-serve` extracts on its own every `interval_minutes` (60) when capture is
  on, `budget_per_run` sessions per run, and doubles the wait after a run the
  extractor stopped (up to 16 intervals). Over HTTP a turn is queued with
  `POST /capture` and the counts read with `GET /capture/status`; over MCP with
  the `uteke_capture` tool. Queueing is refused while capture is off (HTTP 409);
  the status always answers.

## uteke connect / disconnect

Install the lifecycle hooks of an agent app (#1465): recall at session start,
capture of each prompt and reply. Supported: `claude` (Claude Code), `codex`,
`cursor`. For `pi` and `hermes`, whose hooks are a plugin, `connect` installs the
memory-provider plugin (`uteke init <agent> --memory-provider`) and `disconnect`
removes its files while they are unchanged.

```bash
uteke connect claude --dry-run   # what would be added; writes nothing
uteke connect claude             # install or refresh
uteke connect claude --absolute-path   # pin this binary's path instead of using PATH
uteke disconnect claude          # remove exactly what connect added
```

By default the hook finds `uteke` through `PATH`, so a program named `uteke` in an earlier
`PATH` directory would receive every prompt and reply. `--absolute-path` (the hook agents
only, not `pi`/`hermes`) writes the running binary's path instead; `doctor` reports the
install as stale when that file is gone, and `uteke connect <agent> --absolute-path`
refreshes it. See [Claude Code](integrations/claude-code.md#which-uteke-runs-path-or-a-fixed-path).

Each hook runs `uteke hook-run <event>` (hidden) through a guard that is a no-op
when `uteke` is not on `PATH`; `hook-run` always exits 0 and logs errors to
`~/.codecora/uteke/hook.log`. What was added is recorded in
`~/.codecora/uteke/connect/<agent>.json`; the original file is backed up once to
`<file>.uteke-bak`. `uteke doctor` reports hooks that a newer uteke would install
differently, or that were removed. Capture hooks need `[capture] enabled = true`.
Details: [Claude Code](integrations/claude-code.md), [Codex](integrations/codex.md),
[Cursor](integrations/cursor.md).

## uteke view

Open the read-only browser viewer (#1470): memories, namespaces, rooms and health on
a loopback page guarded by a per-launch token. Prints the URL; Ctrl+C stops it.

```bash
uteke view                 # http://127.0.0.1:8768/?token=...
uteke view --port 9000     # 0 picks a free port
```

`--host` with a non-loopback address is refused unless `UTEKE_AUTH_TOKEN` is set.
Read-only: only `GET`, only registry read routes, no CORS. Details:
[Viewer](viewer.md).

## uteke tombstone

```bash
uteke tombstone list                 # hash prefix, date, reason and the start of the text
uteke tombstone remove <hash|id>     # allow the value again (8+ hash characters or the memory id)
```

`uteke promote <id>` (restoring the forgotten memory) also removes its tombstone.

## uteke consolidate

Find and merge duplicate memories using cosine similarity.

```bash
# Preview duplicates (dry run)
uteke consolidate --threshold 0.60 --dry-run

# Merge duplicates
uteke consolidate --threshold 0.60

# Higher threshold = more conservative (default: 0.90)
uteke consolidate --dry-run
```

Recommended threshold: **0.60–0.70** for small embedding models (embeddinggemma-q4). Keeps newer memory, removes older.

## uteke prune

Remove deprecated and expired temporal memories.

```bash
# Preview what would be pruned
uteke prune --ttl 30 --dry-run

# Prune deprecated/expired memories
uteke prune --ttl 30
```

## uteke lifecycle

Safe memory lifecycle management: deprecate, promote, and inspect lifecycle status.

```bash
# Show lifecycle status (active vs deprecated counts + config)
uteke lifecycle status

# Scoped to a namespace
uteke lifecycle status --namespace my-agent

# Run a lifecycle cycle (deprecate aged + prune expired)
uteke lifecycle cycle

# Scoped cycle
uteke lifecycle cycle --namespace my-agent

# Promote a deprecated memory back to active
uteke lifecycle promote <memory-id>

# Restore is an alias for promote
uteke lifecycle restore <memory-id>
```

| Subcommand | Description |
|------------|-------------|
| `status` | Show active/deprecated counts and lifecycle config |
| `cycle` | Run lifecycle cycle (deprecate aged memories + prune expired) |
| `promote <id>` | Restore a deprecated memory to active status |
| `restore <id>` | Alias for `promote` |

See [Configuration → Memory Lifecycle](/configuration#memory-lifecycle) for lifecycle config options.

## uteke provenance

Show the full provenance report for a memory (#1172) — who wrote it, from
what source, its trust tier, whether the content still matches the hash
recorded at write time, and the complete event chain with actor +
evidence.

```bash
# Human-readable audit report
uteke provenance <memory-id>

# JSON (for tooling)
uteke provenance <memory-id> --json
```

Hash verdicts: `✓ matches` (content unchanged since write), `✗ MISMATCH`
(content modified after write — investigate), `—` (memory predates
schema v18; hash will be recorded on next content update).

## uteke supersede

Resolve a conflict: mark the old memory superseded by a newer one (#1053) —
wires the supersession edge pair, soft-deprecates the old memory, and records
the resolution in the contradiction ledger (#1172).

```bash
uteke supersede <old-memory-id> <new-memory-id> --reason "decision pivot"
```

## uteke contradictions

Inspect the contradiction resolution ledger (#1172) — memories that were
superseded by conflict resolution and are not restored.

```bash
# List the resolution ledger
uteke contradictions list

# Filter by namespace, cap entries
uteke contradictions list --namespace ops --limit 20

# JSON (for tooling)
uteke contradictions list --json

# Restore a superseded memory (full UUID or unambiguous prefix)
uteke contradictions undo <memory-id>
```

## uteke namespace

Manage namespaces — list, inspect, switch defaults, and manage members (#1181).

```bash
# List all namespaces with counts
uteke namespace list

# Show stats for a namespace
uteke namespace stats my-agent

# Switch default namespace (saved to config)
uteke namespace switch my-agent

# Move a memory to another namespace (#1181 — no re-embed)
uteke namespace move <memory-id> other-ns

# Rename a namespace — merges into an existing target (#1181)
uteke namespace rename old-ns new-ns

# Delete a namespace with an explicit strategy for its memories (#1181)
uteke namespace delete temp-ns --strategy merge --target archive --confirm
uteke namespace delete ghost-ns --strategy deprecate --confirm
```

Delete strategies: `refuse` (default — refuses while any memory references the
name), `merge` (move all memories to `--target`, the name vanishes), or
`deprecate` (soft-delete — restorable via `uteke lifecycle`/promote, never
hard-deleted). `--confirm` is required for `delete`.

### uteke namespace migrate (#1427)

Moves whole namespaces into `default`, carrying each namespace's meaning over to
a room, a tag, an author and/or a source (namespaces are deprecated, see
[Namespaces are deprecated](/multi-agent#namespaces-are-deprecated)).

```toml
# map.toml: one table per namespace
[map."research-x"]
room = "research-x"      # created when it does not exist
author = "agent-a"       # targets compose: room, tag, author, source

[map."hermes"]
tag = "agent:hermes"

[map."scratch"]
drop = true              # soft-delete; needs --allow-drop
```

```bash
uteke namespace migrate --map map.toml                  # dry run: report only
uteke namespace migrate --map map.toml --apply          # export first, then migrate
uteke namespace migrate --map map.toml --apply --allow-drop
```

- The dry run writes nothing. `--apply` first writes a full export
  (`namespace-migrate-<UTC time>.ndjson` next to the database, created owner-only,
  mode 0600 on Unix, since it holds every memory) and only then
  migrates, one transaction per namespace. Restore it with
  `uteke import --full` into a fresh store. Run it on a copy of the database first.
- `default` cannot be named in the map. Namespaces the map does not name are
  listed and left alone; a mapped namespace with no live memories is reported
  and skipped, so re-running the same map changes nothing.
- Only live memories move. Soft-deleted ones stay in their namespace (an export
  does not carry them).
- A memory that already has a different `author` or `source` keeps it; the
  report counts it as a conflict. A tag already present is not added twice.

Namespace resolution order: `--namespace flag` → `UTEKE_NAMESPACE` env → `uteke.toml` → `"default"`

## uteke tags

Manage tags across all memories.

```bash
# List all tags with counts
uteke tags list --by-count

# Rename a tag
uteke tags rename old-tag new-tag

# Delete a tag from all memories
uteke tags delete unused-tag
```

## uteke recall — Advanced Flags

Additional flags not shown in the basic example above:

```bash
# Minimum similarity score filter
uteke recall "database config" --min 0.7
uteke recall "database config" --strict    # confident matches only: similarity >= min_score_strict (default 0.5)
uteke recall "database config" --min 0.25  # opt-in similarity threshold (default 0.0 since #1223: fusion scores are rank-based)

# Time-travel: query memories at specific point in time
uteke recall "deployment process" --at 2026-06-01T12:00:00Z

# Relationship graph traversal
uteke recall "auth" --related --depth 2

# Recall strategy (fusion | vector | fts5 | hybrid | graph)
uteke recall "auth" --strategy fusion   # default since 0.16.0 — weighted RRF of vector + hybrid rankings
uteke recall "auth" --strategy hybrid   # vector + FTS5 fused via RRF (k=60)
uteke recall "auth" --strategy vector   # vector similarity only
uteke recall "auth" --strategy fts5     # full-text search only
uteke recall "auth" --strategy graph    # hybrid + graph-signal reranking (#378)
```

### `--strategy graph` (Graph-augmented RAG)

The `graph` strategy runs the hybrid (RRF) pipeline, then fuses graph
signals from the `memory_edges` table into each result's score. A memory
that is well-connected in the graph (referenced by many others, high edge
density) drifts upward; isolated memories are untouched.

The boost is **additive and log-scaled**, so it saturates quickly and never
lets a single hub dominate. Configure the weights under `[recall]` in
`uteke.toml` (see [Configuration](./configuration#recall)):

```bash
# Subtle boost (default): well-connected memories nudge up slightly
uteke recall "architecture" --strategy graph

# Stronger authority boost via env var
UTEKE_GRAPH_AUTHORITY_WEIGHT=0.3 uteke recall "architecture" --strategy graph
```

Cold start (no edges yet): `graph` behaves identically to `hybrid` — the
boost is zero when there are no signals.

### AI-context output

```bash
uteke recall "api design" --context
```

| Flag | Description |
|------|-------------|
| `--min <score>` | Minimum score (default 0.0 since 0.17.x, #1223: fusion scores are rank-based, not cosine). `--min-score` is accepted as an alias (#1593) |
| `--strict` | Confident matches only: keep a hit when its `similarity` (the vector-leg cosine) reaches `min_score_strict` (default 0.5). Hits found by full-text search only have no similarity and are dropped. `score` and `--min` are unchanged, and `--strict` works under every strategy (#1650). See [the floor benchmark](../benchmarks/internal/STRICT_FLOOR.md) |
| `--at <timestamp>` | Query memories at point in time (RFC3339), including those deprecated after it (#1598) |
| `--history` | Deprecated, no effect: `--at` always includes memories deprecated after it. Needs `--at` |
| `--related` | Follow relationship edges |
| `--depth <n>` | Traversal depth for --related |
| `--context` | AI-prompt formatted output |
| `--salience <true\|false>` | Without the flag the salience boost is on with weight 0.1. `--salience true` uses the configured `[recall] salience_weight` (default 0.15); `--salience false` turns it off |
| `--recency <true\|false>` | Without the flag the recency boost is on with weight 0.1. `--recency true` uses the configured `[recall] recency_weight` (default 0.15); `--recency false` turns it off |

The Jaccard reranking signal has no flag: set `[recall] jaccard_weight` above 0 to enable it.

### Reading recall scores (#1405)

A recall score is **`base_score` + boosts**, and the base means something
different per strategy. Scores are for ordering results within one query;
they are not a similarity percentage and are not comparable across strategies,
queries or stores.

| Strategy | `base_score` | Typical range of the base |
|----------|--------------|---------------------------|
| `fusion` (default) | Weighted RRF of the vector and hybrid rankings: `1.7 / (60 + rank_v + 1) + 1.0 / (60 + rank_h + 1)`. A function of rank, not of similarity | at most about `0.044` (rank 1 in both) |
| `hybrid` | RRF of the vector and FTS5 rankings divided by the best possible sum, so a memory at the top of both rankings is about `1.0` | up to `1.0` |
| `vector` | The vector similarity of the query and the memory | up to about `1.0` |
| `fts5` | Normalised keyword rank (rank 1 is `1.0`) | up to `1.0` |

Boosts are **added** on top and are not capped, so a final score can exceed
`1.0` (for example `1.1169` for a hybrid result):

- `salience_boost` = salience score x salience weight. The salience score
  combines importance, access signal and pinned state.
- `recency_boost` = recency score x recency weight. A memory created just now
  has recency score `1.0`; it decays with the memory type's half-life.
- `jaccard_boost` and `graph_boost` apply only when those signals are enabled.

Both weights are `0.1` by default, in the CLI and in `uteke-serve` alike. The
configured `[recall]` weights (default `0.15`) are used only when
`--salience` / `--recency` are passed explicitly, and `uteke-serve` does not read
them yet (#1355).

The salience score includes an access signal, so scores drift up a little as a
memory is recalled: in a three-recall test on a fresh store `salience_boost`
went `0.025`, `0.02676`, `0.02977`. Do not expect the same query to return
exactly the same scores twice.

Because fusion's base is so small, the boosts dominate its absolute value, and
two memories at neighbouring ranks have final scores that differ only in the
third or fourth decimal. Rounded to three decimals they can look identical even
though the order is decided. Use `--explain` for the exact breakdown; this is
the output for the top result of a default (fusion) recall:

```json
{
  "strategy": "fusion",
  "final_score": 0.16899788,
  "base_score": 0.043997884,
  "vector_similarity": 0.76310307,
  "vector_rank": 1,
  "rrf_score": 0.043997884,
  "fusion_vector_contribution": 0.027868852,
  "fusion_hybrid_contribution": 0.016129032,
  "salience_boost": 0.025,
  "recency_boost": 0.1
}
```

Here `0.16899788 = 0.043997884 (rrf_score) + 0.025 (salience) + 0.1 (recency)`.

What this means for thresholds: `--min` compares against the final score, so a
value that makes sense for `vector` (similarity scale) removes everything in
`fusion` (base around 0.04), and a small value keeps almost everything in
`hybrid` or `fts5`. The default is `0.0` for this reason (#1223); only set
`--min` after looking at `--explain` output for your own queries and strategy.

## uteke room

Room-based memory management:

```bash
# Create a room
uteke room create "project-kickoff" --title "Project Kickoff"

# List rooms (cross-namespace by default, #392)
uteke room list
uteke room list --namespace my-agent  # scoped

# Show room stats and participants
uteke room stats "project-kickoff"

# Find rooms for a topic (#1704): id/title/description match fused with the
# rooms of the memories a recall finds. Keep the query to 2-3 keywords.
uteke room search "database decision"
uteke room search budget --limit 5 --json

# Recall within a room (semantic)
uteke room recall "project-kickoff" --query "database decision"

# Change the title or description, or rename the room (#1202)
uteke room update "project-kickoff" --description "Kickoff decisions"
uteke room rename "project-kickoff" "kickoff"

# Add an existing memory to a room without copying it (#1589)
uteke room link <memory-id> "project-kickoff" --author alice

# Move a memory from one room to another (#1202)
uteke room move-memory <memory-id> --from "project-kickoff" --to "kickoff"

# Generate structured document from room
uteke room document "project-kickoff"

# Room summary (LLM-free, tag clustering)
uteke room summary "project-kickoff"

# Delete room (memories are NOT deleted, only room links)
uteke room delete "project-kickoff" --confirm

# Link a document to a room (#859, positional slug)
uteke room add-document "project-kickoff" "architecture-spec"

# Unlink a document from a room (#859)
uteke room remove-document "project-kickoff" "architecture-spec"

# List documents linked to a room (#859)
uteke room list-documents "project-kickoff"

# List rooms that reference a document (#859, positional slug)
uteke room list-rooms "architecture-spec"

# Consolidate a room's memories (#1088) — dry-run by default, no LLM calls
uteke room consolidate "project-kickoff"

# Execute consolidation (LLM calls + writes; budget-capped)
uteke room consolidate "project-kickoff" --apply --max-calls 20
```

### Room consolidation notes (#1088)

- **Dry-run default**: shows memory count, batches, and estimated LLM calls — zero API calls.
- **`--apply`** requires LLM config (same `[extraction]` setup as `import --extract`).
- **`--max-calls`** (default 10) is a hard budget: failed LLM calls count too, batches beyond the cap are skipped.
- Source memories are **soft-deprecated** (never deleted), each carrying a `consolidated into <id>` reason.
- New consolidated records pass a provenance policy gate (hedging language, trust tier) before being written.

## uteke bench

Run performance benchmarks with synthetic data:

```bash
# Default: 100, 1000, 10000 memories
uteke bench

# Custom counts
uteke bench --counts 500,5000

# JSON output
uteke bench --json
```

Measures insert throughput, recall latency (avg + p95), and storage footprint.

## uteke pin / unpin

Pin memories so they never decay:

```bash
uteke pin <id>
uteke unpin <id>
```

## uteke importance

Recalculate importance scores:

```bash
uteke importance
```

## uteke aging

Memory aging management with auto-cleanup.

```bash
# Show hot/warm/cold breakdown
uteke aging status

# Preview memories older than 90 days
uteke aging preview --older-than-days 90

# Delete memories older than 180 days
uteke aging cleanup --older-than-days 180 --yes
```

| Subcommand | Flags | Description |
|------------|-------|-------------|
| `status` | — | Show hot/warm/cold/never-accessed counts |
| `preview` | `--older-than-days N` (default 180), `--max-access-count N` (default 1) | Dry-run preview of cleanup candidates |
| `cleanup` | `--older-than-days N` (default 180), `--max-access-count N` (default 1), `--yes` | Delete aged memories (`--yes` skips confirmation) |

## uteke import

Import memories from a file (or stdin with `-`). Format is auto-detected from
the extension and content; override with `--format`.

```bash
# Import JSONL exported by `uteke export`
uteke import memories.jsonl

# Import markdown/plain text (split into chunks)
uteke import notes.md --tags imported,notes
```

### Moving a whole store (`--full`)

```bash
uteke export --full bundle.ndjson          # on the source: memories, rooms, graph, documents, ...
uteke import --full bundle.ndjson          # on the target
```

Export files (`--output` / the bundle) are created owner-only (mode 0600 on Unix), also over an
existing file, because they hold the memories of the store.

The bundle restores documents with their slug, title, parent (hierarchy), path and content,
plus rooms (with each member's role), edges, the graph and the timeline, keeping the original
ids. Memories keep their author, author type, provenance hash and recall count, and timeline
events keep their actor and evidence. Deprecated memories are left out of the bundle, and a
bundle row marked `deprecated` is restored deleted, never live. A bundle written by
0.22.1 or earlier lacks these fields: import applies the defaults and rebuilds document paths
from the parent chain. It carries no vectors: `import`
regenerates them (memories are marked for `uteke repair --reembed`, document chunks are
re-embedded right away). `--full` makes any other file an error instead of a plain import. The
bundle is a file, so a large document does not pass through a tool response; `POST /import`
stays JSONL-only.

A JSONL export carries each memory's room ids. Import links the memory to every room that
exists in the target store, also for a duplicate of a stored memory, and reports the new
links (`room_links`) and the rooms the target lacks (`rooms_missing`). Create the missing
rooms (`uteke room create <ID>`) and import the file again to finish the links; nothing is
copied.

A JSONL row keeps its identity: import stores it under its `id` and keeps its
`memory_type`, `importance`, `pinned`, `content_type` and `slug`. A missing or invalid value
falls back to the old default (a new id, `fact`, `0.5`, not pinned, `text`, no slug), so files
from older versions import as before. Importing the same file again adds nothing. A copy into
another namespace (`--namespace`) gets new ids, since an id is unique across the store. The result
counts what was left out, apart from `skipped` (invalid or rejected rows):

- `duplicates_skipped`: the row is already stored (same id and content, the same content in
  the namespace, or for a row without an id a near-identical memory). Its rooms are linked to
  the stored memory.
- `id_conflicts_skipped`: the id is stored with other content. The stored memory is not
  changed.
- `soft_deleted_skipped`: the content (or id) was deleted in this store. Importing an old
  export does not bring it back.

A row marked `"deprecated": true` (a Uteke Cloud export carries deleted memories) is stored
deleted, without a vector.

### A Uteke Cloud export

A Cloud export (`GET /export` on Uteke Cloud) imports with its documents and rooms:

```bash
uteke import cloud-export.ndjson                 # or --defer-embed for a large one
```

- The manifest (first line) is skipped.
- `"type": "document"` rows become documents: slug, title, content, tags and the parent (a slug,
  or the parent's id), whatever the order of the rows. A document with the same slug is updated
  in place when it changed and left alone when it did not. A parent that is neither in the
  file nor stored is counted in `parent_unresolved` and the document goes to the root.
- `"type": "room"` rows create the rooms that do not exist (title, description; an existing
  room is not changed) and link their `doc_slugs`. A slug that is not stored is counted in
  `room_documents_missing`.
- Documents and rooms are stored before the memories, so each memory's `rooms` find the rooms
  of the same file.

The result adds `documents_imported`, `documents_updated`, `documents_unchanged`,
`parent_unresolved`, `rooms_imported`, `rooms_existing`, `room_documents_linked` and
`room_documents_missing` (each only when non-zero). With `--defer-embed` the document chunks
are stored without vectors as well. Over HTTP, `POST /import` takes a body of up to 64 MiB
(`UTEKE_MAX_IMPORT_SIZE`, in bytes; the other routes keep 10 MB). A larger file gets a 413
that names the limit: split it into several calls (with `"defer_embedding": true` each returns
in seconds), raise the limit on the server, or run `uteke import` on the server host, which has
no limit.

### Store now, embed later (`--defer-embed`)

A JSON/JSONL import embeds every row while it imports. With the local model that
keeps the CPU busy for the whole file, and an embedding backend that is down stops
the import at that row. `--defer-embed` stores the rows without a vector instead:
the import does not wait on the embedder, the rows are found by keyword search at
once, and the vectors are added later in portions you choose:

```bash
uteke import big-export.jsonl --defer-embed
uteke repair --reembed --limit 500 --pause-ms 20   # repeat until nothing is left
```

`repair --reembed` only picks rows that have no vector, so each run continues where
the last one stopped and reports how many it did not try. `--limit N` stops after N
rows were embedded; a row that fails does not count, so rows the backend keeps
rejecting cannot hold back the rest. Rows embedded this way also get
the cosine links `remember` makes. Without vectors the cosine duplicate check
(similarity of 0.95 or more) cannot run during the import, so only exact copies of
a stored memory, or of an earlier row in the same file, are skipped; near-duplicates
stay. `--defer-embed` works with JSON/JSONL input (not `--full`, `--extract` or
`--batch-dir`).

Over HTTP the same switch is `"defer_embedding": true` in the `POST /import` body; the
response then adds `deferred_embedding`. That is the bulk-import runbook for a server: send
the file with `defer_embedding` (each call returns in seconds because no row is embedded in
the request, so a file larger than the body limit can be split into several calls), then run
`uteke repair --reembed --limit N` on the server until nothing is left.

### LLM fact extraction (`--extract`)

Raw source material (chat transcripts, long notes, exported dumps) is noisy:
greetings, filler, boilerplate. Importing it verbatim pollutes recall. With
`--extract`, uteke first sends the text to an OpenAI-compatible chat-completions
endpoint, asks the model to distill it into atomic facts, and stores one memory
per fact.

This is opt-in. Without `--extract`, import makes no network calls and stays
fully offline.

```bash
# Distill a transcript into atomic facts before storing
uteke import session.txt --extract \
  --extract-model gpt-4o-mini \
  --extract-base-url https://api.openai.com/v1 \
  --extract-api-key sk-...

# Use a local Ollama model
uteke import notes.md --extract \
  --extract-model llama3.1 \
  --extract-base-url http://localhost:11434/v1
```

Settings resolve in this order: CLI flag > `UTEKE_EXTRACTION_*` env var >
`[extraction]` config section > built-in default. The API key falls back to the
embedding / `OPENAI_API_KEY` credential, so an existing OpenAI-compatible setup
needs no duplicate key.

| Flag | Description |
|------|-------------|
| `--extract` | Enable LLM extraction (off by default) |
| `--extract-model <M>` | Override the chat model |
| `--extract-base-url <U>` | Override the endpoint base URL |
| `--extract-api-key <K>` | Override the API key |
| `--extract-max-facts <N>` | Cap facts kept per document (0 = default) |

See [configuration](#extraction) for the `[extraction]` config block and
`UTEKE_EXTRACTION_*` environment variables.

#### Scene-segmented output (#1009)

When the LLM extracts facts, it groups them by topic (scene) and assigns
a priority score (0.0–1.0) to each. This means:

- **Scene tags** — each fact gets a `scene:<topic>` tag automatically, so you can filter by topic during recall.
- **Priority** — important decisions get higher `importance` (0.7–0.9), routine facts get lower (0.3–0.5).
- **Type** — facts are tagged with a semantic type (`decision`, `fact`, `preference`, etc.) when the model provides one.

Offline extraction (no LLM) is unaffected — it continues to produce flat
strings with default importance.

### Batch import (`--batch-dir`)

Import all files from a directory in one command. Each file is processed
according to its type:

| Extension | Strategy | Description |
|-----------|----------|-------------|
| `.md` / `.markdown` | Document (default) | Full content → auto-chunk → embed. No LLM. |
| `.md` + `--extract` | Memory extract | LLM fact extraction → atomic facts → embed. |
| `.txt` / `.jsonl` | Memory extract | LLM fact extraction (requires `--extract`). |
| `.txt` / `.jsonl` + `--as-doc` | Document override | Import as document, skip LLM. |

```bash
# Dry run — preview what would be imported
uteke import --batch-dir ./knowledge --dry-run

# Import all markdown as documents (no LLM, fully offline)
uteke import --batch-dir ./docs --recursive --tags docs

# Import with LLM fact extraction
uteke import --batch-dir ./notes --extract \
  --extract-model gpt-4o-mini \
  --extract-base-url https://api.openai.com/v1 \
  --tags imported

# Force all files as documents (skip extraction)
uteke import --batch-dir ./raw --as-doc --recursive

# Limit file size (default: 1MB)
uteke import --batch-dir ./large-docs --max-size 500000
```

| Flag | Description |
|------|-------------|
| `--batch-dir <PATH>` | Directory to import (enables batch mode) |
| `--recursive` | Recurse into subdirectories |
| `--as-doc` | Force all files as documents (skip LLM extraction) |
| `--as-memory` | Force all files through LLM extraction |
| `--max-size <BYTES>` | Skip files larger than N bytes (default: 1,000,000) |
| `--dry-run` | Preview files and strategies without importing |

## uteke graph

Knowledge graph operations (v0.2.0). Nodes and edges stored in SQLite (`graph_nodes`, `graph_edges` tables, schema v7).

```bash
# List all nodes (optionally filter by entity type)
uteke graph nodes
uteke graph nodes --entity-type person

# List all edges (optionally filter by relation)
uteke graph edges --relation owns

# Find neighbors of a node via BFS
uteke graph neighbors alice --depth 2

# Shortest path between two nodes
uteke graph path alice "project-x" --max-depth 5

# Query edges by relation type
uteke graph query part_of

# Show graph statistics
uteke graph stats
```

| Subcommand | Flags | Description |
|------------|-------|-------------|
| `nodes` | `--entity-type <t>` | List graph nodes |
| `edges` | `--relation <r>` | List graph edges |
| `neighbors <label>` | `--depth N` (default 1) | BFS neighbors of a node |
| `path <source> <target>` | `--max-depth N` (default 5) | BFS shortest path |
| `query <relation>` | — | Query edges by relation type |
| `stats` | — | Show graph statistics |

## uteke edges

List auto-wired edges for a memory (v0.2.1, #346). Edges are auto-extracted from content on every `remember()` call using pure pattern matching — no LLM.

### Supported patterns

| Pattern | Edge type | Resolved via |
|---------|-----------|--------------|
| `[[slug]]` | `references` | `memories.slug` lookup |
| `@tag` | `tagged_as` | most recent memory with that tag |
| `^<uuid>` | `supersedes` | direct memory UUID |
| `><uuid>` | `replies_to` | direct memory UUID |
| `rel:<type>:<uuid>` in `--meta` | `<type>` | direct memory UUID (legacy compat) |

### Usage

```bash
# List direct edges (both directions)
uteke edges <memory-id>

# Multi-hop BFS across the edge table
uteke edges <memory-id> --deep 2

# Only show incoming edges (backlinks)
uteke edges <memory-id> --direction incoming

# JSON output
uteke edges <memory-id> --json
```

With `--deep N`, returns memory ids reachable within N hops (cycles detected, start excluded).

`--direction` accepts `incoming`, `outgoing`, or `both` (default `both`).

## uteke rebuild-backlinks

Rebuild `referenced_by` backlinks from existing forward edges (v0.2.1, #350).

Every forward edge (`references`, `tagged_as`, `supersedes`, `replies_to`)
automatically gets an inverse `referenced_by` edge on `remember()`. This
command repairs stores that pre-date #350, or were written to via the
low-level edge API. Idempotent.

```bash
# Rebuild and print a summary
uteke rebuild-backlinks

# Print only the count of new backlinks (script-friendly)
uteke rebuild-backlinks --quiet

# JSON output
uteke rebuild-backlinks --json
```

## uteke dream

Run the full maintenance pipeline in one command (v0.2.1, #353).

Executes phases in dependency order: lint → backlinks → dedup → contradict → orphans → compact → verify.
Each phase records its status. Errors in individual phases are recorded but do not abort the pipeline.

```bash
# Run all phases
uteke dream

# Run specific phases only
uteke dream --phases lint,orphans

# Dry-run (preview what would change)
uteke dream --dry-run

# Scoped to a namespace
uteke dream --namespace my-agent

# Run only contradiction detection
uteke dream --phases contradict
```

| Flag | Description |
|------|-------------|
| `--phases <list>` | Comma-separated subset: lint, backlinks, dedup, contradict, orphans, compact, verify |
| `--dry-run` | Preview without making changes |
| `--namespace <ns>` | Run scoped to a specific namespace (backlinks and verify are global) |
| `--json` | JSON output |

### Phases

| Phase | Description |
|-------|-------------|
| `lint` | Check for invalid memory types, missing slugs, stale deprecated flags |
| `backlinks` | Rebuild `referenced_by` edges (same as `rebuild-backlinks`) |
| `dedup` | Find and merge near-duplicate memories (cosine ≥ 0.90) |
| `contradict` | Detect contradictory memories (threshold 0.65) and flag for review |
| `orphans` | Find disconnected, low-importance memories |
| `compact` | Apply auto-prune to cold-tier and deprecated memories |
| `verify` | Verify DB and index consistency |

## uteke feedback

Record feedback on a memory's usefulness (v0.8.0, #718). Adjusts importance score: +0.05 for helpful, -0.10 for unhelpful.

```bash
uteke feedback helpful <memory-id>
uteke feedback unhelpful <memory-id>
```

With `--json` flag, outputs structured JSON with id, feedback type, delta, and new importance value.

## uteke orphans

Find orphan memories — disconnected nodes with low importance and few accesses (v0.2.1, #351).

```bash
# Find orphans with default thresholds
uteke orphans

# Custom threshold and result count
uteke orphans --threshold 0.2 --limit 100

# JSON output for scripting
uteke orphans --json
```

| Flag | Description |
|------|-------------|
| `--threshold <n>` | Importance below which a memory is a candidate (default: 0.3) |
| `--limit <n>` | Max results, 0 = all (default: 50) |
| `--namespace <ns>` | Scope to namespace |
| `--json` | JSON output |

## uteke timeline

View chronological event log for a memory (v0.2.1, #347).

Every memory has an audit trail: creation, updates, type changes, supersession,
edge additions, and pin/unpin events.

```bash
# Show timeline for a memory
uteke timeline <memory-id>

# Limit to last N events
uteke timeline <memory-id> --limit 10

# JSON output
uteke timeline <memory-id> --json
```

| Flag | Description |
|------|-------------|
| `--limit <n>` | Max events, 0 = all (default: 20) |
| `--json` | JSON output |

## Other Commands

| Command | Description |
|---------|-------------|
| `uteke get <id>` | Retrieve a single memory by UUID |
| `uteke forget <id>` | Delete a memory by UUID |
| `uteke forget --tag <tag>` | Delete all memories with a tag |
| `uteke forget --cold` | Delete all cold-tier memories |
| `uteke forget --all` | Delete all memories in namespace |
| `uteke consolidate` | Find and merge duplicate memories |
| `uteke prune` | Remove deprecated/expired memories |
| `uteke stats` | Show store statistics with tier breakdown |
| `uteke export` | Export memories to JSONL (no embeddings) · `--full` = structural export: manifest + rooms/graph/edges/documents/chunks/timeline (round-trips the whole store, format `structural-v1`) |
| `uteke import <file>` | Import memories from JSONL/Markdown/text (`--extract` distills with an LLM) — auto-detects structural exports (manifest first line) and restores the full store; restored memories re-embed via `uteke repair --reembed` |
| `uteke doctor` | Health check (DB, index, model, consistency) |
| `uteke verify` | Verify DB and index consistency |
| `uteke verify-checksums --binary <path>` | Verify binary integrity against SHA256 checksums |
| `uteke repair` | Rebuild index from SQLite |
| `uteke repair --rebuild` | Rebuild index from SQLite (explicit) |
| `uteke repair --reembed` | Regenerate missing embeddings for memories without vectors (#906); `--limit N` stops after N rows were embedded and `--pause-ms MS` sleeps between rows (#1608) |
| `uteke namespace list` | List all namespaces with memory counts |
| `uteke namespace stats <name>` | Show stats for a namespace |
| `uteke namespace switch <name>` | Set default namespace in config |
| `uteke namespace move <id> <ns>` | Move a memory to another namespace, no re-embed (#1181) |
| `uteke namespace rename <old> <new>` | Rename a namespace; existing target = merge (#1181) |
| `uteke namespace delete <ns> --confirm` | Delete a namespace (`--strategy refuse\|merge\|deprecate`, #1181) |
| `uteke init --agent <type>` | Initialize integration (opencode, pi, claude, cursor, hermes) |
| `uteke init --agent hermes --memory-provider` | Install uteke as Hermes's default memory provider (auto recall + extraction). See [Hermes integration, Mode B](integrations/hermes.md). **Note:** Hermes Mode B deprecated — see [integrations/hermes.md](integrations/hermes.md) for migration. |
| `uteke init --agent pi --memory-provider` | Install uteke as pi's default memory provider (auto recall + extraction, #575/#577) |
| `uteke init --agent claude --memory-provider` | Install uteke as Claude Code's default memory provider (auto recall + extraction, #575/#577) |
| `uteke init --agent cursor --memory-provider` | Install uteke as Cursor's default memory provider (auto recall + extraction, #575/#577) |
| `uteke init --agent opencode` | Generate AGENTS.md with uteke instructions for OpenCode (#612) |
| `uteke completions <shell>` | Generate shell completions |

## uteke-serve (Server Mode)

Start a persistent HTTP server for fast AI agent access. Eliminates cold start (~980ms → ~21ms per operation).

```bash
# Start server on default port (8767)
uteke-serve

# Custom port
uteke-serve --port 9000

# With logging
RUST_LOG=info uteke-serve --port 8767
```

When `[server] enabled = true` is set in config, the CLI auto-routes commands through the server. Falls back to local store if server is not running.

### HTTP Endpoints

See the [HTTP API Reference](/api-reference) for the complete list of 50+ endpoints, request/response schemas, and examples.

### MCP Server

Two MCP transport modes are available (v0.2.1, #381):

```bash
# stdio transport (for Claude Desktop, Cursor, etc.)
uteke-mcp

# HTTP transport (POST /mcp on uteke-serve)
curl -X POST http://127.0.0.1:8767/mcp \
  -H "Content-Type: application/json" \
  -H "MCP-Protocol-Version: 2025-06-18" \
  -d '{"jsonrpc":"2.0","method":"initialize","params":{},"id":1}'
```

Protocol version: `2025-06-18` (Streamable HTTP spec).

## Document Commands (#406, #411, #438, #440)

Wiki/knowledge base engine — hierarchical documents with auto-chunking, tree operations, and hybrid search.

### `uteke doc create`

Create or update a document from file, content, or stdin.

```bash
# From file
uteke doc create architecture --file guide.md --tags wiki,tech

# From inline content
uteke doc create notes --content "# Notes\n\nQuick notes" --title "My Notes"

# From stdin
cat README.md | uteke doc create readme --file -

# As child of another document
uteke doc create chapter1 --parent book --file ch1.md
```

| Flag | Description |
|------|-------------|
| `--title <title>` | Document title (auto-derived from first `#` heading if omitted) |
| `--file <path>` | Read content from file (`-` for stdin) |
| `--content <text>` | Inline content (alternative to `--file`) |
| `--tags <tags>` | Comma-separated tags |
| `--parent <slug>` | Parent document slug (creates as child, #438) |

### `uteke doc get`

Get a document by slug or ID.

```bash
uteke doc get architecture
uteke doc get architecture --json
```

### `uteke doc list`

List all documents (global — not namespace-scoped, v0.7.0).

```bash
uteke doc list --limit 20
uteke doc list --tree          # Indented tree view
```

| Flag | Description |
|------|-------------|
| `--limit <n>` | Max results (default: 20) |
| `--tree` | Show as indented tree (recursive children) |

### `uteke doc search`

Search documents using hybrid, semantic, or FTS5 search (#440).

```bash
uteke doc search "authentication flow"
uteke doc search "database" --mode semantic
uteke doc search "API reference" --mode fts --limit 5
```

| Flag | Description |
|------|-------------|
| `--mode <mode>` | `hybrid` (default), `semantic`, or `fts` |
| `--limit <n>` | Max results (default: 5) |

### `uteke doc similar`

Find documents that are close to a document, or to a draft text that is not stored yet (#1730). Nothing is written.

```bash
uteke doc similar deploy-runbook
uteke doc similar deploy-runbook --threshold 0.9 --limit 5 --json
uteke doc similar --file draft-skill.md        # a draft: is there already one like it?
cat draft.md | uteke doc similar --file -
uteke doc similar --text "# Rotate keys\n\nSteps to rotate the signing keys"
```

The comparison uses the **document vector**: the mean of the document's chunk vectors, L2-normalised (no extra embedding call; a draft is chunked and embedded like a document body, so its score equals the one the stored document would get). A hit carries the edge type of the memory graph: `possible_duplicate` at cosine 0.92 and above, `similar_to` at 0.80 and above, `related` only when you pass a `--threshold` below 0.80. Both limits are configurable in [`[doc_similarity]`](configuration.md#document-similarity-1730). The relations are derived from the stored vectors on every call, not stored as edges.

A document without a usable vector (no chunks, a chunk not embedded yet after a structural import, vectors of another size) is skipped and counted: `documents_without_vector` in `--json`, a note in text mode. Run `uteke repair --reembed` to embed it. Asking for the neighbours of such a document returns `source_embedded: false` and no hits. `--text` and `--file` need the embedding model; a slug does not.

| Argument | Description |
|----------|-------------|
| `<slug>` | A stored document (slug or id); omit with `--text` / `--file` |
| `--text <text>` | Draft text instead of a stored document |
| `--file <path>` | Read the draft from a file (`-` for stdin) |
| `--threshold <f>` | Minimum cosine, 0 to 1 (default: 0.80, the configured `similar_threshold`) |
| `--limit <n>` | Max results, 1 to 100 (default: 10) |
| `--namespace <ns>` | Only compare with documents of this namespace (documents created through the API have none, so this is rarely useful) |

`--json` prints `{source, threshold, similar_threshold, duplicate_threshold, source_embedded, documents_without_vector, matches, hits: [{document, score, relation}]}`; `matches` counts the hits before `--limit`.

### `uteke doc duplicates`

Report the near-duplicate documents of the library, a namespace or a subtree (#1730).

```bash
uteke doc duplicates
uteke doc duplicates --subtree skills --threshold 0.9
uteke doc duplicates --json
```

Every pair of documents at or above the duplicate threshold (default 0.92) is listed, best first, and the pairs that share a document are merged into groups. With `--subtree <slug>` only that document and its descendants are compared; with `--namespace` only that namespace; both together use the intersection. A scope of more than 2000 documents with vectors is refused (all pairs are scored): narrow it with a subtree.

| Flag | Description |
|------|-------------|
| `--subtree <slug>` | Only this document and its descendants |
| `--namespace <ns>` | Only documents of this namespace |
| `--threshold <f>` | Minimum cosine, 0 to 1 (default: 0.92, the configured `duplicate_threshold`) |
| `--limit <n>` | Max pairs listed, 1 to 500 (default: 50); the groups always cover every pair |

`--json` prints `{threshold, scanned, documents_without_vector, total_pairs, pairs: [{a, b, score}], groups: [[slug, ...]]}`.

### `uteke doc update`

Partially update a document — only provided fields are changed (#589). Content changes trigger chunk rebuild; metadata-only or title-only updates skip it.

```bash
# Update title only (no chunk rebuild)
uteke doc update architecture --title "New Architecture Guide"

# Update content from file (triggers chunk rebuild)
uteke doc update architecture --file updated-guide.md

# Update content from stdin
echo "# New content" | uteke doc update architecture --file -

# Replace tags
uteke doc update architecture --tags wiki,tech,v2

# Replace metadata
uteke doc update architecture --metadata '{"draft": false, "reviewer": "CTO"}'

# Multiple fields at once
uteke doc update architecture --title "Final Guide" --tags wiki,published
```

| Flag | Description |
|------|-------------|
| `--title <title>` | New title (no chunk rebuild) |
| `--content <text>` | New content (triggers chunk rebuild) |
| `--file <path>` | Read new content from file (`-` for stdin; triggers chunk rebuild) |
| `--tags <tags>` | Replace tags (comma-separated) |
| `--metadata <json>` | Replace metadata (JSON string) |

At least one field is required. Omitted fields are left unchanged.

### `uteke doc move`

Move a document to a new parent or root (#438).

```bash
uteke doc move chapter1 --parent new-book
uteke doc move orphan-doc               # Move to root
```

### `uteke doc delete`

Delete a document by ID or slug (cascades to children and chunks).

```bash
uteke doc delete architecture
uteke doc delete <id>
```

### `uteke doc export`

Export all documents as JSON or markdown.

```bash
uteke doc export                          # markdown to stdout
uteke doc export --json                   # JSON to stdout
uteke doc export --front-matter           # markdown, each document with YAML front-matter
uteke doc export --front-matter --output docs.md
```

`--front-matter` (off by default, so the plain output is unchanged) puts a YAML
block before each document body, so a vault or static-site import keeps the
hierarchy:

```yaml
---
slug: "chapter-1"
title: "Chapter 1: \"Getting started\""
tags: ["guide", "draft"]
parent: "book"
---
```

`parent` is the parent document's slug, or `""` for a root document. Values are
written as double-quoted strings, so titles with `:`, `#`, quotes or newlines
stay valid YAML. The body after the closing `---` is byte-for-byte what the
export prints without the flag. With `--json` the flag is ignored, since each
JSON document already carries these fields.

There is no `uteke doc import` that reads this front-matter back:
`uteke import --batch-dir <dir> --as-doc` stores a file's whole content,
front-matter included, under a slug taken from the file path. For a lossless
round trip of documents (slug, title, hierarchy, content) use
`uteke export --full` and `uteke import --full`.

## Skill Library Commands (#1731, LAB)

The personal skill library: index the skills of this machine, find the few that fit a task, spot near-duplicates and record which exact version a human approved. A skill is a directory with a `SKILL.md` (YAML frontmatter with `name` and `description`, then the instructions) and any reference files below it; the directory name is the slug (lower-case letters, digits and single hyphens, at most 64 characters). The shared registry (a pool across users, GitHub ingest, team sharing) stays in Cloud. The tables use the Cloud names (`skills`, `skill_versions`, `skill_usage`, `skill_ledger`), so moving a library there is a copy.

Skill files are stored as documents with `content_type = 'skill'` (slug `skill/<slug>/<path>`, `SKILL.md` the root), so search is the document engine and the embedding pipeline, but they never appear in `doc list`, `doc search`, `recall` or `doc similar` / `doc duplicates`. `search`, `list`, `show` and `similar` are also read-only HTTP routes (`POST /skill/search`, `/skill/list`, `/skill/show`, `/skill/similar`) and MCP tools (`uteke_skill_search`, `uteke_skill_list`, `uteke_skill_show`, `uteke_skill_similar`), open to read-only tokens; see [api-reference.md](api-reference.md) and [mcp.md](mcp.md). `index`, `approve`, `record` and `lint` read the local disk or change the trust anchor and stay CLI-only. Skill files are not documents for `doc get` / `doc update` either: they answer "not found" there.

### `uteke skill index`

```bash
uteke skill index ~/.claude/skills
uteke skill index ./my-skill --owner team-a --json
```

Reads every skill directory under the given one (the directory itself when it holds a `SKILL.md`), to 3 levels deep, **without modifying any file**, symbolic links are never followed. For each skill it records the package hash, lints `SKILL.md`, and embeds the text files (`.md`, `.markdown`, `.txt`) for search; other files are hashed only. It needs the embedding model. It reports `created`, `updated`, `reverted` and `unchanged` skills, directories it could not read (`rejected`, with the reason, e.g. a directory name that is not a slug or a duplicate slug) and skills that are close to another skill (`near_duplicates`, `possible_duplicate` from 0.92 and `similar_to` from 0.80, the `[doc_similarity]` thresholds).

A new hash makes a new `draft` version and the old one stops being current: an approved one becomes `stale` (the approval does not carry over), a draft becomes `archived`. Putting the exact bytes of an earlier version back makes it current again, with its approval. Only the current version is searchable.

Bounds, so a hostile or accidental tree cannot exhaust memory or time: a package of at most 200 files, 1 MiB per file, 8 MiB in all and 8 directory levels (over a bound the whole package is rejected, never truncated: a hash of part of a package is not the hash of the package); at most 32 indexed text files of 128 KiB each and 512 KiB in all per package (larger ones are hashed but not searchable; `SKILL.md` must fit); 500 skills and 5000 directories per run; 2000 skills per library. One package is held in memory at a time.

| Flag | Description |
|------|-------------|
| `<dir>` | Directory with skills, or one skill directory |
| `--owner <name>` | Owner recorded on new skills (at most 128 characters) |
| `--actor <name>` | Who is indexing, recorded in the ledger |

### `uteke skill search`

```bash
uteke skill search "rotate the signing keys"
uteke skill search "deploy" --limit 3 --approved --json
```

Finds skills by meaning and keywords (hybrid by default, `--mode semantic|fts|hybrid`), one hit per skill with the file that matched best. `--limit` is 1 to 50 (default 5), the query at most 2000 characters. `--approved` keeps only skills whose current version is approved. Hybrid search falls back to keywords when the embedding model is unavailable; `--mode semantic` fails instead. Read-only.

### `uteke skill list` / `uteke skill show`

`list [--status draft|approved] [--limit 1..200]` lists the current version of each skill by slug. `show <slug>` prints the current version (hash, status, validity, pinned, locked, name, description, files with their sizes), the usage counters, the version history (up to 50) and the last 20 ledger rows. `approved_hash` in `--json` is the hash a client may load: the current hash when it is approved, else `null`.

### `uteke skill similar`

```bash
uteke skill similar deploy
uteke skill similar --file draft/SKILL.md      # a draft that is not stored
```

Skills close to a skill, or to a draft text: the cosine of the `SKILL.md` document vectors, the same computation as [`uteke doc similar`](#uteke-doc-similar) (the mean of the chunk vectors, L2-normalised), with the same relations and `--threshold`, `--limit` (1 to 100). The skill itself is never a hit. `--text` / `--file` need the embedding model.

### `uteke skill lint`

```bash
uteke skill lint ~/.claude/skills
uteke skill lint                     # what the last index found, for the library
```

Checks `SKILL.md`. Errors (the skill is invalid and cannot be approved): no or unclosed frontmatter, no `name`, a `name` that is not a slug or differs from the directory name, no `description` or one over 1024 characters. Warnings: an empty body, a `compatibility` over 500 characters. With a directory nothing is stored and no store is needed. The exit status is 1 when a skill is invalid or a directory unreadable.

### `uteke skill approve`

```bash
uteke skill approve deploy@<hash> --actor alice
```

Records that a human approved exactly this version: the trust anchor a client checks before loading the skill (compare the hash of the files it holds with `approved_hash`). `<hash>` is the full 64-character package hash or a prefix of at least 12 hexadecimal characters that names one version. Only the **current** and **valid** version can be approved; approving an approved version is a no-op (no ledger row). Every approval, and every index change, is appended to `skill_ledger` with the hash before and after; database triggers refuse to update or delete a ledger row.

### The package hash

The hash covers the whole package, so it detects a changed reference file or script, not only `SKILL.md`: SHA-256 over every regular file of the directory (any depth, `.git` excluded; symbolic links and special files are never followed or hashed), sorted by relative path in byte order with `/` separators, each file contributing the path length in bytes (8 bytes big-endian), the path (UTF-8), the content length (8 bytes big-endian) and the content. The length prefixes make the encoding unambiguous. The result is lower-case hex. The hash does not include the directory name or any timestamp, so the same files anywhere hash the same. `uteke skill approve` is built to be compatible with a client that hashes "path, length and bytes of every file" this way; a client that encodes the fields differently computes a different hash and must adopt this encoding.

### `uteke skill record`

`uteke skill record <slug> <recalled|loaded|succeeded|failed>` adds one to a counter of the skill's current version (`skill_usage`); counters start again for a new version.

## Graph API (#408, #542)

### Visualization

```bash
# All nodes + edges + stats
curl http://127.0.0.1:8767/graph

# Response: { nodes: [...], edges: [...], stats: {...} }
```

### Mutation

```bash
# Create a typed edge
curl -X POST http://127.0.0.1:8767/graph/edge \
  -H "Content-Type: application/json" \
  -d '{"from": "<uuid>", "to": "<uuid>", "edge_type": "references"}'

# Delete an edge by ID
curl -X DELETE "http://127.0.0.1:8767/graph/edge?id=<edge-uuid>"
```

## Server Authentication (#409)

Dual-role API token model:

| Flag / Env | Role | Access |
|------------|------|--------|
| `--auth-token` / `UTEKE_AUTH_TOKEN` | Admin | All endpoints (GET, POST, DELETE) |
| `--read-only-token` / `UTEKE_READ_ONLY_TOKEN` | ReadOnly | All read endpoints (GET + POST search/list/graph). 403 on writes. |

```bash
# Start with dual tokens
uteke-serve --auth-token admin-secret --read-only-token viewer-key

# Read-only client (GET only)
curl -H "Authorization: Bearer viewer-key" http://127.0.0.1:8767/stats

# Write attempt fails
curl -X POST -H "Authorization: Bearer viewer-key" ...  # 403 Forbidden
```

## Limits

The input limits are fixed in this release:

| Limit | Value |
|---------|-------|
| Memory content | 100,000 characters |
| Tags per memory | 20 |
| Tag length | 50 characters |
| Server request body | 10,485,760 bytes (10 MB) |

A `[limits]` section and the `UTEKE_MAX_*` variables are parsed but not applied
yet, so changing them has no effect. The default `--limit` for recall, search and
list is 10 on every surface.

For the history of every command, see the
[CHANGELOG](https://github.com/codecoradev/uteke/blob/main/CHANGELOG.md).
