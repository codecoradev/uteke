# Listing Uteke on the Hermes Marketplace

Status as of 2026-11-11. Read from the live page
(`hermes-agent.nousresearch.com/marketplace`), from the catalog source in
`NousResearch/hermes-agent` (`plugin-catalog/README.md`, the submission guide and
the memory-provider guide at `main`, cloned 2026-11-11) and from the validator
source (`hermes_cli/agent_plugins.py`). Re-read the admission rules before
opening the PR: they change, and a new rule can delist an existing entry.

This complements `2026-11-10-directory-listings.md`, which covers third-party
lists (it mentions Hermes Atlas, not this marketplace).

## 1. What the marketplace is

- It is the **official Nous catalog**, generated from one YAML file per plugin in
  `plugin-catalog/` of the `hermes-agent` repo. Live count: 1,731 plugins, equal
  to the 1,731 entry files in the repo.
- Trust tiers on the page: Official 21, Certified 12, Community 1,698. The repo
  calls "Certified" `verified`; only Nous maintainers set `official` or
  `verified`.
- Shelves (categories): Memory 105, Tools 808, Desktop 308, Models 122,
  Automation 118, General 90, Platform 71, Web 66, Voice 43. Tabs: Plugins,
  Skills, Connectors. Default sort is by GitHub stars.
- Users install by name: `hermes plugins install <name>` (also a
  `hermes://plugin/install?catalog=<name>` deep link and the Desktop dialog).
  Every entry also gets `/marketplace/<name>` and `/docs/plugins/<name>`,
  rendering the README **at the pinned commit**.
- **Uteke is not listed**: no `plugin-catalog/*uteke*` file, and the live count
  matches the repo.
- Entry is a pull request, human-reviewed. "There is no self-serve registry, no
  automated ingestion." Memory-provider plugins can no longer be bundled in
  Hermes core ("closed to new providers"), so this catalog is the route.

### Who is on the Memory shelf

| Plugin | Stars | Notes |
|---|---|---|
| mem0 | 68.8k | server-side extraction, cloud or self-hosted |
| hindsight | 60.3k | knowledge graph, cloud or local |
| openviking | 42.0k | managed or local |
| agentmemory | 30.3k | needs a separate server |
| honcho | 8.1k | |
| reme | 3.8k | local-first, HTTP/embedded |
| **mnemosyne** | 3.6k | closest rival: local SQLite, vector + FTS5, zero cloud |
| deja-memory | 1.5k | local, no model, no server; same "runs a separately installed binary" pattern we need |

Uteke has 286 stars, so it will sit low on the default sort. Discoverability
has to come from the card text, the category filter and search, not rank.

## 2. How an entry works (rules that decide the design)

One file, `plugin-catalog/<name>.yaml`: `name`, `repo` (https), a **40-hex `sha`
pin**, optional `subdir`, `description`, `maintainer`, `tier` (always
`community` from a contributor), `category: memory`, `requires_hermes`,
`capabilities`, optional `image` (2:1, on a GitHub host), `screenshots`,
`known_issues`. Installs clone the repo and check out exactly that commit; every
update is a new reviewed PR.

Admission rules, checked against what we ship:

| Rule | Meaning | Where Uteke stands |
|---|---|---|
| 5 | submitted by the repo owner or a major contributor | OK: `codecoradev` owns the repo; the PR must come from an owner account |
| 17 | no bundled binaries, no bytecode | OK: the `uteke` binary is installed separately (precedent: deja-memory); the plugin must never download one |
| 3 | no self-updating code | OK if the plugin never runs `uteke upgrade` |
| 6 | declared `capabilities` match what registers | needs a check: `requires_env` is `[]` today although the code reads `UTEKE_*` |
| **11** | **secrets only through `agent.secret_scope.get_secret`; never from env files or other files** | **FAILS today**: reads `UTEKE_TOKEN` with `os.environ` and from `$HERMES_HOME/uteke.json` |
| 13 | disclose network calls, subprocesses, stored data | missing: no disclosure text in the manifest or README |
| 14 | `requires_hermes` is a SemVer floor, declared in `plugin.yaml` too | missing |
| 12 | no hanging under cron/gateway | recall already has a 15 s timeout and a circuit breaker; confirm no prompt waits |
| 9 | no core overrides, public `register_*` surfaces only | OK as written (`ctx.register_hook`) |

Gaps found in `integrations/hermes/__init__.py` (the package already committed
for catalog scanners):

