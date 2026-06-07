---
title: Roadmap
---

# Roadmap

Demand-gated — we build what people actually use. Track progress on [GitHub Issues](https://github.com/ajianaz/uteke/issues).

## v0.0.2 — Core Engine `✓ Done`

- [#40 Persistent vector index (usearch)](https://github.com/ajianaz/uteke/issues/40) `✓ Done`
- [#39 Multi-agent namespaces](https://github.com/ajianaz/uteke/issues/39) `✓ Done`
- [#38 Tiered memory (Hot/Warm/Cold)](https://github.com/ajianaz/uteke/issues/38) `✓ Done`
- [#37 Health checks (doctor, verify, repair)](https://github.com/ajianaz/uteke/issues/37) `✓ Done`
- [#53 Website (landing + docs)](https://github.com/ajianaz/uteke/issues/53) `✓ Done`

## v0.0.3 — Tag Management & Operations `✓ Done`

- [#42 Tag-based filtering and management](https://github.com/ajianaz/uteke/issues/42) `✓ Done`
- [#44 Memory aging with auto-cleanup](https://github.com/ajianaz/uteke/issues/44) `✓ Done`
- [#47 Shell hook for auto-context loading](https://github.com/ajianaz/uteke/issues/47) `✓ Done`
- [#48 Configuration file (uteke.toml)](https://github.com/ajianaz/uteke/issues/48) `✓ Done`
- [#31 Graceful shutdown (SIGINT handler)](https://github.com/ajianaz/uteke/issues/31) `✓ Done`
- [#32 File logging with daily rotation](https://github.com/ajianaz/uteke/issues/32) `✓ Done`
- [#41 Upgrade Node.js 20→24 in CI](https://github.com/ajianaz/uteke/issues/41) `✓ Done`

## v0.0.4 — Server Mode & Intelligence `✓ Done`

- [#43 Bulk memory operations (tag, cold, all)](https://github.com/ajianaz/uteke/issues/43) `✓ Done`
- [#45 Namespace switching & defaults](https://github.com/ajianaz/uteke/issues/45) `✓ Done`
- [#54 Daemon/server mode — warm recall <50ms](https://github.com/ajianaz/uteke/issues/54) `✓ Done`
- [#54 CLI auto-routes to server (21ms recall)](https://github.com/ajianaz/uteke/issues/54) `✓ Done`
- [#51 Auto-forget & temporal facts](https://github.com/ajianaz/uteke/issues/51) `✓ Done`
- [#52 Consolidation & deduplication](https://github.com/ajianaz/uteke/issues/52) `✓ Done`
- [#86 Cora AI code review in CI](https://github.com/ajianaz/uteke/issues/86) `✓ Done`

## v0.0.5 — Docker & Deployment `✓ Done`

- [#95 UTEKE_HOME env var for custom data directory](https://github.com/ajianaz/uteke/issues/95) `✓ Done`
- [#96 uteke-serve reads uteke.toml, default host 0.0.0.0](https://github.com/ajianaz/uteke/issues/96) `✓ Done`
- [#97 Dockerfile (multi-stage build, model baked in)](https://github.com/ajianaz/uteke/issues/97) `✓ Done`
- [#99 Docker image build & push on release (GHCR)](https://github.com/ajianaz/uteke/issues/99) `✓ Done`

## v0.0.6 — Hardening & Fixes `✓ Done`

- JSON output omits embedding vector (~3KB savings per memory) `✓ Done`
- import() now persists vector index (fixes data loss on restart) `✓ Done`
- Docker: non-root container with USER uteke `✓ Done`
- Dependabot: automated dependency updates `✓ Done`

## v0.0.7 — Core Stability `✓ Done`

- [#120 Tag queries: LIKE → json_each() for exact matching](https://github.com/ajianaz/uteke/issues/120) `✓ Done`
- [#127 Configurable tier thresholds (hot_days, warm_days)](https://github.com/ajianaz/uteke/issues/127) `✓ Done`
- [#129 Test coverage: 34 → 94 tests](https://github.com/ajianaz/uteke/issues/129) `✓ Done`
- Tag substring false positive fix ("rust" ≠ "rustacean") `✓ Done`

## v0.0.8 — Stability & Architecture `✓ Done`

- [#130 Architecture: module split (1471→352 lib.rs)](https://github.com/ajianaz/uteke/issues/130) `✓ Done`
- [#132 Input validation (max 10K chars, 20 tags, 1MB payload)](https://github.com/ajianaz/uteke/issues/132) `✓ Done`
- [#134 Binary checksums (SHA256) + verify-checksums subcommand](https://github.com/ajianaz/uteke/issues/134) `✓ Done`
- [#138 Schema versioning + migration framework](https://github.com/ajianaz/uteke/issues/138) `✓ Done`
- [#144 Error handling rewrite — sanitized user-friendly messages](https://github.com/ajianaz/uteke/issues/144) `✓ Done`
- [#137 Python wrapper: 7→21 methods with type hints](https://github.com/ajianaz/uteke/issues/137) `✓ Done`
- [#49 Memory benchmark binary (library-level timing)](https://github.com/ajianaz/uteke/issues/49) `✓ Done`
- Memory consolidation command `✓ Done`
- Import/Export (JSONL backup and restore) `✓ Done`
- [#174 README overhaul + landing page refresh (GTM Phase 1+2)](https://github.com/ajianaz/uteke/issues/174) `✓ Done`

## v0.0.9 — Website Migration `✓ Done`

- [#194 Website migrated to VitePress](https://github.com/ajianaz/uteke/issues/194) `✓ Done`
  - SvelteKit (3,750 LOC, 10 deps) → VitePress (1,300 LOC markdown, 2 deps)
  - Built-in full-text search
  - Build time: ~15s → ~6s
  - Brand theme (amber/dark) preserved

## v0.0.10 — Codebase Quality `✓ Done` `Current`

- [#187 Split commands.rs into per-command modules](https://github.com/ajianaz/uteke/issues/187) `✓ Done`
- [#186 Split store.rs into focused modules](https://github.com/ajianaz/uteke/issues/186) `✓ Done`
- [#178 Remove all Hermes branding](https://github.com/ajianaz/uteke/issues/178) `✓ Done`
- [#196 Address all Cora code review findings](https://github.com/ajianaz/uteke/issues/196) `✓ Done`
- Safe slice for deprecated IDs `✓ Done`
- Index lock before SQLite write `✓ Done`
- HTTP status checking in server proxy `✓ Done`
- Aging cleanup filter fix `✓ Done`
- Schema migration transactions `✓ Done`
- Batch bulk deletes `✓ Done`
- SQLite-first dual-write `✓ Done`
- Embedding docs corrected (768d) `✓ Done`
- Shell hook idempotency guards `✓ Done`

## Phase A — Growth (100+ stars) `Planned`

Better embeddings, richer integrations, broader reach.

- Better embeddings (larger model option) `Planned`
- [#46 Import from external knowledge sources](https://github.com/ajianaz/uteke/issues/46) `Planned`
- Python SDK (PyO3 bindings) `Planned`
- Editor integrations (VS Code, JetBrains) `Planned`
- Node.js SDK (NAPI) `Planned`

## Phase B — Collaboration (500+ stars) `Future`

Optional cloud sync, team features, and gateway integrations.

- Cloud sync (opt-in, end-to-end encrypted) `Future`
- Team collaboration & shared namespaces `Future`
- API gateway integration (LangChain, CrewAI, others) `Future`
- [#100 TLS support & reverse proxy documentation](https://github.com/ajianaz/uteke/issues/100) `Future`
- [#101 API key authentication for uteke-serve](https://github.com/ajianaz/uteke/issues/101) `Future`

## Phase C — Ecosystem (1000+ stars) `Vision`

Plugin ecosystem, advanced consolidation, community extensions.

- Plugin ecosystem (custom embedding, storage backends) `Vision`
- Advanced consolidation (cross-namespace, summarization) `Vision`
- Community extensions marketplace `Vision`
- Managed cloud API (optional paid tier) `Vision`
