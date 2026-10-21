# Scope-isolation suite

A retrieval surface must never return a record from outside the scope it was asked for (#1467, after the namespace orphans of #1353 and the unified-recall leak of #1270). The suite is a regression gate: it runs in `cargo test`, so a failure fails the `Test` check of a pull request.

## The fixture

`uteke_core::test_support::seed_two_scopes()` (behind the hidden `test-support` feature of `uteke-core`, used as a dev-dependency only) builds one store:

- two namespaces, `alpha` and `beta`, whose memories share the **same vocabulary** (`quartz meridian orchid ledger`), so every surface has hits in both scopes for one query;
- three plain memories per namespace tagged `project:<ns>` and `shared`, and one memory per namespace in room `room-<ns>`;
- every memory ends in `of <namespace>`, so a response can be checked by text, whatever its shape;
- one global document, `shared-doc`, with the same vocabulary (documents have no namespace by design, #1271).

A check passes only if the response scoped to one namespace holds no `of <other>` record and no `project:<other>` tag. Where hits are expected it first asserts the scope's own marker is present, so a check cannot pass by returning nothing.

## What is covered

| Transport | Where | Surfaces |
|---|---|---|
| Core | `crates/uteke-core/src/isolation_tests.rs` | `recall` and `recall_hybrid` (all five strategies), tag filters, unified recall (`All`, `Memory`), `search`, `list`, tag counts, `export`, `count`, scoped room recall, room listing, `graph_data`, `graph_view` |
| HTTP | `crates/uteke-server/src/handlers/isolation_tests.rs` | `/recall` (five strategies × `memory`/`all`, `pack`, `explain`, tags), `/search`, `/list`, `/recent`, `/export`, `/graph`, `/room/memories`, `/room/recall`, `/orphans`, `/tags`, `/room/list`, `/stats`, `/doc/search`, `/doc/list` |
| MCP | `crates/uteke-mcp/src/lib.rs`, `mod isolation_tests` | `uteke_recall` (five strategies × `memory`/`all`, `pack`, `explain`, tags), `uteke_search`, `uteke_list`, `uteke_graph`, `uteke_room_recall`, `uteke_room_memories`, `uteke_tags_list`, `uteke_room_list`, `uteke_stats`, `uteke_contradictions` |
| CLI | `crates/uteke-cli/tests/scope_isolation.rs` | `search`, `list` (with a tag), `export`, `room recall`, `orphans`, `tags list`, `stats`, `room list`, `contradictions list` |

`uteke recall` is not in the CLI suite: it needs the ONNX runtime, which CI does not have. The same code path is covered by the core, HTTP and MCP suites.

## Exemptions

These read surfaces take no scope, and each is listed with its reason in the suite instead of being skipped silently:

- **Fetch by ID** (`GET /memory`, `/provenance`, `/timeline`, `/edges`, `POST /memory/doc-refs`, `uteke_get`, `uteke_provenance`, `uteke_timeline`): IDs are global handles.
- **By room or slug** (`/room/summary`, `/room/stats`, `/doc/get`, `uteke_room_stats`, ...): the room or the document is the scope; documents are global.
- **No records**: `/health`, `/guide`, `/routes`, `/store`, `/namespaces`.
- **`SearchType::Document`**: the global document view; it returns documents only, never a memory, and `All` and `Memory` never return a document when scoped (pinned in the core suite).

## Adding a retrieval surface

1. Add it to `surfaces()` in the suites of the transports that expose it. A surface that exists in one transport must be covered in all of them.
2. If it truly takes no scope, add it to `EXEMPT` with the reason.

Two tests enforce this: the HTTP suite fails when a read row (a `GET`, or a `POST` that does not write) of `api_registry::ENDPOINTS` is neither covered nor exempt, and the MCP suite fails when a tool of `READ_ONLY_TOOLS` is neither covered nor exempt, or when a tool that advertises a `namespace` argument is not covered.

## Requiring it

The suite runs in the `Test` job (`cargo test --workspace`). `Test` is a required status check of the `develop-protected` and `main-protected` rulesets (checked 2026-10-21), so a failing isolation test blocks the merge of a pull request. If a ruleset is ever rebuilt, keep `Test` in its required checks: the gate is only as strong as that setting.
