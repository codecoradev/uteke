# uteke-memory — Hermes Auto-Recall Plugin

> ⚠️ **Mode B (MemoryProvider) is DEPRECATED for Hermes (removed 2026-06-29).**
>
> This plugin has been **rewritten** as a standard Hermes Python plugin that
> registers a `pre_llm_call` hook. It no longer uses the `MemoryProvider`
> interface.
>
> **For Hermes, use this plugin (Mode C) + `uteke-tool` (Mode A).**
> - **Mode A:** `uteke init --agent hermes` → manual `uteke(action=...)` calls
>   via uteke-serve HTTP daemon.
> - **Mode C (this plugin):** Python plugin → automatic recall injected into
>   every turn via the `pre_llm_call` hook. No shell hook, no daemon.
>
> **This plugin template remains for:** `pi`, `Claude Code`, and `Cursor` agents
> (which still support the `--memory-provider` pattern — see `__init__.py.tmpl`
> for the old MemoryProvider code).

## Overview

The `uteke-memory` plugin is a lightweight Hermes plugin that provides
**automatic semantic memory recall** on every turn.

### How It Works

1. On plugin load, it reads config from `~/.hermes/uteke.json` or env vars.
2. It registers a `pre_llm_call` hook via `ctx.register_hook()`.
3. Every turn, Hermes calls the hook with the user message.
4. The hook runs `uteke recall` (subprocess or HTTP), filters by min_score,
   and returns `{"context": "<recalled-memories>..."}`.
5. Hermes injects the context into the user message before the LLM call.
6. The system prompt cache prefix is preserved (injection is into user message).

### Why a Plugin Instead of Shell Hook?

| Aspect | Shell Hook | Python Plugin |
|--------|-----------|--------------|
| Process | Separate subprocess | In-process (same Python runtime) |
| Contextvars | ❌ Not available | ✅ Full access |
| Performance | Process spawn overhead | Direct function call |
| Agent name | Hacky (parse `cwd`) | Reliable (`HERMES_HOME`) |
| Registration | `hooks:` in config.yaml | `ctx.register_hook()` |

## Files

| File | Purpose |
|------|---------|
| `__init__.py.tmpl` | Plugin entry point — `register(ctx)` + `_pre_llm_call()` |
| `plugin.yaml.tmpl` | Hermes plugin manifest |

The `.tmpl` suffix is because `uteke init --agent hermes` strips it when
installing to `~/.hermes/plugins/uteke-memory/`.

## Install

```bash
# Copy to Hermes plugins dir
cp -r extensions/hermes-memory-provider ~/.hermes/plugins/uteke-memory/
cd ~/.hermes/plugins/uteke-memory/
for f in *.tmpl; do mv "$f" "${f%.tmpl}"; done

# Enable in config.yaml
hermes config set plugins.enabled.0 uteke-memory

# Or edit manually:
# plugins:
#   enabled:
#     - uteke-memory
```

## Config

| Variable | Default | Description |
|----------|---------|-------------|
| `UTEKE_BIN` | (search PATH) | Path to uteke binary |
| `UTEKE_HOME` | (inherit) | HOME dir for uteke store |
| `UTEKE_NAMESPACE` | (agent profile) | Memory namespace |
| `UTEKE_SERVER_URL` | (empty) | HTTP URL for uteke-serve |
| `UTEKE_TOKEN` | (empty) | Bearer token for uteke-serve |
| `UTEKE_RECALL_LIMIT` | `5` | Memories per turn |
| `UTEKE_RECALL_MIN_SCORE` | `0.40` | Min relevance score |
| `UTEKE_RECALL_TIMEOUT` | `15` | Max seconds per recall |

## Transport Modes

| Mode | Config | How |
|------|--------|-----|
| **Subprocess** (default) | `UTEKE_BIN` or PATH lookup | Shells out to `uteke` binary |
| **HTTP** (optional) | `UTEKE_SERVER_URL` env var | Calls `uteke-serve` HTTP API |

Use HTTP mode when:
- The `uteke` binary is not available on PATH or hangs
- `uteke-serve` is already running as a container
- You want container-based access (e.g., `http://uteke:8767`)

## See Also

- [docs/integrations/hermes.md](../../docs/integrations/hermes.md) — full integration guide
- [docs/integrations/hermes.md#mode-c](../../docs/integrations/hermes.md#mode-c--uteke-memory-plugin-pre_llm_call-hook-automatic-recall) — Mode C detailed setup