# uteke-mcp: Uteke memory as an Agent Plugin

A portable [Agent Plugins v1](https://agent-plugins.org) package: it gives any host
that supports the format (Hermes Agent among them) persistent, offline memory.

- `skills/uteke/` teaches the agent when and how to recall and remember.
- `mcp.json` starts the `uteke-mcp` server over stdio: recall, remember, rooms, docs.

## Easiest: Uteke Cloud

No install and nothing to run: add the hosted server and sign in.

```sh
hermes mcp install uteke-cloud      # from the Hermes MCP catalog
```

(or add `https://uteke.app/mcp` as a remote MCP server; sign-in is OAuth, free during
the open beta). Choose this package, below, when memory must stay on your machine.

## Local: needs

The `uteke-mcp` binary on `PATH`. It ships in each
[Uteke release](https://github.com/codecoradev/uteke/releases/latest) next to `uteke`
(`uteke upgrade` keeps both current). The plugin does not download or bundle it.

## Install

```sh
hermes plugins install uteke-mcp                          # from the catalog
hermes plugins install codecoradev/uteke/integrations/agent-plugin
```

## Data and permissions

- The server reads and writes the local store (`~/.codecora/uteke/`, or `UTEKE_HOME`).
  Nothing leaves the machine; no API key and no credential is needed or declared.
- It exposes write tools (`remember`, `forget`, `update`, rooms, docs). A host that asks
  for approval before a tool call will ask before these run.
- Recalled text is wrapped in `<uteke-stored-data>` so the model treats it as data.

The automatic-recall hook for Hermes is a separate plugin, `uteke`
(`integrations/hermes`). Use either or both.

The skill is a copy of the one in `codecoradev/skills`; refresh it when that one moves.
License: Apache-2.0.
