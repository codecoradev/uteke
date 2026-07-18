---
title: Shell Hooks
---

# Shell Hooks

Auto-load project-scoped memory when you cd into a project directory.

## Installation

```bash
uteke hook install bash   # or zsh, fish
```

## How It Works

When you `cd` into a directory containing `.uteke/uteke.db`, the shell hook automatically activates that project's memory store. No manual `--store` flag needed.

## Supported Shells

| Shell | Status |
|-------|--------|
| bash | ✅ |
| zsh | ✅ |
| fish | ✅ |

## Project Setup

Create a project-scoped memory database:

```bash
mkdir -p .uteke
uteke --store .uteke remember "This project uses PostgreSQL"
```

Now when you cd into the project, uteke automatically uses `.uteke/uteke.db`.

## See Also

- [Configuration](/configuration) — store path and other config options
- [CLI Reference](/cli-reference) — full command options
