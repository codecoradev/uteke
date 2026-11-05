# Cursor hooks

`uteke connect cursor` installs lifecycle hooks in Cursor's agent: a session
starts with the project's memories in context, and each prompt and reply is
queued for extraction (#1465).

```bash
uteke connect cursor --dry-run   # show what would be added, write nothing
uteke connect cursor             # install (run again to refresh)
uteke disconnect cursor          # remove exactly what connect added
```

`uteke init cursor` is separate: it sets up the MCP server and the rules.

## What it installs

Three entries in the user hooks file `~/.cursor/hooks.json`
(`{"version": 1, "hooks": {...}}`; `connect` adds `version` only when the file
has none):

| Cursor event | Runs | Does |
|---|---|---|
| `sessionStart` | `uteke hook-run session-start --agent cursor` | Returns the project's memories as `{"additional_context": "..."}` (a budgeted recall pack inside a `<uteke-stored-data>` block). |
| `beforeSubmitPrompt` | `uteke hook-run prompt --agent cursor` | Queues the prompt as a `user` turn and always answers `{"continue": true}`. |
| `afterAgentResponse` | `uteke hook-run stop --agent cursor` | Queues the reply (`text`) as an `assistant` turn. |

Turns are grouped by Cursor's `conversation_id`. Capture hangs on the prompt
and response events because `stop` and `sessionEnd` carry no message text.

Each command is guarded: without `uteke` on `PATH` the hook does nothing and
exits 0, and the prompt hook still answers `{"continue": true}`, so a prompt is
never held back. Cursor treats a non-zero exit other than 2 as a failed hook
and continues anyway. `hook-run` always exits 0 and logs errors to
`~/.codecora/uteke/hook.log`. Restart Cursor after connecting.

`uteke connect cursor --absolute-path` pins the binary's path instead of resolving `uteke`
through `PATH`; the prompt hook still answers `{"continue": true}` when the file is gone.
See [which `uteke` runs](claude-code.md#which-uteke-runs-path-or-a-fixed-path) for the
trade-off.

## Known Cursor bug: session-start context dropped

Some Cursor versions ignore `additional_context` returned by `sessionStart`
(reported on the Cursor forum for 3.2.7). The hook still runs and capture still
works, but the recalled memories do not reach the agent. Until Cursor fixes it,
use the MCP tools from `uteke init cursor` for recall.

## Capture, the manifest, upgrades

The same as for Claude Code: the capture hooks need `[capture] enabled = true`
in the global `uteke.toml`; what was added is recorded in
`~/.codecora/uteke/connect/cursor.json`; the original file is backed up once to
`hooks.json.uteke-bak`; `uteke doctor` reports stale or removed entries. See
[Claude Code hooks](claude-code.md#what-connect-writes-and-how-disconnect-undoes-it).

Reference: the Cursor hooks documentation (cursor.com/docs/hooks).
