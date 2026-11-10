# Directory and list submissions

Status as of 2026-11-10. Rules below were read from each list's own
CONTRIBUTING file or docs that day; re-read the file before opening a PR, since
these lists change often. Tick a box when done, so nothing is submitted twice.

## 0. Already done (do not repeat)

- [x] Official MCP Registry: `io.github.codecoradev/uteke` 0.23.0, active
      (remote `https://uteke.app/mcp`). Published by `.github/workflows/mcp-registry.yml`.
- [x] awesome-rust: already listed under Applications > MLOps
      (`rust-unofficial/awesome-rust`). Optional: refresh its one-line description.
- [x] GitHub repo description and 20 topics, social preview image.
- [x] crates.io, Homebrew tap, Docker Hub, GitHub Releases (cosign-signed).

## 1. Free, automatic: wait, then verify

These import the official registry or crawl GitHub. No action except a check.
Look again about a week after 2026-11-10; if an entry is missing then, use the
manual route in the same row.

| Where | Expected | If missing |
|---|---|---|
| Claude app / host MCP catalog | next refresh of the registry copy (hourly poll) | none, just wait |
| PulseMCP | ingests the registry; its submit form is closed | none (do not email) |
| Glama | indexes GitHub and the registry | section 2, step 1 |
| mcp.directory | imports the registry | `/submit` form |
| API Tracker (apitracker.io/mcp-servers) | imports the registry | submit form |
| ClaudePluginHub | crawls GitHub | submit the repo URL |

- [ ] Verify each row once, a week later.

## 2. The critical chain: Glama, then awesome-mcp-servers

`punkpeye/awesome-mcp-servers` (about 97k stars, about 4,600 entries) is the
biggest single list. Its CI expects every new entry to carry a Glama score
badge, so Glama comes first.

1. [ ] **Glama listing.** Search `glama.ai/mcp/servers` for `uteke` (nothing on
       2026-11-10). If absent, use "Add Server" (needs a signed-in account:
       GitHub works; the email route has a captcha, so an owner must do it).
       Then **claim** it (`glama.json` is already in the repo root, naming
       `ajianaz`; redo the "Claim ownership" flow so Glama reads it).
       Glama grades licence, quality and maintenance; Apache-2.0 helps.
2. [ ] Open `https://glama.ai/mcp/servers/codecoradev/uteke` and confirm the
       badge image `.../badges/score.svg` renders.
3. [ ] **PR to awesome-mcp-servers**, section "Knowledge & Memory". Fork,
       branch `add-uteke`, one line, keep alphabetical order among neighbours
       (the guideline; check the lines around yours on the day):

```markdown
- [codecoradev/uteke](https://github.com/codecoradev/uteke) [![codecoradev/uteke MCP server](https://glama.ai/mcp/servers/codecoradev/uteke/badges/score.svg)](https://glama.ai/mcp/servers/codecoradev/uteke) 🦀 🏠 ☁️ 🍎 🪟 🐧 - Long-term memory for AI agents: one offline Rust binary with hybrid vector and full-text recall, rooms for shared memory, point-in-time recall. Also hosted as Uteke Cloud.
```

   Legend: 🦀 Rust, 🏠 local, ☁️ cloud, 🍎🪟🐧 macOS/Windows/Linux.
   The list fast-tracks agent-authored PRs that end the title with `🤖🤖🤖`;
   only add it if an agent opens the PR. The queue is long (3,800+ open PRs on
   2026-10-31): expect weeks. After the merge, check the entry is still in the
   README a month later (a previous submitter reported theirs dropped).

## 3. Remote list: awesome-remote-mcp-servers (uncertain)

`punkpeye/awesome-remote-mcp-servers` is for hosted servers. Requirements:
public URL that answers `initialize`, Streamable HTTP, public sign-up allowed,
and a Glama **connector** listing with its badge.

Uteke Cloud answers `initialize` with `401` plus `WWW-Authenticate` and OAuth
metadata (checked 2026-11-10). The list has a 🔐 marker for OAuth servers, but
Glama may flag a token-gated endpoint as unhealthy. So:

- [ ] Add the connector on Glama (`glama.ai/mcp/connectors`); wait for its
      health check. Only if it is healthy, open the PR (three-line entry,
      section "Knowledge & Memory", marker 🔐). Otherwise skip, do not argue.

## 4. Memory-specific lists (no Glama needed)

- [ ] `topoteretes/awesome-ai-memory`: table row. Columns: Name, URL,
      Open/Close, GitHub URL, Category, Storage. Row: `Uteke | https://uteke.app |
      Managed, Open source | https://github.com/codecoradev/uteke | Memory Tool |
      Vector`. (Its README says "open a pull request"; there is no CONTRIBUTING file.)
