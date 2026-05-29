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

## Submitting a PR

1. **Fork** the repository
2. **Create a branch** from `develop` — use descriptive names like `fix/embedding-crash` or `feat/export-command`
3. **Make your changes** — keep PRs focused and small
4. **Add tests** for new functionality
5. **Ensure CI passes** — `cargo test`, `cargo fmt`, `cargo clippy` all green
6. **Open a Pull Request** against the `develop` branch

## Commit Messages

Use clear, descriptive commit messages:

```
fix: handle empty query in recall command
feat: add --limit flag to list command
docs: update README with shell completion examples
refactor: extract embedding normalization into helper
```

Prefix with type: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`.

## Architecture

Uteke is a Cargo workspace with two crates:

| Crate | Purpose |
|-------|---------|
| `uteke-core` | Library — storage, embedding, vector search |
| `uteke-cli` | CLI binary — clap commands, JSON output |

```
crates/
├── uteke-core/         # Memory engine library
│   └── src/
│       ├── lib.rs      # Uteke struct — main API
│       ├── memory/     # SQLite store + usearch vector index
│       └── embed/      # ONNX embedding engine
└── uteke-cli/          # CLI binary
    └── src/main.rs     # clap commands
```

## Reporting Issues

- **Bugs:** Use the [Bug Report](https://github.com/ajianaz/uteke/issues/new?template=bug_report.md) template
- **Features:** Use the [Feature Request](https://github.com/ajianaz/uteke/issues/new?template=feature_request.md) template

## Questions?

Open an issue with the `question` label. We're happy to help.

## License

By contributing, you agree that your contributions will be licensed under the [Apache License 2.0](LICENSE).
