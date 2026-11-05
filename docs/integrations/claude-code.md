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
| `SessionStart` | `uteke hook-run session-start` | Recalls memories for the project (the last part of the session's `cwd`) as a budgeted context pack and prints them inside a `<uteke-stored-data>` block, marked as reference data; Claude Code adds that output to the context. Prints nothing when nothing matches. |
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

## Which `uteke` runs: `PATH` or a fixed path

The default entries resolve `uteke` through `PATH` each time the hook runs. That is
deliberate: the settings never change on an upgrade, and the same text works on every
machine. The cost is trust in `PATH`. A directory earlier in `PATH` than the real
install (a project-local `bin` added by direnv or a package manager, say) that holds a
program named `uteke` is run instead, and receives what the hook sends on stdin: every
prompt and every assistant reply, and the session start event. uteke cannot tell the two
apart.

If that matters to you, pin the binary:

```bash
uteke connect claude --absolute-path
```

This writes the path of the `uteke` you ran (as the OS reports it) into each command,
quoted for `sh -c`, and the hook runs that file only if it is still executable; if it is
gone the hook does nothing and exits 0, as before. The trade-offs: an upgrade or move that
changes the path leaves the hooks pointing at nothing, so `uteke doctor` lists the agent
as stale and `uteke connect claude --absolute-path` refreshes it; and the settings now
hold a path that differs per machine. Run `connect` without the flag to go back to the
`PATH` form. `disconnect` removes the entries either way, by the manifest
(`~/.codecora/uteke/connect/claude.json`, which records the path when the flag was used).
Without the flag the entries and the manifest are exactly what earlier versions wrote.

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
lists the agent as stale and `uteke connect claude` refreshes them (with
`--absolute-path` if you installed that way). `doctor` also
reports entries that were removed from the settings file.

Extraction uses the offline rules, so the turns stay on your machine. They go to an LLM
endpoint only if you set `mode = "llm"` under `[extraction]`; an API key alone does not
(`connect` prints which extractor receives the turns).