1. Rule 11, above.
2. `self._env = dict(os.environ)` hands the **whole Hermes environment** (other
   plugins' keys included) to the `uteke` child process. Pass only `PATH`,
   `HOME`, `UTEKE_*` and what the binary needs.
3. Manifest name is `uteke-memory`; the catalog key and manifest `name` must
   agree, and the affiliated project should hold the bare key `uteke`.
4. `docs/integrations/hermes.md` and the plugin docstring say the memory-provider
   mode was "removed 2026-06-29". The current Hermes docs describe memory
   providers as the main catalog path. Correct both before a reviewer reads them.
5. The package README says "do not edit by hand"; the catalog renders this README
   on the plugin page. It needs a user-facing README, banner and screenshots.
6. It is recall-only (a `pre_llm_call` hook). No tools, no capture.

## 3. Options

| | A. Native hook plugin (what we have) | B. Native memory provider | C. Portable Agent Plugin |
|---|---|---|---|
| Shape | `plugin.yaml` + `__init__.py`, `pre_llm_call` | same files, `kind: exclusive`, `MemoryProvider` ABC | `plugin.json` + `skills/uteke/SKILL.md` + `mcp.json` (stdio `uteke-mcp`) |
| User gets | automatic recall | `hermes memory setup`, tools, recall, per-turn capture, `memory.provider: uteke` | agent tools + the skill; no auto-recall |
| Effort | small: fix the gaps above | 3 to 5 days | about half a day, no Python |
| Shelf | Memory (a hook plugin is allowed) | Memory, native feel | Tools / Skills / Connectors |
| Reuse elsewhere | none | none | the same package fits Cursor Marketplace and cursor.directory (directory-listings plan, section 5) |

Portable-package facts read from the validator: `plugin.json` needs `$schema`
(`https://agent-plugins.org/schemas/1.0.0/plugin.schema.json`) and a
`[a-z0-9.-]` `name` up to 64 characters; each skill directory name must equal
the `name` in its `SKILL.md` frontmatter (the skills repo's `skills/uteke` does;
the in-repo `uteke-memory` skill has no `name` and would be skipped); `mcp.json`
has exactly `$schema` and `mcpServers`; a stdio `command` is one bare token
(`uteke-mcp`) with a separate `args` list; credentials never go in `mcp.json`.

**Recommendation:** ship A first (smallest change, reuses the committed
package), add C in parallel as a quick win, and do B as a later SHA bump once A
is accepted. Catalog keys: `uteke` for the memory plugin (A, later B) and
`uteke-mcp` for C.

## 4. Plan

### Phase 0: decisions and evidence (owner, 1 day)

- [ ] Catalog key and manifest name: `uteke` (recommended) or keep
      `uteke-memory`. `uteke` also means users who ran `uteke connect hermes`
      already have `~/.hermes/plugins/uteke-memory/`; installing both would
      inject recalls twice. `uteke connect hermes` should detect the old directory and say so.
- [ ] Who opens the PR: it must be an owner account (`ajianaz`) on a fork of
      `NousResearch/hermes-agent`.
- [ ] Add a CI job that runs the real validator against the package, since it
      was **not run for this plan** (installing Hermes into a scratch
      environment was blocked in the planning session):
      `hermes plugins validate integrations/hermes --install-deps`, pinned to a
      Hermes release. Start non-blocking, then required.
- [ ] Look up the current Hermes release and its SemVer floor for
      `requires_hermes` (recent entries use `>=0.21.6`; it must never exceed the
      current release).

### Phase 1: make `integrations/hermes` catalog-grade (A)

Edit the **embedded templates** (`crates/uteke-cli/assets/hermes-uteke-memory/*.tmpl`,
the same source `uteke init` installs from), then run
`scripts/render-hermes-integration.sh` and commit the result: a test in
`crates/uteke-cli/src/init.rs` fails when `integrations/hermes/` drifts from the
templates. Keep `extensions/hermes-uteke-memory/` in step.

- [ ] Secrets: read the token with `agent.secret_scope.get_secret("UTEKE_TOKEN")`,
      declare it (`requires_env` / `config_schema`, `type: secret`), stop reading
      it from `uteke.json`; keep a fallback to the old path for older Hermes.
- [ ] Child environment: pass an allow-list, not `dict(os.environ)`.
- [ ] Manifest: `name`, `version` (plugin SemVer), `requires_hermes` in
      `plugin.yaml`, `capabilities` that match the code.
- [ ] Tests: unit tests for the secret path and the env allow-list; keep
      `test_recall_parse.py`.
- [ ] README written for users, with the disclosure (below), install
      (`hermes plugins install uteke`, `uteke` binary from the signed release),
      config, uninstall, known issues. Banner (1200x600) and up to 6
      screenshots under `docs/assets/`, referenced by pinned raw URL.
- [ ] Fix `docs/integrations/hermes.md` (memory-provider statement, new install
      path) and the docstring.

### Phase 2: entry PR to `NousResearch/hermes-agent`

- [ ] Pin `sha` to a commit on `main` that contains the finished package. Hermes
      clones the whole repository at that commit, and ours is a large Rust
      workspace; measure the clone time. If it is a problem, move the package to
      a small repo (`codecoradev/hermes-uteke`) before the first pin.
- [ ] Add `plugin-catalog/uteke.yaml` (draft below), run
      `hermes plugins validate` on a checkout of the pinned commit, fix every
      failure and read every warning.
- [ ] PR description: what it does, which Hermes surfaces it uses, and the
      rule-13 disclosure lines. Wait for the catalog CI job, answer the reviewer.
- [ ] Do not ask for `verified` in the PR. After listing, a maintainer may
      consider it; mem0 and hindsight are community tier even though they are
      the vendors.

Draft entry (placeholders in angle brackets):

```yaml
name: uteke
title: Uteke
repo: https://github.com/codecoradev/uteke
sha: <40-hex commit on main>
subdir: integrations/hermes
version: "<plugin semver>"
description: >-
  Offline-first long-term memory: one Rust binary, hybrid vector and full-text
  recall, no API key. Recalls relevant memories into every turn. Disclosure -
  runs the separately installed `uteke` binary as a subprocess (or talks to a
  `uteke-serve` / Uteke Cloud URL you configure); recalled text is injected into
  the user message; the token is read through Hermes's secret scope; nothing is
  sent anywhere unless you configure a server URL; no telemetry, no
  self-updater, no background daemon started by the plugin.
maintainer: codecoradev
tier: community
category: memory
requires_hermes: ">=<floor>"
docs_url: https://github.com/codecoradev/uteke/blob/<sha>/integrations/hermes/README.md
image: https://raw.githubusercontent.com/codecoradev/uteke/<sha>/docs/assets/hermes-banner.png
platforms: [linux, macos]
capabilities:
  provides_tools: []
  provides_hooks: [pre_llm_call]
  provides_middleware: []
  requires_env: [UTEKE_TOKEN]
known_issues:
  - Needs the `uteke` binary on PATH (or UTEKE_BIN); install it from the signed GitHub release.
```

### Phase 3: portable package `uteke-mcp` (C), in parallel

- [ ] `integrations/agent-plugin/` with `plugin.json`, `skills/uteke/SKILL.md`
      (copy of the skills repo's `skills/uteke`), `mcp.json` (stdio `uteke-mcp`,
      `UTEKE_*` passed as `"NAME": "${NAME}"`, no secrets inline).
- [ ] Validate with the same CI job; second catalog entry `uteke-mcp`
      (`category: tools`), same disclosure.
- [ ] Reuse it for Cursor Marketplace and cursor.directory (the plan in
      `2026-11-10-directory-listings.md`, section 5, asks for exactly this format).
- [ ] Open question: whether a Uteke Cloud variant (`streamable-http`, OAuth) is
      allowed to carry the token header; the docs forbid credentials in
      `mcp.json`. Test with the validator before promising it.

### Phase 4: native memory provider (B), after Phase 2 is accepted

- [ ] Implement `MemoryProvider`: `name`, `is_available` (no network),
      `initialize`, `get_tool_schemas` (remember, recall, search, forget, rooms),
      `handle_tool_call`, `get_config_schema`, `save_config`, `prefetch`,
      `queue_prefetch`, `sync_turn`, `on_session_end`, `on_pre_compress`.
      Skip automatic writes when `agent_context` is `cron` or `subagent`.
- [ ] Capture through `uteke capture` / `extract-queue` (#1466), **off by
      default** and disclosed; it already refuses secrets (`[privacy]`).
- [ ] plugin.yaml `kind: exclusive`; the provider registers the name `uteke`
      (reviewers check the registered name, not only the file name).
- [ ] Ship as a SHA-bump PR; `capabilities.provides_tools` must list the tools.

### Phase 5: after it merges

- [ ] Smoke test in CI: `hermes plugins install uteke` against the pinned
      commit.
- [ ] Release routine: add a step to `AGENTS.md` (after the tag) to open a
      SHA-bump PR to `hermes-agent` whenever `integrations/hermes` changed, and
      a small script that prints the `sha`/`version` diff. Updates reach users
      only this way.
- [ ] Verify the live page `/marketplace/uteke` and that the README renders.
- [ ] Tick the Hermes row in `2026-11-10-directory-listings.md`.

## 5. Using Nginjen

Nginjen (the scrape and search MCP tools) worked for every page read here:
the plugin detail page, the submission guide and the search for the portable
format. Uses beyond this plan:

- **Monitoring**: a weekly scrape of `/marketplace?kind=plugins` filtered to
  Memory, to track competitors (rank, stars, new entries) and to confirm our
  entry is still listed (a delisting is silent unless a rule changes).
- **Pre-flight**: scrape the submission guide and `plugin-catalog/README.md`
  before each PR and diff against the last copy, since the rules change.
- **Nginjen as its own listing**: the Web shelf (66 entries) has scrape and
  search backends (for example unbrowse, a `web_extract` backend). A portable
  package with Nginjen's streamable-HTTP MCP endpoint would fit rule 3 and 17
  with no Python. Owner decision; same admission rules apply.

## 6. Risks and open points

- **Review latency and a human gate.** No timeline is published; a reviewer may
  decline on rules 3 or 9 rather than ask for a fix.
- **Rank.** 286 stars against 3,600 for the closest local rival. Compete on the
  card: a short, concrete description (single binary, offline, recall figures
  with a link to `benchmarks/RECEIPTS.md`), a banner and screenshots.
- **Double install** of `uteke-memory` and `uteke`; handled in Phase 0.
- **Clone size** of the monorepo at install time (Phase 2).
- **Not verified here**: the validator was not run; acceptance of a hook-only
  plugin on the Memory shelf is inferred from deja-memory and from the schema,
  not confirmed with a maintainer; the exact `requires_hermes` floor.
