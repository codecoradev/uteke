# AGENTS.md — uteke

## Project

Uteke — local-first memory engine for AI agents.
Rust workspace: uteke-core (engine), uteke-cli, uteke-server, uteke-mcp, docgen.

## Stack

- **Rust** (workspace, single version in root Cargo.toml)
- **Storage**: SQLite (rusqlite bundled) + dual-engine vector index
- **Vector engines**: usearch (default, C++ FFI) OR vecq (quantized, zero C++ dep) —
  runtime-selected via `UTEKE_VECTOR_BACKEND` env or `[vector] backend` in uteke.toml (#1168)
- **Embeddings**: EmbeddingGemma Q4 ONNX (default) / OpenAI / Ollama
- **Package manager**: BUN for any JS/tooling (not npm). Rust uses cargo.
- **Git author**: email MUST be ajianaz@users.noreply.github.com

## Branch & release rules (STANDARD — applies to all CodeCora repos)

- **Branches**: `develop` = integration, `main` = release (tags cut from main).
- **Branch naming**: `feat/`, `fix/`, `docs/`, `chore/`, `perf/`, `security/`,
  `refactor/`, `test/`, `build/`, `ci/`. `release/x` and bare names are rejected.
- **main only accepts PRs from** `develop` or `chore/release-*` (Source Branch check).
- **No main→develop sync PR.** The repository rejects merge commits and `main` only
  receives squash commits, so `main` never becomes an ancestor of `develop` through a
  PR (a squash/rebase merge drops the `-s ours` merge and changes nothing, #1381).
  The divergence is resolved inside each release branch instead — see
  "Release procedure" below. Skipping that makes the develop→main PR conflict in
  ~30 files (merge-base stays at v0.13.2).
- **PR body**: `## What` / `## Why` / `## Testing` headers are REQUIRED (CI enforces).
- **CI is green + Cora bot clean** before merge; never trust one gate alone. "Clean"
  means a REAL verdict ("✅ No issues found" or concrete findings): "Review could not
  complete" / "empty result" is a blocker. Since #1386 the Cora job diffs against the
  PR's own base branch (`base-branch: origin/${{ github.base_ref }}`); before that it
  defaulted to `origin/develop`, so PRs into `main` were reviewed against an empty
  diff. Re-run the job once; if it is still empty, investigate instead of accepting.
- Version: single workspace version in root Cargo.toml; internal deps (uteke-core/
  uteke-mcp) must be bumped to the same version in the release commit.
- `docs/api-reference.md` is GENERATED (`cargo run -p docgen`) — never hand-edit.

## Release procedure (develop → main)

1. **Pre-flight.** The owner approves the version number: propose minor vs patch with
   the rationale and wait (never decide and tag in the same run). Check the CORE
   contract: every CORE behaviour change in the release needs a uteke-cloud contract
   issue, and its answer should be read first. Run the pre-release mutation gate
   (owner rule 2026-08-19, local only; the CI workflow only triggers on `develop*`
   head branches, so release PRs never run it) against the code changed since the
   previous release:
   `git diff vPREV..develop -- crates/uteke-core > /tmp/rel.diff` then
   `cargo mutants -p uteke-core --in-diff /tmp/rel.diff -j 2 --timeout 120`
   (install with `cargo install cargo-mutants --locked`; check `df -h` first).
   Measured on v0.20.0..develop: 38 mutants, about 5 minutes. A missed mutant in
   changed code gets a killing test (exact-value assertions); a miss only reachable
   by tests that need the ONNX runtime (absent in CI) is listed in the release PR.
   The full crate (`cargo mutants -p uteke-core -j 2`, 2058 mutants, an estimated
   8 hours, not measured) is optional. If the owner explicitly releases without
   the gate, say so in the PR body.
2. **Release-prep PR into `develop`** (`chore(release): vX.Y.Z - version bump,
   changelog, docs sync`): workspace `Cargo.toml`, the internal deps in the three
   crates, `Cargo.lock`, both READMEs' version line, the version line of both skill
   copies (`.agents/skills/uteke-memory/SKILL.md` and
   `crates/uteke-cli/assets/uteke-memory-skill.md`, enforced by tests),
   `docs/core-contract.json` (regenerate with `cargo run -p docgen`), and the
   CHANGELOG entry. Merge it with the normal merge gate.
3. **Release branch.** `git checkout -b chore/release-vX.Y.Z-main origin/develop`, then
   merge `main` keeping develop's side. First prove `main` has no content `develop`
   lacks: `git diff <develop's previous release commit> origin/main` is empty and
   `git diff --name-status origin/develop origin/main | awk '$1=="A"'` lists nothing.
   Then `git merge -s ours origin/main` and check `git diff origin/develop HEAD` is
   empty. If `main` DOES carry unique content, resolve by hand instead (never
   server-side: protected branch). Push the branch.
4. **PR into `main`** (`chore: release vX.Y.Z to main`), **squash** merge (the ruleset
   rejects merge commits). Apply the full merge gate; do not accept an empty Cora
   result without a written justification on the PR.
5. **Tag only after the merge.** Verify `origin/main` is the merge commit and its tree
   equals `develop`, then create an ANNOTATED tag on that commit
   (`git tag -a vX.Y.Z <sha>`) and push it. Tagging before the merge once pointed
   the tag at the previous release (v0.15.0). The Release workflow takes ~35 min
   (the legacy-ORT build is the long pole) and publishes GitHub assets, crates.io
   (uteke-core/cli/mcp), Docker tags and opens the Homebrew tap PR.
6. **After the tag.** Merge the homebrew-tap PR the workflow opened (its title/body
   follow the tap's rules since #1384, and the job already re-verified every SHA256
   against `checksums-sha256.txt`; Cora there was fixed in homebrew-tap#6), then
   `brew update && brew upgrade codecoradev/tap/uteke`. Verify crates.io and the
   Docker tags, upgrade the production server and run `uteke verify` (repair if
   `consistent` is false). Do not run `uteke-mcp --version` / `uteke-serve --version`
   in a shell: they start a server and hang.

## CORE/LAB contract — source of truth (MUST READ before API-surface work)

Uteke OSS ships a written compatibility contract to uteke-cloud: **CORE** endpoints
must stay semantically compatible; everything else is **LAB** (free to evolve, no
port promise). The canonical contract (CORE/LAB lists, change-control rules,
anti-drift mechanism, backlog) lives in the PROD uteke document tree
`kontrak-core-cloud` with sub-docs `01-aturan-perubahan`, `02-daftar-core`,
`03-daftar-lab`, `04-anti-drift`.

- Read it before touching any HTTP/MCP/CLI surface:
  `uteke doc get kontrak-core-cloud` (children:
  `uteke doc get kontrak-core-cloud/01-aturan-perubahan` … `/04-anti-drift`).
- **Change control (LOCKED):** the contract changes ONLY on an explicit owner
  (ajianaz) order, quoted in the parent doc's change history. Agents may draft
  amendment proposals as issues/room notes but must NOT edit the contract
  unprompted.
- Changing CORE semantics in OSS requires a contract issue on uteke-cloud FIRST.
  Execution backlog: cloud #81–#85; analysis archive: cloud issue #69.
- **uteke-cloud's tracker is Gitea** (`codecoradev/uteke-cloud` on
  gitea.azfirazka.com, e.g. `tea issues list --login <your-gitea-login> --repo
  codecoradev/uteke-cloud`). The GitHub repo of the same name is a stale mirror
  whose issue numbers do not match.

## Source of truth for workflow standards

The canonical workflow SOP (merge gate, release flow, governance) lives in the
uteke room `codecora-workflow-standard` (namespace `codecora`) on the shared
store. This file is the repo-enforced subset — when rules disagree, the room
wins, and this file gets updated in the same commit that changes the rule.

## Uteke memory conventions (Uteke Conventions v1, 2026-10-07)

**Standard**: read Uteke room `uteke-conventions` (namespace `important`; full
docs: slugs `uteke-conventions` and `uteke-project-registry`) BEFORE reading or
writing any memory. Claude Code sessions write to namespace `macmini`; never
write to `default`. Tag project memories `project:<nama>` per the registry
(component + parent tags when both apply, e.g. `project:samson-mobile` +
`project:samson`), plus `tool:<nama>` when the memory is specific to a tool.
The uteke project room is `uteke` (namespace `hermes`). No tokens or
credentials in memories.
