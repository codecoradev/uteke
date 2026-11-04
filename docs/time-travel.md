---
title: Time-Travel Queries
---

# Time-Travel Queries

Query memories as they existed at a specific point in time.

## How It Works

Every memory has temporal validity (`valid_from` / `valid_until`). The `--at` flag answers "what did we believe at that instant, and was it true then?": a memory is returned when

- `created_at <= at` and it was not yet deprecated (`at < deprecated_at`), and
- `valid_from <= at < valid_until`.

A memory that was forgotten, superseded or replaced **after** `at` therefore comes back, as it was. The same rule applies to `recall`, `list` and room recall, over HTTP (`POST /recall`, `/list`, `/room/recall`), MCP (`uteke_recall`, `uteke_list`) and the CLI. There is no extra parameter for it.

> **Behaviour change.** Before this release `at` never returned a memory that had been deprecated since, although the rule above was documented. `uteke recall --at T --history` and `uteke list --at T --history` (0.22.x) are now the default: the flags are still accepted so scripts do not break, but they are deprecated and have no effect.

How it is found: deprecation removes a memory's vector from the search index, so recall adds the memories deprecated after `at` from SQL and scores them by cosine similarity against the embedding kept in the row. They stay queryable until `prune` removes the row (`deprecated_ttl_days`; 0 keeps them). `update` edits a memory in place, so `at` shows its current content; supersede is the versioned path.

## Usage

```bash
# List memories that existed on a given date
uteke list --at 2026-06-01T12:00:00Z

# Semantic recall filtered to memories valid at a point in time
uteke recall "deployment process" --at 2026-06-01T12:00:00Z
```

Timestamps use [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339) format.

## Saying when a fact was true

A new memory starts to hold at the write time. To record a different interval,
give it as RFC 3339, on any surface:

```bash
uteke remember "the office is in Bandung" --valid-from 2025-01-01T00:00:00Z --valid-until 2026-01-01T00:00:00Z
uteke update <id> --valid-until 2027-01-01T00:00:00Z
```

- `POST /remember` and `PUT /memory`: `valid_from`, `valid_until`.
- MCP `uteke_remember` and `uteke_update`: the same two arguments.

The values go into the `valid_from` / `valid_until` columns that `--at` reads.
A value that is not RFC 3339, or an interval that does not start before it ends,
is refused (HTTP 400) and nothing is written. A new memory starts at the write
time, so a `valid_until` in the past needs a `valid_from` too. A deprecated
memory is refused (restore it first). `POST /remember` also keeps the old copy
of both values in the metadata for one release. `uteke export` and `import`
carry the interval, so a round trip does not lose it.

Rows written before this change, with the values only in the metadata, are not
rewritten: the metadata is free text and not always a valid interval. Set the
columns with `uteke update --valid-from/--valid-until`.

## Use Cases

- **Post-mortem analysis** — see what the system knew before an incident
- **Audit trail** — trace how knowledge evolved over time
- **Change tracking** — compare memory state between two points in time

## See Also

- [Relationship Graph](/relationship-graph) — link memories with temporal edges
- [CLI Reference](/cli-reference) — full command options
