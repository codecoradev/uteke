---
title: Pi Extension
---

# Pi Extension

uteke provides a [pi coding agent](https://pi.dev) extension that shows memory stats in the footer and reminds the agent to use uteke actively.

## Status Bar

Shows memory stats across all namespaces in the pi footer:

```
🧠 uteke:   🔥 4 hot   🟡 0 warm   ❄️ 63 cold   (67 total)
```

Auto-refreshes on session start and after each agent turn.

## Agent Memory Injection

The extension injects a system prompt that reminds the agent to use uteke:

- → Save important context proactively (decisions, progress, architecture)
- → Recall relevant memories before starting tasks
- → Use namespaces and tags for organization
- → Save session summaries when ending

## Install

1. **Global install (all projects)**

```bash
cp -r extensions/uteke-status ~/.pi/agent/extensions/
```

2. **Project-local install**

```bash
cp -r extensions/uteke-status .pi/extensions/
```

3. **Reload pi**

```
/reload
```

## Commands

| Command | Description |
|---------|-------------|
| `/uteke-stats` | Manually refresh memory stats in status bar |

## Requirements

- ✓ uteke installed and in PATH
- ✓ sqlite3 CLI (pre-installed on macOS and most Linux)
- ✓ pi coding agent (v0.14+)

## How It Works

- **Stats query** — Reads `~/.uteke/uteke.db` via sqlite3 CLI for cross-namespace totals
- **System prompt** — Injects uteke usage rules via `before_agent_start` event
- **Status refresh** — Updates footer on `session_start` and `turn_end` events
