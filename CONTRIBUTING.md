# Contributing to Uteke

Uteke is a solo-maintained project with a strong product direction. Contributions are welcome, but **alignment matters more than volume**.

This document helps you decide *whether* and *how* to contribute in a way that's likely to get merged, so neither of us wastes time.

## How this project is run

- Uteke has one active maintainer ([@ajianaz](https://github.com/ajianaz)).
- Review bandwidth is limited.
- Not every contribution can be accepted, even if it's technically correct. Alignment with project direction matters as much as code quality.
- For scope and direction, check open issues and [ROADMAP.md](ROADMAP.md) if available. Read them before opening anything non-trivial.

This is normal for a solo project. A "no" on a PR is not personal.

## Quick start

```bash
# Prerequisites: Rust 1.75+ (via https://rustup.rs), Git
git clone https://github.com/codecoradev/uteke.git
cd uteke
cargo build --workspace
cargo test --workspace
```

## Where to discuss

Use GitHub Issues for tracking concrete bugs and features. For design discussions or "should I work on X?", open an issue first.

## What makes a good contribution

These get merged fast:

- **Bug fixes** with clear reproduction steps and tests.
- **Docs / typos / small UX fixes** — open a PR directly.
- **Pre-discussed features** — alignment in an issue first.
- **Small, focused changes** — easy to review, low risk.

If your change is small and obvious (typo, narrow bugfix, small docs change), open a PR directly. No issue required.

## Keep changes focused

**Only change what's needed to accomplish your stated goal.**

If you're fixing a bug in `store.rs`, don't also:

- Reformat other files
- Clean up unrelated code
- Fix lint issues in files you didn't need to touch
- Combine multiple unrelated fixes in one PR

**One PR = one logical change.** Multi-concern PRs will be asked to split.

## Discuss first (required for larger changes)

For anything beyond a small fix, **discussion is required before opening a PR**. This includes:

- New features
- API changes or new endpoints
- Refactors or "cleanup" work
- Performance rewrites
- Architectural changes
- Anything touching many files or subsystems
- Changes to the embedding, vector search, or FTS5 subsystems

Pull requests with significant unsolicited changes will be closed without detailed review. This isn't meant to discourage contribution. It ensures alignment before significant work goes in.

A 10-minute conversation saves a 500-line PR that doesn't fit the roadmap.

## Quality bar

Every PR is reviewed against:

- `cargo fmt --all -- --check` — must be clean
- `cargo clippy --workspace --all-targets -- -D warnings` — must be clean
- `cargo test --workspace` — must pass
- [Cora review](https://github.com/codecoradev/cora-cli) — run locally before pushing (`cora review --base origin/develop`)
- `cargo build --release --workspace` — must compile
- No new heavy dependencies without justification
- No perf regressions in hot paths: embedding, vector search, FTS5, server response latency

If you're not sure how to measure perf or what counts as a hot path, ask in an issue. Better to confirm than get bounced.

## Changes to core subsystems require a test

The most common way a PR breaks Uteke is a **local fix with global blast radius**: the diff solves one case, reads fine, passes clippy, and silently breaks the same subsystem in other cases. Review alone does not catch these. A test does.

If your change touches behavior in any of these load-bearing paths, the PR must add or extend a test:

- **Memory storage (store.rs)**: SQLite writes, schema migrations, atomic file operations
- **Vector search (vector.rs)**: index read/write, normalization, similarity computation
- **FTS5 hybrid search (fts5.rs)**: tokenization, RRF fusion, ranking
- **Embedding (embed/)**: ONNX inference, tokenizer, dimension handling
- **Schema migration (schema.rs)**: version upgrades, data migration
- **API server (uteke-server/)**: endpoint registration, request/response handling
- **CLI commands (uteke-cli/)**: argument parsing, output formatting

The bar for the test is real coverage of the contract, not a placeholder. Test the edge case that would actually break. If you can't see how to test it, ask in an issue before opening the PR.

UI rendering, themes, and anything the type-checker already guarantees do not need tests.

## What Uteke is not

To set expectations:

- Not trying to be a full knowledge management platform (Notion, Obsidian, Outline).
- Not building: web UI, graph visualization, multi-user collaboration, enterprise SSO.
- Not a curated "first open-source contribution" project. Beginners are welcome but expect normal review.
- Mechanical refactors, broad style changes, drive-by rewrites are not helpful.
- AI-assisted contributions are welcome, but the PR must reflect understanding of the existing patterns. Low-effort AI-generated code that wasn't read by the author will be closed.

## Branches

Branch off `develop`. Use these prefixes (kebab-case):

| Prefix        | Use for                                  |
| ------------- | ---------------------------------------- |
| `feat/`       | New feature                              |
| `fix/`        | Bug fix                                  |
| `chore/`      | Refactor, tooling, config, dependencies  |
| `docs/`       | Docs-only changes                        |
| `perf/`       | Performance work                         |
| `security/`   | Security fix or hardening                |
| `refactor/`   | Code restructuring                       |
| `test/`       | Test additions/changes                   |

Examples: `feat/namespace-memory`, `fix/rrf-normalization`, `security/path-guard`.

Don't open PRs from your fork's `develop` or `main` branch. Work on a feature branch.

## Commits & PRs

The **PR title becomes the squash commit** for most PRs. Title must follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(recall): add --entity flag for filtered recall
fix(fts5): handle empty query without panic
chore(deps): bump usearch to 0.4.0
security(server): tighten CORS headers
docs(readme): update installation instructions
```

Types: `feat`, `fix`, `chore`, `docs`, `perf`, `refactor`, `test`, `build`, `ci`, `security`.

Common scopes: `recall`, `remember`, `server`, `cli`, `fts5`, `vector`, `embed`, `schema`, `store`, `consolidate`, `search`.

**Fill out the PR template.** Include: what changed, why, how you tested. The more specific, the faster the review.

**Open a draft PR early** if you want feedback mid-flight. Mark "Ready for review" when done.

### What gets merged faster

- Clear problem statement
- Small, focused diff
- Follows existing patterns (read 2–3 nearby files before writing yours)
- All checks pass (fmt, clippy, tests, Cora)
- Manual testing notes describing the steps you took

### What gets bounced back

- Mixed-concern PRs
- Large architectural PRs without prior discussion
- New dependencies without justification
- Breaking changes without migration notes
- Incidental reformatting unrelated to the change
- AI-generated code that obviously wasn't read by the author

## Code Review with Cora CLI

[Cora](https://github.com/codecoradev/cora-cli) is an AI-powered code review tool that runs automatically on every PR via CI. It uses SARIF output and posts review comments directly on the PR.

### CI (Automatic)

Every PR to `develop` or `main` triggers the `Cora Review` CI job:

- Downloads the latest `cora-cli` binary
- Runs `cora review --base origin/develop --format sarif --severity major`
- Posts results as a PR comment (grouped by severity)
- **Blocks merge** if any Error-level issues are found

### Local (Recommended)

Run Cora locally **before pushing** to catch issues early:

```bash
# Review your uncommitted changes
cora review --base HEAD~1 --format text

# Review against develop
cora review --base origin/develop --format text
```

> **Tip:** Cora found real bugs in Uteke's own PRs (BM25 score always 0, missing metadata in server mode, RRF normalization). Running it locally saves CI cycles.

## Code Style

- Follow existing patterns. Read 2–3 adjacent files before adding new ones.
- Rust: `cargo fmt` + `cargo clippy` clean. Clippy warnings are errors in CI.
- Comments: only for *why*, not *what*. Code should explain itself.
- No emojis in code or commit messages.

## Architecture

Uteke is a Cargo workspace:

| Crate | Purpose |
|-------|---------|
| `uteke-core` | Library — storage, embedding, vector search, FTS5 |
| `uteke-cli` | CLI binary — clap commands, JSON output |
| `uteke-server` | HTTP server — persistent daemon for fast agent access |

```
crates/
├── uteke-core/             # Memory engine library
│   └── src/
│       ├── lib.rs          # Uteke struct — main API
│       ├── memory/         # SQLite store + usearch vector index
│       ├── embed/          # ONNX embedding engine
│       └── ...
├── uteke-cli/              # CLI binary
│   └── src/
│       └── commands/       # Per-command modules
└── uteke-server/           # HTTP server binary
    └── src/
        └── main.rs         # Actix-web server
```

### Key Design Decisions

- **RwLock for vector index** — Read-heavy workload (recall/search) benefits from shared read locks; write ops take exclusive write lock
- **Mutex for embedder** — ONNX tokenizer requires `&mut self` internally
- **FTS5 hybrid search** — Vector similarity merged with FTS5 full-text search via Reciprocal Rank Fusion (RRF, k=60)
- **SQLite-first dual-write** — `remember()` writes to SQLite before vector index
- **Atomic file writes** — All file saves use `.tmp` + rename pattern
- **Schema versioning** — Integer counter; auto-migration on upgrade

## Reporting Issues

- **Bugs:** Use the [Bug Report](https://github.com/codecoradev/uteke/issues/new?template=bug_report.yml) template
- **Features:** Use the [Feature Request](https://github.com/codecoradev/uteke/issues/new?template=feature_request.yml) template

## FAQ

**Q: Should I ask before fixing a typo or obvious bug?**
A: No, open a PR directly.

**Q: I have an idea for a new feature.**
A: Open a GitHub issue. Don't open a PR without prior discussion.

**Q: My PR was closed without detailed feedback.**
A: Usually means it didn't align with project direction, or scope was too large to review responsibly. This is normal for a solo project.

**Q: Can I work on an open issue?**
A: Comment first to confirm it's still relevant. For anything non-trivial, discuss approach before implementing.

**Q: My PR conflicts after develop moved. Should I rebase?**
A: If the change is still relevant and reasonably small, yes. Large stale PRs may be closed with an offer to reopen after rebase.

## Security issues

Don't file them as public issues. See [SECURITY.md](SECURITY.md).

## Deep-dive guides

Detailed contributor and maintainer guides live in [`docs/contributing/`](docs/contributing/):

- [Testing guide](docs/contributing/testing.md) — testing contract, subsystem invariants, what makes a good test

## Code of Conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

Contributions are licensed to the project under the Apache-2.0 license (inbound = outbound). No additional agreement is required.

## Contributions are unpaid

Contributing to this project is **voluntary and unpaid**. There is no
compensation, payment, bounty, or financial reward of any kind for
contributions — now or in the future. You contribute on your own time,
at your own discretion, because you want to improve the project.

If any paid-contribution program is ever introduced, it will be announced
explicitly and this document will be updated. Until then, assume every
contribution is volunteer work under the Apache-2.0 license terms above.
