# Namespace deprecation: what is done and what is next (#1427)

The decided plan is in [2026-10-13-unified-surfaces.md](2026-10-13-unified-surfaces.md), Phase 3 (H to K). This note records where it stands and expands the remaining tasks. It adds no new decision.

## 1. Done (see the CHANGELOG)

- **Phase 1, the signal:** writing to a non-default namespace returns a deprecation warning on HTTP, MCP and the CLI; docs and tool descriptions steer to rooms, tags and author.
- **Phase 2 (task H), attribution:** `author` is accepted with or without a room (schema v22, forward-only); export and import carry it.

## 2. Next, in order

### Task I: `uteke namespace migrate --map <file>` (phase 3)

Map file (TOML), one entry per namespace:

```toml
[map."research-x"]
room = "research-x"          # or: tag = "x" | author = "agent-a" | source = "host-1" | drop = true
```

Behaviour:

- **Dry run by default**: prints, per namespace, how many memories move, where, and any conflict (a target room that does not exist is created, a `drop` is listed with its count). Nothing is written.
- **The real run (`--apply`) exports first** to a timestamped file next to the database, then applies the map in one transaction per namespace, so a failure leaves earlier namespaces migrated and later ones untouched, and the export is the way back.
- Targets compose: a namespace can map to `room` + `author`. `drop` requires `--allow-drop`.
- Unmapped namespaces are reported and left alone.
- Prod has 42 namespaces over about 12k memories: the plan's rule stands, **run it on a copy first**.

Tests: dry run changes nothing (row hash before/after); apply moves counts as reported; export + import restores the original; each target kind; a conflicting target is reported.

### Task J: single-namespace mode (phase 4)

As specified in the plan: every write goes to `default`, a non-default value is recorded as the tag `ns:<x>`, reads ignore the parameter and carry a `warning`, `GET /namespaces` returns `["default"]`, `rename`/`delete` return `namespace_deprecated`, import maps namespace to the `ns:<x>` tag and export reports `"default"`. This also removes the namespace items of #1439.

**Pre-condition (owner):** the recall setup that still passes a namespace moves to tags first: `~/.claude/CLAUDE.md` (recall `macmini` with tags), `~/.claude/hooks/uteke-recall.sh` and the `uteke-conventions` document. That needs an explicit owner order; the agent does not edit those files. Sequence: I, run it on a copy, owner moves the recall setup, then J, then a release.

### Task K: remove namespace CRUD (phase 5)

At least one minor after J: delete `rename` and `delete` and their routes (`namespace_deprecated` today). It is a breaking removal on a CORE route, so it needs a contract issue to uteke-cloud first, and cloud already returns `namespace_deprecated` for the same calls (Amendment 2), so the divergence closes.

## 3. Risks

- Prod data shape (42 namespaces, some duplicates of the same project): the map file is hand-written; a `--suggest` mode that proposes a map from namespace names and counts would help, but is optional.
- Anything outside this repository that still passes `namespace` (hooks, other agents' configs) keeps working through J because the parameter is accepted, not rejected.
