# Claude Code hooks

`uteke connect claude` installs lifecycle hooks in Claude Code, so a session
starts with the project's memories in context and the conversation is queued
for extraction without the agent calling a tool (#1465).

```bash
uteke connect claude --dry-run   # show what would be added, write nothing
uteke connect claude             # install (run again to refresh)
uteke disconnect claude          # remove exactly what connect added
```

`uteke init claude` is separate: it sets up the MCP server and the skill. Use
both if you want the tools and the automatic hooks.

## What it installs

Three entries in `~/.claude/settings.json` under `hooks`:

| Claude Code event | Runs | Does |
|---|---|---|
| `SessionStart` | `uteke hook-run session-start` | Recalls memories for the project (the last part of the session's `cwd`) as a budgeted context pack and prints them; Claude Code adds that output to the context. Prints nothing when nothing matches. |
| `UserPromptSubmit` | `uteke hook-run prompt` | Queues the prompt as a `user` turn (`uteke capture`). |
| `Stop` | `uteke hook-run stop` | Queues `last_assistant_message` as an `assistant` turn. |

Each entry runs a guarded command:

```sh
sh -c 'command -v uteke >/dev/null 2>&1 && uteke hook-run session-start --agent claude || true'
```

- `uteke` is found through `PATH` when the hook runs. No path is written into the
  settings, so an upgrade or a moved binary needs no change, and the command is
  the same on every machine.
- Without `uteke` on `PATH` the hook does nothing and exits 0.
- `hook-run` itself always exits 0. Errors go to `~/.codecora/uteke/hook.log`
  (rotated to `hook.log.1` past 1 MB). It makes no LLM call and no network
  request except to a local `uteke-serve`, which it uses when one is running.

## Capture must be on

The prompt and stop hooks queue turns only when capture is enabled in the global
`uteke.toml`:

```toml
[capture]
enabled = true
```

While it is off they do nothing (and `connect` says so). Queued turns become
memories when `uteke extract-queue run` runs, or on `uteke-serve`'s schedule. See
[`uteke capture`](../cli-reference.md#uteke-capture-extract-queue).

## What connect writes, and how disconnect undoes it

- The existing file is read as JSON and every key you have stays in place, in its
  order. Hooks you wrote yourself are kept; uteke's entries are appended to the
  event arrays. A file that is not valid JSON is refused and left untouched.
- The first real run copies the file to `settings.json.uteke-bak`.
- The entries added are recorded in `~/.codecora/uteke/connect/claude.json`, with
  the uteke version. Running `connect` again replaces only those entries, so a
  second run with the same version changes nothing.
- `disconnect` removes the recorded entries that still match exactly. An entry you
  edited is left alone and listed. Event arrays and the `hooks` key that `connect`
  created are removed once empty, and a settings file that `connect` created is
  deleted when nothing else is in it.
- `--dry-run` on either command prints the entries it would add or remove.

## Upgrades

The hooks call `uteke hook-run`, so upgrading uteke upgrades them. If a release
changes the entries themselves (a new event, a different timeout), `uteke doctor`
lists the agent as stale and `uteke connect claude` refreshes them. `doctor` also
reports entries that were removed from the settings file.
