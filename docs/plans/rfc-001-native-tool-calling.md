# RFC 001: Native Tool-Calling System

> **Status:** Draft | **Author:** CTO | **Date:** 2026-07-07
> **Replaces:** iii engine adoption (rejected due to ELv2 license risk)

## Summary

Add native tool-calling capabilities to Uteke — function registry, cross-agent invocation, and declarative triggers. This transforms Uteke from a **semantic memory engine** into a **memory + coordination engine**, enabling any AI agent (Claude Code, OpenCode, Cursor, Hermes) to register functions and invoke them across agents without external orchestration runtime.

**Why:** The iii engine (github.com/iii-hq/iii) provides similar capabilities but is licensed under Elastic License 2.0 (ELv2) — a proprietary, non-open-source license with managed service restrictions that overlap with our product goals. Building natively in Uteke avoids all license risk while leveraging existing infrastructure (SQLite, HTTP server, embedding pipeline).

---

## Background & Motivation

### Problem

Currently, cross-agent coordination is manual:
- Functions are **hardcoded per agent** — no discovery mechanism
- Agent A calling Agent B requires **manual webhook + coordination.py** 
- No **declarative triggers** — event → action must be implemented per-case via cron
- No **live discovery** — new workers are invisible until manually configured

### What We Evaluated

**iii engine (github.com/iii-hq/iii):** General-purpose orchestration runtime with Worker/Function/Trigger primitives, 4 SDK languages, WebSocket protocol, built-in queue/cron/pubsub/observability.

| Criterion | iii | Verdict |
|-----------|-----|---------|
| Feature completeness | ✅ Full (registry, invocation, triggers, discovery, observability) | Excellent |
| License | ❌ ELv2 (engine), Apache 2.0 (SDK) | **Blocked** |
| Runtime dependency | ⚠️ Separate binary, 24/7 required | Unacceptable |
| Semantic memory | ❌ iii-state is KV-only | Still need Uteke |
| Complexity | ⚠️ WebSocket management, worker lifecycle | Over-engineered for our fleet |

### Why ELv2 Is a Deal-Breaker

