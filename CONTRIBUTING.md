# Contributing to Uteke

Thanks for your interest in contributing! This guide covers the basics.

## Prerequisites

- **Rust** 1.75+ — install via [rustup](https://rustup.rs/)
- **Git**

## Build

```bash
git clone https://github.com/ajianaz/uteke.git
cd uteke
cargo build --workspace
```

## Test

```bash
# Run all tests
cargo test --workspace

# Run tests for a specific crate
cargo test -p uteke-core
cargo test -p uteke-cli
```

## Code Style

```bash
# Format check — must pass
cargo fmt --all -- --check

# Lint — must pass with no warnings
cargo clippy --workspace --all-targets -- -D warnings
```

Run `cargo fmt` before committing. Clippy warnings are treated as errors in CI.

## Code Review with Cora CLI

[Cora](https://github.com/ajianaz/cora-cli) is an AI-powered code review tool that runs automatically on every PR via CI. It uses SARIF output and posts review comments directly on the PR.

### CI (Automatic)

Every PR to `develop` or `main` triggers the `Cora Review` CI job:

- Downloads the latest `cora-cli` binary
- Runs `cora review --base origin/develop --format sarif --severity major`
- Posts results as a PR comment (grouped by severity: 🔴 Error / 🟡 Warning / 🔵 Note)
- **Blocks merge** if any Error-level issues are found

### Local (Recommended)

Run Cora locally **before pushing** to catch issues early:

```bash
# Install cora-cli (one-time)
# Download from https://github.com/ajianaz/cora-cli/releases

# Review your uncommitted changes
cora review --base HEAD~1 --format text

# Review against develop
cora review --base origin/develop --format text
```

> **Tip:** Cora found real bugs in Uteke's own PRs (BM25 score always 0, missing metadata in server mode, RRF normalization). Running it locally saves CI cycles.

### Configuration

The Cora CI action is at `.github/actions/cora-review/action.yml`:

| Input | Default | Description |
|-------|---------|-------------|
| `base-branch` | `origin/develop` | Branch to compare against |
| `severity` | `major` | Minimum severity to report |
| `cora-version` | `latest` | cora-cli version tag |
| `upload-sarif` | `false` | Upload SARIF to GitHub Code Scanning |

LLM secrets are fetched via Infisical OIDC — no API keys stored in GitHub.

### Cora + Code Scanning

To enable SARIF upload to GitHub Code Scanning tab:

```yaml
# In .github/workflows/cora-review.yml
uses: ./.github/actions/cora-review
with:
  upload-sarif: 'true'
```

## Submitting a PR

1. **Fork** the repository
2. **Create a branch** from `develop` — use descriptive names like `fix/embedding-crash` or `feat/export-command`
3. **Make your changes** — keep PRs focused and small
4. **Add tests** for new functionality
5. **Run Cora locally** to catch review issues early
6. **Ensure CI passes** — `cargo test`, `cargo fmt`, `cargo clippy`, Cora Review all green
7. **Open a Pull Request** against the `develop` branch

### CI Checks

Every PR runs these checks (all must pass before merge):

| Check | Description |
|-------|-------------|
| Build | `cargo build --workspace` |
| Check | `cargo check --workspace` |
| Clippy | `cargo clippy` — warnings = error |
| Format | `cargo fmt --check` — must be formatted |
| Test | `cargo test --workspace` — 107 unit tests |
| Cora Review | AI code review (blocking on errors) |
| CodeRabbit | AI review (non-blocking, advisory) |
| Cargo Audit | Dependency vulnerability scan |
| Trivy FS Scan | Filesystem security scan |
| GitGuardian | Secret leak detection |

## Commit Messages

Use clear, descriptive commit messages:

```
fix: handle empty query in recall command
feat: add --entity flag to remember command
docs: update README with hybrid search section
refactor: extract embedding normalization into helper
```

Prefix with type: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`.

## Architecture

Uteke is a Cargo workspace with three crates:

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
│       │   ├── store.rs    # Store struct — SQLite operations
│       │   ├── vector.rs   # Vector index (usearch, RwLock)
│       │   ├── fts5.rs     # FTS5 full-text search + RRF fusion
│       │   ├── schema.rs   # Schema versioning + migrations
│       │   ├── crud.rs     # CRUD operations
│       │   ├── types.rs    # Type definitions (Memory, RecallResult, etc.)
│       │   └── mod.rs      # Module re-exports
│       ├── embed/          # ONNX embedding engine
│       ├── operations.rs   # High-level operations (remember, recall, etc.)
│       ├── consolidate.rs  # Memory consolidation (dedup)
│       ├── maintenance.rs  # Doctor, verify, repair
│       ├── import_export.rs # JSONL import/export
│       └── error.rs        # Error types
├── uteke-cli/              # CLI binary
│   └── src/
│       ├── main.rs         # Entry point
│       └── commands/       # Per-command modules
│           ├── remember.rs
│           ├── recall.rs
│           ├── list.rs
│           ├── server.rs   # Server proxy
│           └── ...         # Other commands
└── uteke-server/           # HTTP server binary
    └── src/
        └── main.rs         # Actix-web server
```

### Key Design Decisions

- **RwLock for vector index** — Read-heavy workload (recall/search) benefits from shared read locks; write ops (remember/forget) take exclusive write lock
- **Mutex for embedder** — ONNX tokenizer requires `&mut self` internally; architectural limitation
- **FTS5 hybrid search** — Vector similarity merged with FTS5 full-text search via Reciprocal Rank Fusion (RRF, k=60)
- **SQLite-first dual-write** — `remember()` writes to SQLite before vector index; `forget()` acquires index lock before SQLite delete
- **Atomic file writes** — All file saves use `.tmp` + rename pattern to prevent corruption on crash
- **Schema versioning** — Integer counter in `schema_version` table; auto-migration on upgrade

## Reporting Issues

- **Bugs:** Use the [Bug Report](https://github.com/ajianaz/uteke/issues/new?template=bug_report.md) template
- **Features:** Use the [Feature Request](https://github.com/ajianaz/uteke/issues/new?template=feature_request.md) template

## Questions?

Open an issue with the `question` label. We're happy to help.

## License

By contributing, you agree that your contributions will be licensed under the [Apache License 2.0](LICENSE).
