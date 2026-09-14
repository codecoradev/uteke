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
- **POST-RELEASE SYNC IS MANDATORY**: after a release tag is cut from main, merge
  `main` back into `develop` immediately (PR if develop is protected). Skipping this
  makes the next develop→main PR conflict — paid on 0.17.0 (2026-09-06).
- **PR body**: `## What` / `## Why` / `## Testing` headers are REQUIRED (CI enforces).
- **CI is green + Cora bot clean** before merge; never trust one gate alone.
- Version: single workspace version in root Cargo.toml; internal deps (uteke-core/
  uteke-mcp) must be bumped to the same version in the release commit.
- `docs/api-reference.md` is GENERATED (`cargo run -p docgen`) — never hand-edit.

## Conflicts at release time

When develop→main conflicts, do NOT hand-resolve server-side (protected branch
rejects direct pushes). Instead: branch from the release commit, `git merge main`,
resolve keeping develop's side (unless main carries unique content — verify), push
the branch as `chore/release-*`, and open the release PR from it.

## Source of truth for workflow standards

The canonical workflow SOP (merge gate, release flow, governance) lives in the
uteke room `codecora-workflow-standard` (namespace `codecora`) on the shared
store. This file is the repo-enforced subset — when rules disagree, the room
wins, and this file gets updated in the same commit that changes the rule.
