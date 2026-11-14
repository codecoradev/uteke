# Uteke for Hermes

Automatic long-term memory for [Hermes Agent](https://github.com/NousResearch/hermes-agent).
Before every turn this plugin recalls the memories that matter from
[Uteke](https://github.com/codecoradev/uteke) and adds them to the prompt. Uteke
is one offline Rust binary with hybrid vector and full-text recall: no API key,
no cloud account, nothing to run in the background.

- **Local by default.** Recall runs the `uteke` binary on your machine.
- **Optional server.** Point it at your own `uteke-serve` with one setting.
- **Read-only.** The plugin recalls; it never writes, edits or forgets a memory.
- **No surprises.** It never downloads or updates anything.

## Want it hosted? Start with Uteke Cloud

If you would rather not run anything locally, use [Uteke Cloud](https://uteke.app):
`hermes mcp install uteke-cloud` adds the hosted server and you sign in once. It gives
the agent the Uteke tools (recall, remember, rooms, docs). This plugin is the
*automatic recall* add-on for a local store (or your own `uteke-serve`); it does not
sign in to Uteke Cloud.

## Install (local)

You need the `uteke` binary first. Download it from the
[latest release](https://github.com/codecoradev/uteke/releases/latest) (each release is
signed with cosign) or use the install script, then check `uteke --version`.

Then install the plugin, either way:

```sh
hermes plugins install uteke        # from the Hermes plugin catalog
uteke init --agent hermes --memory-provider   # or render it with the CLI
```

You can also paste `codecoradev/uteke/integrations/hermes` into the Hermes Desktop
**Install plugin** dialog.

Enable it and start a new session:

```sh
hermes plugins enable uteke
```

Store memories with the CLI (`uteke remember "..."`), the MCP server (`uteke-mcp`) or
any other Uteke integration; this plugin only reads them back.

## Settings

Non-secret settings come from `$HERMES_HOME/uteke.json` (preferred) or environment variables:

| Setting | Variable | Default |
|---|---|---|
| Path to the binary | `UTEKE_BIN` | found on `PATH` |
| Uteke home (holds the store) | `UTEKE_HOME` | inherited |
| Memory namespace | `UTEKE_NAMESPACE` | the Hermes profile name |
| Server URL (HTTP transport) | `UTEKE_SERVER_URL` | empty: use the binary |
| Memories per turn | `UTEKE_RECALL_LIMIT` | `5` |
| Drop hits below this score | `UTEKE_RECALL_MIN_SCORE` | `0` (keep all) |
| Seconds allowed per recall | `UTEKE_RECALL_TIMEOUT` | `15` |

The server **token** is a secret: set `UTEKE_TOKEN` in Hermes (the plugin settings
form, or the profile `.env`). A `token` key in `uteke.json` is ignored.

## What it does with your data

- Runs the `uteke` binary as a subprocess with an allow-listed environment (`PATH`,
  `HOME`, locale, temp dir and `UTEKE_*`): your other Hermes keys are not passed on.
  It sets `UTEKE_NO_UPDATE_CHECK=1`, so the binary makes no update request.
- With `UTEKE_SERVER_URL` set, sends each recall query (the first 500 characters of
  your message) and the token to that URL, and nothing else.
- Injects the recalled text into the user message inside a `<recalled-memories>`
  block marked as data, not instructions.
- Recall is bounded by a timeout and a circuit breaker (five failures pause it for
  two minutes), so a missing binary or a down server never blocks a turn.
- No telemetry, no self-updater, no background process started by the plugin.

## Troubleshooting

- **Nothing is recalled.** Run `uteke recall "<something you stored>"` in a terminal. If
  that finds nothing, the plugin has nothing to inject. If the command is not found,
  set `UTEKE_BIN`.
- **Wrong memories.** Memories are scoped by namespace; set `UTEKE_NAMESPACE` to the one
  you store into.
- **Remove it.** `hermes plugins disable uteke`, or `uteke disconnect hermes`.

## For maintainers

`plugin.yaml` and `__init__.py` are rendered from the embedded templates in
`crates/uteke-cli/assets/hermes-uteke-memory/` (what `uteke init` installs). Edit the
templates, run `scripts/render-hermes-integration.sh`, and commit; a test fails on drift.
The Hermes catalog pins this directory to an exact commit, so a change reaches users
only through a reviewed pin bump in `NousResearch/hermes-agent`
(`docs/plans/2026-11-11-hermes-marketplace.md`).

License: Apache-2.0.