- [ ] `TeleAI-UAGI/Awesome-Agent-Memory`: read its CONTRIBUTING first (not read yet).
- [ ] Hermes Atlas "Best memory providers" (`hermesatlas.com/lists/best-memory-providers`):
      Uteke has a Hermes integration (`docs/integrations/hermes.md`). Find its
      submission route first (not read yet); competitors listed there include
      mnemosyne, ClawMem, Sibyl-Memory.
- [ ] `hesreallyhim/awesome-claude-code`: one submission through their issue
      form (Uteke ships Claude Code hooks via `uteke connect claude`). They
      reject promotion-style entries, so describe the hooks, not the product.
      Requirement: 100+ stars (285 today) or 14+ days old. One shot only.

## 5. Assistant marketplaces (need prerequisites)

| Where | What it takes | Cost | Decision |
|---|---|---|---|
| Cursor Marketplace | open source; `.cursor-plugin/plugin.json` + root `mcp.json`; publisher application at cursor.com/marketplace/publish; manual review | free | worth it |
| cursor.directory | GitHub repo, Agent Plugins format | free | do with the row above |
| Cline marketplace | issue on `cline/mcp-marketplace` with repo URL, 400x400 PNG logo, reason; needs `llms-install.md` | free | worth it |
| Gemini CLI extensions | `gemini-extension.json` at repo root + repo topic `gemini-cli-extension`; crawled within a day | free | cheapest win |
| Claude connector / plugin directory | portal needs a Claude Team or Enterprise org (about $50/month), privacy-policy URL, tool annotations, a test account | paid | owner decision |
| Docker MCP Catalog | PR to `docker/mcp-registry`; our image's entrypoint always runs `uteke-serve` (HTTP), not stdio `uteke-mcp` | free | blocked until a stdio mode exists |
| MCP Market | GitHub repo URL; free queue 4 to 6 weeks, or $29 for 24 hours | free / $29 | free queue |
| mcpservers.org | free form | free | do |
| Microsoft MCP Center | comment on a GitHub issue, 1 to 2 weeks, about 110 curated servers | free | low value, skip for now |
| mcp.so | $39 only path found; reports of a stub duplicate | $39 | skip |
| Smithery | proxy shut down 2026-10-31 | n/a | skip |
| LobeHub | claiming needs broad GitHub OAuth scopes | n/a | skip |
| goose | closed; points to the official registry | n/a | skip |

## 6. Prerequisites in this repo (do before sections 3 and 5)

Checked against `crates/uteke-mcp/src/lib.rs` on 2026-11-10:

- [ ] **Tool annotations** (`title`, `readOnlyHint`, `destructiveHint`,
      `openWorldHint`) are absent. Required by Anthropic, shown by Cursor and Smithery.
- [ ] **`resources/list` and `prompts/list`** are not handled. Scanners expect
      empty lists, not "method not found".
- [ ] Errors from tools should come back as results with `isError: true`
      (partly done; verify per tool).
- [x] `llms-install.md` at the repo root (Cline installs from it).
- [ ] A privacy-policy URL on uteke.app.
- [x] Manifests `glama.json` and `gemini-extension.json` (version kept in step by
      `registry_manifests_match_workspace_version`).
- [ ] Manifest `.cursor-plugin/plugin.json` (with the Cursor Marketplace application).
- [ ] Repo topic `gemini-cli-extension` (the gallery crawler needs it; the repo is
      at the 20-topic limit, so swap out one, e.g. `rag`).

The first three touch the MCP surface. Per AGENTS.md, read
`kontrak-core-cloud` first; if CORE semantics change, a uteke-cloud contract
issue comes first. Adding annotations and empty lists should be additive.

## 7. Launch posts (owner decision, outside the lists)

Listings bring steady trickle; a spike on Trendshift needs one concentrated
post. Candidates: Show HN, r/rust, r/mcp (the 70+ memory systems landscape
thread), r/LocalLLaMA, X, dev.to, This Week in Rust (submit a PR to
`rust-lang/this-week-in-rust`). One post per channel, at least a few days apart,
after the README demo and social preview are live (both are).

## Order of work

1. Section 6 prerequisites that are cheap (`glama.json`, `llms-install.md`,
   `gemini-extension.json`) in one PR.
2. Section 2 (Glama, then awesome-mcp-servers). Longest queue, start first.
3. Section 4 and the free rows of section 5 (one PR each).
4. Section 1 verification a week later.
5. Section 6 MCP annotations as their own code PR; then Cursor, Cline, and,
   if wanted, the Claude directories.
