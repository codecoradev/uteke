# Codex hooks

`uteke connect codex` installs lifecycle hooks in Codex: a session starts with
the project's memories in context, and each prompt and reply is queued for
extraction (#1465). It works like the [Claude Code hooks](claude-code.md); only
the file and the trust step differ.

```bash
uteke connect codex --dry-run   # show what would be added, write nothing
uteke connect codex             # install (run again to refresh)
uteke disconnect codex          # remove exactly what connect added
```

## What it installs

Three entries in `~/.codex/hooks.json` (`$CODEX_HOME/hooks.json` when
`CODEX_HOME` is set), in Codex's hook shape:

| Codex event | Runs | Does |
|---|---|---|
| `SessionStart` | `uteke hook-run session-start --agent codex` | Prints the project's memories (a budgeted recall pack inside a `<uteke-stored-data>` block); Codex adds plain stdout as developer context. |
| `UserPromptSubmit` | `uteke hook-run prompt --agent codex` | Queues the prompt as a `user` turn. |
| `Stop` | `uteke hook-run stop --agent codex` | Queues `last_assistant_message` as an `assistant` turn. Prints nothing (Codex rejects plain text on `Stop`). |

Each entry has a `command` for macOS and Linux and a `commandWindows` for
Windows. Both are guarded: without `uteke` on `PATH` the hook does nothing and
exits 0. `hook-run` itself always exits 0 and logs errors to
`~/.codecora/uteke/hook.log`.

## Trust the hooks once

Codex runs hooks it does not manage only after you review them. After
`uteke connect codex`, start Codex, run `/hooks`, and trust the three uteke hooks.
Codex records the trust against each hook's hash, so a refresh that changes an
entry asks again.

Hooks are on by default in Codex; `[features] hooks = false` in
`~/.codex/config.toml` turns them all off.

## Capture, the manifest, upgrades

The same as for Claude Code: the capture hooks need `[capture] enabled = true`
in the global `uteke.toml`; what was added is recorded in
`~/.codecora/uteke/connect/codex.json`; the original file is backed up once to
`hooks.json.uteke-bak`; `uteke doctor` reports stale or removed entries. See
[Claude Code hooks](claude-code.md#what-connect-writes-and-how-disconnect-undoes-it).

Reference: the Codex hooks documentation (developers.openai.com/codex/hooks).
