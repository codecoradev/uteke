# Development Roadmap

## Phase 0: Validate (1-2 minggu)

**Status:** Not Started **Goal:** Validate demand dan technical feasibility

### Tasks

- [ ] Interview 10-20 target users (Cursor/Claude Code users)
- [ ] Confirm pain points match assumptions
- [ ] Prototype minimal HNSW + SQLite in Rust (proof of concept)
- [ ] Benchmark: can we hit <5ms retrieval with 10K vectors?
- [ ] Check domain availability (uteke.dev, uteke.ai)
- [ ] Socialize concept on HN/Reddit, gauge interest
- [ ] Decision: Go/No-Go

### Success Criteria

* 8/10 interviewees confirm "context loss" as top pain point
* PoC retrieval < 10ms for 10K vectors
* 100+ upvotes on HN Show post


---

## Phase 1: MVP (4-6 minggu)

**Status:** Not Started **Goal:** Usable CLI tool with memory + git sync

### Core Features

- [ ] Memory Engine: Vector (HNSW) + Structured (SQLite)
- [ ] Context Manager: Basic retrieval + ranking
- [ ] CLI tool: `uteke remember`, `uteke recall`, `uteke context`
- [ ] Git sync: Auto .uteke/ in repo, push/pull
- [ ] Embedding: Support local model (embeddings-fast-rs)
- [ ] Basic config: .uteke/config.toml

### Integration

- [ ] VS Code extension: Basic sidebar + command palette
- [ ] Cursor compatibility (same extension)

### Testing

- [ ] Unit tests: >80% coverage on core modules
- [ ] Integration test: CLI → SQLite → HNSW → recall
- [ ] Performance benchmark: 10K vectors < 5ms
- [ ] Cross-platform test: Linux + macOS

### Deliverable

* `uteke` CLI binary (2-5MB)
* VS Code extension (marketplace draft)
* README + quickstart guide
* GitHub repo public


---

## Phase 2: IDE + SDK (3-4 minggu)

**Status:** Not Started **Goal:** Production-ready IDE extension + multi-language SDK

### IDE

- [ ] VS Code extension v1.0: Auto-capture, inline recall, context panel
- [ ] Cursor extension (same codebase)
- [ ] JetBrains plugin (proof of concept)

### SDK

- [ ] Python SDK (PyO3 bindings)
- [ ] Node.js SDK (napi-rs)
- [ ] Go SDK (CGo)

### Features

- [ ] Graph memory (entity relationships)
- [ ] Temporal memory (time-indexed events)
- [ ] Dedup engine
- [ ] Context compression

### Deliverable

* VS Code extension v1.0 (published)
* Python + Node SDK on PyPI/npm
* API documentation


---

## Phase 3: Team + Server (4-6 minggu)

**Status:** Not Started **Goal:** Self-hosted sync server + team features

### Sync Server

- [ ] Rust sync server binary
- [ ] WebSocket real-time sync
- [ ] CRDT conflict resolution
- [ ] Docker one-liner deployment
- [ ] Auth (API key / OAuth)

### Team Features

- [ ] Multi-user support
- [ ] Shared memory spaces
- [ ] Permission system (read/write/admin)
- [ ] Audit log

### Deliverable

* Docker image on GHCR
* Self-hosted setup guide
* Team onboarding docs


---

## Phase 4: Cloud + Scale (6-8 minggu)

**Status:** Not Started **Goal:** Managed cloud service + enterprise features

### Cloud

- [ ] Managed sync service (multi-region)
- [ ] Web dashboard
- [ ] Billing (Stripe integration)
- [ ] Free tier: 1 user, 3 devices, 10K memories
- [ ] Pro tier: unlimited users, 1M memories, priority support

### Enterprise

- [ ] SSO (SAML/OIDC)
- [ ] Compliance (SOC2, GDPR)
- [ ] On-premise deployment
- [ ] Custom embedding models
- [ ] SLA guarantee

### Deliverable

* uteke.cloud (managed service)
* Pricing page
* Enterprise sales collateral


---

## Timeline Summary

| Phase | Duration | Milestone |
|-------|----------|-----------|
| Phase 0: Validate | 1-2 minggu | Go/No-Go decision |
| Phase 1: MVP | 4-6 minggu | Public CLI + VS Code extension |
| Phase 2: IDE + SDK | 3-4 minggu | Published extension + SDK |
| Phase 3: Team | 4-6 minggu | Self-hosted sync server |
| Phase 4: Cloud | 6-8 minggu | Managed cloud service |
| **Total** | **\~5-7 bulan** | **Full product** |