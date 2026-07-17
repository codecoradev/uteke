---
title: Time-Travel Queries
---

# Time-Travel Queries

Query memories as they existed at a specific point in time.

## How It Works

Every memory has temporal validity (`valid_from` / `valid_until`). The `--at` flag filters queries to only memories that existed at the given timestamp.

## Usage

```bash
# List memories that existed on a given date
uteke list --at 2026-06-01T12:00:00Z

# Semantic recall filtered to memories valid at a point in time
uteke recall "deployment process" --at 2026-06-01T12:00:00Z
```

Timestamps use [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339) format.

## Use Cases

- **Post-mortem analysis** — see what the system knew before an incident
- **Audit trail** — trace how knowledge evolved over time
- **Change tracking** — compare memory state between two points in time

## See Also

- [Relationship Graph](/relationship-graph) — link memories with temporal edges
- [CLI Reference](/cli-reference) — full command options
