# Uteke — Hermes plugin package

This directory is the **committed, rendered Hermes agent-plugin package** for
[Uteke](https://github.com/codecoradev/uteke). It exists so tooling that scans
repositories for installable plugins — the Hermes Desktop **"Install plugin"**
dialog and the official plugin catalog — can detect Uteke: their scanner looks
for `plugin.yaml` + `__init__.py` at the package root.

**Do not edit the files in this directory by hand.** They are rendered from the
embedded templates in `crates/uteke-cli/assets/hermes-uteke-memory/` (the same
source `uteke init` uses). A CI guard test fails if this package drifts.

## Install

**Option A — Hermes Desktop dialog.** Paste any of these into *Install plugin*:

- `https://github.com/codecoradev/uteke/tree/develop/integrations/hermes`
- `codecoradev/uteke/integrations/hermes`
- `https://github.com/codecoradev/uteke#integrations/hermes`

**Option B — Uteke CLI** (renders the identical plugin into `~/.hermes/plugins/uteke/`):

```sh
curl -sSL codecora.dev/uteke/install | sh
uteke init --agent hermes --memory-provider
```

**Option C — MCP server** (alternative integration, no plugin files):

```sh
hermes mcp add uteke --command uteke-mcp
```

## Activate

Point Hermes at Uteke as the memory provider in `~/.hermes/config.yaml`:

```yaml
memory:
  provider: uteke
```

The `uteke` binary must be on `PATH` (or set `UTEKE_BIN`). Recall then runs
automatically every turn via the `pre_llm_call` hook — no tool calls needed.

## Maintenance

Rendered from templates; regenerate after editing the templates:

```sh
scripts/render-hermes-integration.sh          # write
scripts/render-hermes-integration.sh --check  # drift check (CI parity)
```

Upstream guard tests live in `crates/uteke-cli/src/init.rs` (`mod tests`).