| Activity | ELv2 Status | Impact |
|-----------|-------------|--------|
| Run binary internally | ✅ Allowed | Safe |
| Behind our product (customers don't see iii) | ⚠️ **Grey zone** | "Substantial set of features" clause |
| Expose to third parties | ❌ **Prohibited** | Managed service ban |
| Non-transferable, non-sublicensable | ❌ | More restrictive than BSL/SSPL |

Our products (Uteke, Hermes, CodeCora) **overlap** with iii's capabilities. Using iii behind our product creates an untested legal grey zone. Building native eliminates this entirely.

### Decision

> **Build native tool-calling in Uteke.** iii serves as architectural reference (Worker/Function/Trigger pattern), not a dependency.

---

## Proposed Architecture

### Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    Uteke Engine (existing)                  │
│  SQLite · usearch · EmbeddingGemma · FTS5 · Graph · Rooms   │
└────────────────────────┬────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│              Uteke Tool-Calling (NEW modules)                │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │   Function   │  │  Invocation  │  │   Trigger    │      │
│  │   Registry   │  │   Router     │  │   Engine     │      │
│  │              │  │              │  │              │      │
│  │ SQLite table │  │ HTTP POST    │  │ TOML config  │      │
│  │ CRUD via API │  │ → worker     │  │ Event → fn   │      │
│  │ Auto-expire  │  │ sync/async   │  │ on_remember  │      │
│  └──────────────┘  └──────────────┘  │ on_schedule  │      │
│                                     │ on_tag_match │      │
│                                     └──────────────┘      │
└─────────────────────────────────────────────────────────────┘
                         │
              HTTP REST (port 8767)
                         │
    ┌────────────────────┼────────────────────┐
    │                    │                    │
┌───▼───┐          ┌─────▼─────┐        ┌─────▼─────┐
│Worker │          │   Worker  │        │   Worker  │
│Hermes │          │Claude Code│        │  OpenCode │
│(MCP)  │          │  (MCP)    │        │  (MCP)    │
└───────┘          └───────────┘        └───────────┘
```

### Design Principles

1. **HTTP REST only** — No WebSocket. Simpler, curl-able, language-agnostic, no persistent connection management.
2. **SQLite-backed** — Workers, functions, invocations, triggers all in SQLite. Zero external state.
3. **MCP as primary interface** — Tools exposed via existing uteke-mcp (27 tools → add 4 more).
4. **Namespace isolation** — Functions scoped per namespace, matching existing memory model.
5. **Backward compatible** — All existing endpoints unchanged. New endpoints under `/workers/`, `/invoke`, `/triggers/`.

---

## Detailed Design

### Module 1: Function Registry

Workers (any process) register their callable functions with Uteke. Functions are identified by **stable IDs** in `namespace::name` format.

#### SQLite Schema

```sql
CREATE TABLE IF NOT EXISTS workers (
    id          TEXT PRIMARY KEY,          -- e.g. "hermes-cto", "claude-code"
    name        TEXT NOT NULL,
    endpoint    TEXT NOT NULL,             -- HTTP callback URL (e.g. "http://localhost:9001/webhook")
    namespace   TEXT,                     -- optional namespace scoping
    status      TEXT NOT NULL DEFAULT 'active',  -- active | inactive | expired
    metadata    TEXT,                     -- JSON: version, capabilities, etc.
    heartbeat_at INTEGER,                  -- last heartbeat timestamp
    registered_at INTEGER NOT NULL,
    expires_at  INTEGER                   -- auto-expire (NULL = never)
);

CREATE TABLE IF NOT EXISTS functions (
    id          TEXT PRIMARY KEY,          -- e.g. "cto::review", "hermes::deploy"
    worker_id   TEXT NOT NULL REFERENCES workers(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT,                     -- human-readable description for discovery
    input_schema  TEXT,                    -- optional JSON Schema for request validation
    output_schema TEXT,                    -- optional JSON Schema for response
    tags        TEXT,                      -- JSON array for categorization
    status      TEXT NOT NULL DEFAULT 'active',
    registered_at INTEGER NOT NULL
);

CREATE INDEX idx_functions_worker ON functions(worker_id);
CREATE INDEX idx_functions_tags ON functions(tags);
```

#### HTTP Endpoints

```
POST /workers/register
  Body: { "id": "hermes-cto", "name": "CTO Agent", "endpoint": "http://localhost:9001/webhook",
          "namespace": "cto", "functions": [
            { "id": "cto::review", "description": "Review code changes",
              "input_schema": { "type": "object", "properties": { "pr_url": { "type": "string" } } } },
            { "id": "cto::deploy", "description": "Deploy to staging" }
          ]}
  Response: { "status": "ok", "worker_id": "hermes-cto", "functions_registered": 2 }

POST /workers/heartbeat
  Body: { "id": "hermes-cto" }
  Response: { "status": "ok", "next_heartbeat_before": 300 }

GET /workers/list?namespace=cto&status=active
  Response: { "workers": [{ "id": "hermes-cto", "name": "CTO Agent", "functions": [...] }] }

GET /workers/functions?namespace=cto&tags=deploy
  Response: { "functions": [{ "id": "cto::review", "worker_id": "hermes-cto", ... }] }

DELETE /workers/unregister
  Body: { "id": "hermes-cto" }
  Response: { "status": "ok", "functions_removed": 2 }
```

#### Worker Lifecycle

```
Register → Active (heartbeat every 5 min)
         → Expired (no heartbeat for 15 min → auto-expire)
         → Inactive (manual unregister)
```

Auto-expire prevents stale functions from being invoked (worker crashed, process died).

### Module 2: Invocation Router

Routes function calls from any agent to the appropriate worker endpoint.

#### HTTP Endpoints

```
POST /invoke
  Body: { 
    "function": "cto::review",
    "payload": { "pr_url": "https://github.com/codecoradev/cora-api/pull/42" },
    "mode": "sync",           // sync | async | enqueued
    "caller": "claude-code",   // optional, for audit
    "timeout": 30000          // ms, optional (default 30s)
  }
  
  // sync mode: wait for worker response
  Response (200): { "status": "success", "result": { ... }, "duration_ms": 450 }
  Response (404): { "status": "error", "error": "Function cto::review not found" }
  Response (504): { "status": "error", "error": "Worker timeout after 30000ms" }
  
  // async mode: return immediately with invocation ID
  Response (202): { "status": "accepted", "invocation_id": "inv_abc123" }

GET /invoke/status?id=inv_abc123
  Response: { "status": "completed", "result": { ... } } | { "status": "pending" } | { "status": "failed", "error": "..." }
```

#### Invocation Flow (Sync)

```
1. Agent → POST /invoke { function: "cto::review", payload: {...} }
2. Router → lookup function → find worker endpoint
3. Router → HTTP POST to worker endpoint
   { "invocation_id": "inv_abc123", "function": "cto::review", 
     "payload": {...}, "caller": "claude-code" }
4. Worker → processes request → HTTP response
5. Router → stores invocation log → returns result to caller
```

#### Invocation Log (SQLite)

```sql
CREATE TABLE IF NOT EXISTS invocations (
    id          TEXT PRIMARY KEY,
    function_id TEXT NOT NULL REFERENCES functions(id),
    caller      TEXT,                     -- who invoked
    payload     TEXT,                     -- request payload (JSON)
    result      TEXT,                     -- response (JSON), NULL if pending/failed
    status      TEXT NOT NULL,            -- pending | completed | failed | timeout
    mode        TEXT NOT NULL,            -- sync | async | enqueued
    created_at  INTEGER NOT NULL,
    completed_at INTEGER,
    duration_ms INTEGER                   -- NULL for async still pending
);

CREATE INDEX idx_invocations_function ON invocations(function_id);
CREATE INDEX idx_invocations_status ON invocations(status);
```

### Module 3: Trigger Engine

Declarative event-to-function mapping. No code required — configuration-driven.

#### Config Format (`triggers.toml`)

```toml
[triggers]

# Fire when a memory is stored matching tag pattern
[triggers.on_remember_deploy]
event = "on_remember"
tag_pattern = "deploy:*"
function = "clo::notify_deploy"
namespace = "cto"

# Fire on schedule (cron syntax)
[triggers.daily_health_check]
event = "on_schedule"
cron = "0 9 * * *"
function = "ops::health_check"
namespace = "cto"

# Fire when specific function completes
[triggers.post_review_summary]
event = "on_invocation_complete"
function_pattern = "cto::review"
target_function = "summarizer::create_summary"
namespace = "cto"

# Fire when worker registers
[triggers.on_worker_join]
event = "on_worker_register"
target_function = "ops::log_fleet_change"
namespace = "cto"
```

#### Supported Events

| Event | Trigger | Payload |
|-------|---------|---------|
| `on_remember` | Memory stored with matching tag pattern | `{ memory_id, content, tags, namespace }` |
| `on_forget` | Memory removed | `{ memory_id, namespace }` |
| `on_worker_register` | New worker registered | `{ worker_id, functions[] }` |
| `on_worker_expire` | Worker heartbeat expired | `{ worker_id }` |
| `on_invocation_complete` | Function call finished | `{ invocation_id, function_id, status, result }` |
| `on_schedule` | Cron-based | `{ trigger_time }` |
| `on_tag_match` | Memory stored with exact tag | `{ memory_id, tag, content }` |

#### Trigger Execution

Triggers invoke the target function via the **Invocation Router** — no special code path. This means:
- Triggers get the same logging, timeout, retry semantics as manual invocations
- Target function can be on ANY worker (not just local)
- Async by default (fire-and-forget) to not block the triggering event

### MCP Integration (Phase 3)

Add 4 new tools to existing `uteke-mcp`:

| Tool | Description |
|------|-------------|
| `uteke_register` | Register current worker + functions |
| `uteke_invoke` | Call a registered function by ID |
| `uteke_list_functions` | Discover available functions (filter by namespace/tags) |
| `uteke_trigger_list` | List configured triggers |

Example MCP usage from Claude Code:
```
Tool: uteke_invoke
{ "function": "cto::review", "payload": { "pr_url": "https://github.com/..." } }
```

---

## Implementation Plan

### Phase 1: Function Registry + Invocation (Core)
**Effort:** 5-7 days | **Branch:** `feat/tool-calling-p1`

| Task | Files | Description |
|------|-------|-------------|
| Schema migration | `uteke-core/src/memory/schema.rs` | Add `workers`, `functions`, `invocations` tables |
| Registry types | `uteke-core/src/types.rs` | `Worker`, `Function`, `RegistrationRequest` structs |
| Registry CRUD | `uteke-core/src/registry.rs` (new) | Register, unregister, heartbeat, list, auto-expire |
| Server endpoints | `uteke-server/src/handlers.rs` | 6 new endpoints under `/workers/*` |
| Invocation router | `uteke-core/src/invocation.rs` (new) | Route + HTTP POST to worker, log result |
| Server endpoints | `uteke-server/src/handlers.rs` | 2 new endpoints: `POST /invoke`, `GET /invoke/status` |
| Types | `uteke-server/src/types.rs` | Request/response structs for new endpoints |
| Tests | `tests/` | Integration tests for register → invoke → log |

### Phase 2: Trigger Engine
**Effort:** 5-7 days | **Branch:** `feat/tool-calling-p2`

| Task | Files | Description |
|------|-------|-------------|
| Config parser | `uteke-core/src/triggers.rs` (new) | Parse `triggers.toml` → trigger rules |
| Trigger table | `uteke-core/src/memory/schema.rs` | `triggers` table for persistence |
| Event hooks | `uteke-core/src/operations.rs` | Emit events from remember/forget/invocation |
| Trigger executor | `uteke-core/src/triggers.rs` | Match event → rule → invoke target function |
| Cron scheduler | `uteke-core/src/triggers.rs` | Simple cron evaluator (no dep) for `on_schedule` |
| Server endpoint | `uteke-server/src/handlers.rs` | `GET /triggers/list`, `POST /triggers/reload` |
| Tests | `tests/` | Event → trigger → invocation chain |

### Phase 3: MCP Tools
**Effort:** 3-5 days | **Branch:** `feat/tool-calling-p3`

| Task | Files | Description |
|------|-------|-------------|
| MCP tools | `uteke-mcp/src/tools.rs` | 4 new tools: register, invoke, list_functions, trigger_list |
| Server integration | `uteke-server/src/handlers.rs` | Ensure `/mcp` routes new tool calls |

### Phase 4: Hermes Integration
**Effort:** 3-5 days | **Branch:** `feat/tool-calling-p4`

| Task | Description |
|------|-------------|
| Hermes webhook handler | Receive invocations, route to agent |
| Agent registration on startup | Auto-register functions when Hermes gateway starts |
| Cross-agent invocation skill | Skill for agents to discover and call each other |
| Tool-calling skill | Reusable skill pattern for registering/invoking |

### Total Estimate

| Phase | Days | Cumulative |
|-------|------|------------|
| P1: Registry + Invoke | 5-7 | 5-7 |
| P2: Triggers | 5-7 | 10-14 |
| P3: MCP Tools | 3-5 | 13-19 |
| P4: Hermes Integration | 3-5 | **16-24 (~3-4 weeks)** |

---

## Cross-Tool Compatibility

Because everything is HTTP REST + MCP, any tool can participate:

| Tool | Connection Method | Example |
|------|------------------|---------|
| **Claude Code** | MCP (uteke-mcp) | `uteke_invoke("cto::review", payload)` |
| **OpenCode** | MCP or curl | Same interface |
| **Cursor** | MCP | Same interface |
| **Pi** | MCP | Same interface |
| **Hermes agents** | MCP + webhook | Auto-register on startup |
| **Cora (Rust)** | HTTP directly | `POST /invoke` — same binary, zero overhead |
| **Custom scripts** | curl / HTTP client | Any language, any runtime |

---

## Trade-offs

### Chosen: HTTP REST

| Pro | Con |
|-----|-----|
| ✅ Zero persistent connection management | ⚠️ Higher per-call latency (~5-10ms overhead) |
| ✅ curl-able, trivial debugging | ⚠️ No push notifications (must poll for async) |
| ✅ Language-agnostic, no SDK needed | |
| ✅ Works through proxies, firewalls | |
| ✅ Matches existing uteke-serve pattern | |

### Chosen: SQLite for all state

| Pro | Con |
|-----|-----|
| ✅ Zero external dependency | ⚠️ Single-writer (Mutex lock serializes requests) |
| ✅ ACID, crash-safe | ⚠️ Won't scale to 1000s concurrent invocations |
| ✅ Matches existing Uteke storage pattern | |

**Assessment:** Our fleet is ~6-10 agents. SQLite single-writer is fine. If we need horizontal scaling later, the HTTP REST layer makes it trivial to swap SQLite for PostgreSQL behind the same API.

### Deferred: WebSocket

WebSocket (like iii uses) enables push notifications, bidirectional streaming, and lower per-message overhead. But adds significant complexity:
- Connection lifecycle management (reconnect, backoff, heartbeat)
- Worker discovery protocol
- Message framing and ordering
- No benefit for our call frequency (~10-100 invocations/hour)

**Verdict:** YAGNI. Add WebSocket in v2 if needed.

---

## Security Considerations

1. **Worker endpoint validation:** Only invoke functions whose worker has sent a recent heartbeat (within 15 min). Prevents invoking dead/stale workers.
2. **Input validation:** Optional JSON Schema validation on invocation payload (if function defines `input_schema`).
3. **Namespace isolation:** Functions are namespace-scoped. Agent A cannot invoke functions in Agent B's namespace without explicit access.
4. **Payload size limit:** Same 1MB cap as existing endpoints (`MAX_PAYLOAD_SIZE`).
5. **No authentication bypass:** All new endpoints use existing auth middleware (token-based, same as `/remember`, `/recall`).

---

## Open Questions

1. **Heartbeat interval:** 5 min default? Configurable per worker? Shorter for critical workers?
2. **Retry policy:** Should invocation router retry on worker timeout? How many times? Exponential backoff?
3. **Async result storage:** How long to keep async invocation results? TTL-based cleanup?
4. **Trigger ordering:** If multiple triggers match the same event, execute in parallel or sequentially?
5. **Cron precision:** Simple cron parser (minute granularity) or use an existing Rust crate (`cron`)? Decision: use `cron` crate — battle-tested, ~50KB.

---

## Appendix A: Feature Comparison vs iii

| Feature | iii | Uteke Tool-Calling (proposed) | Gap |
|---------|-----|-------------------------------|-----|
| Function registry | ✅ Live catalog | ✅ SQLite + HTTP | Discovery latency (iii = instant via WS, Uteke = query) |
| Cross-agent invocation | ✅ Via engine routing | ✅ Via HTTP POST to worker | iii has WS, we use HTTP |
| Declarative triggers | ✅ 8 trigger types | ✅ 7 trigger types | iii has `stream` trigger, we defer |
| Live discovery | ✅ Instant via WS | ⚠️ On-demand query | Workers must register first |
| Queue management | ✅ Built-in | ❌ Deferred to v2 | Not needed for current fleet |
| Pub/sub | ✅ Built-in | ❌ Not planned | Not needed for current fleet |
| Streaming channels | ✅ Built-in | ❌ Not planned | Not needed for current fleet |
| Observability (OTEL) | ✅ Built-in | ⚠️ Invocation log table | Sufficient for debugging |
| SDK languages | ✅ TS/Python/Rust/Go | ✅ HTTP REST (any language) | No SDK needed |
| Semantic memory | ❌ (iii-state = KV) | ✅ Full Uteke stack | **Uteke wins** |
| License | ❌ ELv2 (engine) | ✅ Apache 2.0 | **Uteke wins** |
| External dependency | ❌ iii engine required | ✅ Zero | **Uteke wins** |

## Appendix B: Rejected Alternatives

### Alt 1: Adopt iii as-is
**Rejected:** ELv2 license risk. Grey zone for our product overlap.

### Alt 2: Adopt iii SDK only (Apache 2.0), connect to iii engine
**Rejected:** Still requires iii engine binary running 24/7. SDK is Apache 2.0 but engine is ELv2. Still adds runtime dependency.

### Alt 3: Use gRPC instead of HTTP REST
**Rejected:** Requires protobuf definitions, code generation, heavier tooling. HTTP REST is sufficient for our call patterns.

### Alt 4: Use MQTT for worker communication
**Rejected:** Adds MQTT broker dependency. Overkill for request-response pattern. Hermes already has MQTT for other purposes — keep concerns separate.

### Alt 5: Build standalone orchestration service (separate from Uteke)
**Rejected:** Uteke already has SQLite, HTTP server, embedding, rooms. Building a separate service duplicates infrastructure for no benefit.

---

## Changelog

| Date | Change |
|------|--------|
| 2026-07-07 | Initial draft — decision, architecture, phases |
