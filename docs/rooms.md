---
title: Rooms
---

# Rooms

Group related memories by context — meetings, projects, discussions.

## Create a Room

```bash
uteke room create "project-kickoff" --title "Project Kickoff"
```

## Add Memories to a Room

```bash
uteke room add "project-kickoff" <memory-id> --author alice
```

## Semantic Recall Within a Room

```bash
uteke room recall "project-kickoff" --query "database decision"
```

## Generate a Structured Document

Combine room memories into a cohesive document:

> **Note:** The CLI command remains `uteke room document`, but the underlying HTTP API route has been renamed from `POST /room/document` to `POST /room/summary-document`.

```bash
uteke room document "project-kickoff"
```

## Get a Room Summary

```bash
uteke room summary "project-kickoff"
```

## Use Cases

- **Meeting notes** — create a room per meeting, add memory IDs from discussions
- **Project context** — group all project-related memories for easy recall
- **Research** — compile findings into a structured document via `room document`

## See Also

- [Multi-Agent Isolation](/multi-agent) — each agent can have its own rooms
- [CLI Reference — Room Commands](/cli-reference#room-commands) — full command reference
