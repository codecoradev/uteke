# HTTP API Reference

Auto-generated from `uteke-server` route registry and type schemas. Do not edit manually — run `cargo run -p docgen` to regenerate.

**Base URL**: `http://localhost:8767` (default)

**Auth**: Set `--auth-token <TOKEN>` to require `Authorization: Bearer <TOKEN>` header.

## 🏷️ Tags

#### 🟢 `GET` `/tags`

List all tags in a namespace. Accepts `?namespace=X` query param.

#### 🟡 `POST` `/tags/rename`

Rename a tag across all memories.

**Request body**: [`TagRenameRequest`](#tagrenamerequest)

#### 🟡 `POST` `/tags/delete`

Delete a tag from all memories.

**Request body**: [`TagDeleteRequest`](#tagdeleterequest)


## 📌 Pin (Legacy)

#### 🟡 `POST` `/pin`

Pin a memory by ID (legacy — prefer /memory/pin).

**Request body**: [`PinRequest`](#pinrequest)

#### 🟡 `POST` `/unpin`

Unpin a memory by ID (legacy — prefer /memory/pin with pin=false).

**Request body**: [`PinRequest`](#pinrequest)


## 📝 Other

#### 🟢 `GET` `/guide`

Returns the agent-facing memory tools guide for system prompt injection (#1010).

**Response**: [`GuideResponse`](#guideresponse)

*Related: `#1010`*

#### 🟢 `GET` `/store`

Identity of the store this server serves: {"id": "<16 hex digits>"}, the SHA-256 prefix of the store's canonical path (`uteke_core::store_identity`). The CLI compares it with the store it resolved before routing a command here, so `UTEKE_HOME` pointing elsewhere is not sent to this server (#1445). The path itself is never returned. Needs the usual token when auth is on.

*Related: `#1445`*

#### 🟢 `GET` `/namespaces`

List all namespaces in the memory store. `?with_counts=true` adds `count` (total) plus `active`/`deprecated` breakdown fields (#1181).

*Related: `1181`*

#### 🟡 `POST` `/namespaces/rename`

Rename a namespace (`{from, to}`). When `to` already exists this is a merge; the old name vanishes (derived view). Returns `{from, to, moved, target_existed}` (#1181).

**Request body**: [`NamespaceRenameRequest`](#namespacerenamerequest)

*Related: `1181`*

#### 🟡 `POST` `/namespaces/delete`

Delete a namespace with an explicit strategy for its memories (`{name, strategy, target?}`): `refuse` (default — 409 while any memory references the name), `merge` (move all memories to `target`), or `deprecate` (soft-delete — restorable, never hard-deleted) (#1181).

**Request body**: [`NamespaceDeleteRequest`](#namespacedeleterequest)

*Related: `1181`*

#### 🟢 `GET` `/stats`

Get memory statistics (count, etc.) for a namespace. Accepts `?namespace=X` query param.

#### 🟡 `POST` `/stats`

Get memory statistics via POST body. Accepts `{"namespace": "..."}`.

**Request body**: JSON object (see handler source for fields)

*Related: `#786`*

#### 🟢 `GET` `/memory`

Get a single memory by ID. Accepts `?id=...` query param.

**Response**: [`Memory`](#memory)

#### 🔵 `PUT` `/memory`

Update an existing memory's content and/or metadata.

**Request body**: [`MemoryUpdateRequest`](#memoryupdaterequest)

#### 🟡 `POST` `/verify`

Check vector-index/SQLite consistency: row vs vector counts, mismatch flag (#1266).

**Response**: [`VerifyReport`](#verifyreport)

#### 🟡 `POST` `/repair`

Rebuild the vector index from stored embeddings to fix recall desync (#1266). SQLite data is untouched.

**Response**: [`RepairReport`](#repairreport)

#### 🟡 `POST` `/skill/search`

Search the personal skill library (#1731): the skills whose indexed files match `query` (at most 2000 characters), best first, one hit per skill with the file and snippet that matched. `mode` is `hybrid` (default), `semantic` or `fts`; `limit` is clamped to 1 to 50 (default 5); `approved_only` keeps only skills whose current version is approved. Hybrid falls back to keywords without the embedding model, `semantic` needs it (503 without); `hybrid` and `semantic` embed the query, so 429 when two embedding requests already run (`fts` never does). Skill files are not documents: they never appear in `/doc/*`. Nothing is written. Read-only.

**Request body**: [`SkillSearchRequest`](#skillsearchrequest)

*Related: `1731`*

#### 🟡 `POST` `/skill/list`

List the skills of the personal library, by slug, with their current version (status, hash, validity, owner; never the server path it was indexed from). `status` keeps `draft` or `approved` skills; `limit` is clamped to 1 to 200 (default 50) and `total` counts the skills that match before the limit. Nothing is written. Read-only.

**Request body**: [`SkillListRequest`](#skilllistrequest)

*Related: `1731`*

#### 🟡 `POST` `/skill/show`

One skill: its current version (hash, status, validity and lint issues, file manifest), the hash a client may load (`approved_hash`), usage counters, the last 50 versions and the last 20 ledger rows. `slug` is at most 64 characters; 404 for an unknown skill. Nothing is written. Read-only.

**Request body**: [`SkillShowRequest`](#skillshowrequest)

*Related: `1731`*

#### 🟡 `POST` `/skill/similar`

Skills close to a skill (`slug`) or to a draft `text` that is not stored (at most 32768 characters; needs the embedding model, 503 without; 429 when two embedding requests already run). Compares the `SKILL.md` document vectors: `possible_duplicate` from 0.92, `similar_to` from 0.80, `threshold` overrides the floor, `limit` is clamped to 1 to 100 (default 10). Nothing is written. Read-only.

**Request body**: [`SkillSimilarRequest`](#skillsimilarrequest)

*Related: `1731`*

#### 🟢 `GET` `/graph`

Memory graph for visualisation: live memories as nodes, `memory_edges` (auto-links, supersession) plus explicit graph edges as edges, keyed by memory id. Accepts `?namespace=...`, `?node_id=<memory id>` (that memory's neighbourhood) and `?limit=` (edges, default 500, max 5000); response carries `truncated`. Derived `referenced_by` backlinks are omitted.

#### 🟢 `GET` `/provenance`

Full provenance report for a memory (#1172): author/source fields, trust tier, source hash at write vs live-recomputed content hash (tamper evidence), and the full timeline event chain with actor + evidence. Accepts `?id=...` query param.

*Related: `1172`*

#### 🟢 `GET` `/contradictions`

List superseded-but-not-restored memories (#1172) — the auditable contradiction resolution ledger (deprecated rows carrying a live superseded_by edge). Accepts `?namespace=...&limit=...`.

*Related: `1172`*

#### 🟡 `POST` `/contradictions/undo`

Undo a contradiction resolution (#1172): restore the retired memory to active, remove the supersession pair, and record supersession_undone provenance events. Body: `{id}`.

**Request body**: [`ContradictionUndoRequest`](#contradictionundorequest)

*Related: `1172`*

#### 🟡 `POST` `/capture`

Queue one raw conversation turn for deferred, budgeted fact extraction. Refused (409) unless [capture] enabled is on.

**Request body**: [`CaptureRequest`](#capturerequest)

*Related: `#1466`*

#### 🟢 `GET` `/capture/status`

Capture queue counts: pending, extracting, done, failed turns and pending sessions.

*Related: `#1466`*

#### 🟡 `POST` `/lifecycle/cycle`

Run lifecycle aging cycle: deprecate old memories, optionally prune expired ones.

*Related: `#935`*

#### 🟡 `POST` `/lifecycle/promote`

Restore a deprecated memory back to active status.

*Related: `#935`*

#### 🟢 `GET` `/lifecycle/status`

Get lifecycle status: active/deprecated counts and current configuration.

*Related: `#935`*

#### 🟢 `GET` `/lifecycle/deprecated`

List deprecated memories with TTL metadata.

*Related: `#1007`*


## 📦 Import/Export

#### 🟡 `POST` `/import`

Import memories from JSONL (or a JSON array) in `content`. Each row keeps its `id`, type, importance, pinned flag and slug; the response counts `imported`, `skipped`, `duplicates_skipped`, `id_conflicts_skipped` and `soft_deleted_skipped`. A Uteke Cloud export imports whole: `type: "document"` rows become documents (`parent` by slug), `type: "room"` rows become rooms with their `doc_slugs` linked, both before the memories' `rooms`, and the manifest line is skipped; the response then adds `documents_imported`, `documents_updated`, `documents_unchanged`, `parent_unresolved`, `rooms_imported`, `rooms_existing`, `room_documents_linked` and `room_documents_missing` (each only when non-zero). The body limit of this route is 64 MiB (`UTEKE_MAX_IMPORT_SIZE`, bytes; other routes keep 10 MB); a larger body gets a 413 that names the limit. A live row whose content matches only a deprecated copy (forgotten, superseded, consolidated or pruned) is imported as a new live memory; send `skip_deprecated_copies: true` to skip such rows (counted in `duplicates_skipped`; #1716). Bulk runbook: send `defer_embedding: true` (rows are stored without a vector and the call returns in seconds; split a large file into several calls under the body limit), then run `uteke repair --reembed` (add `--limit N` to spread the work) so `/recall` finds them.

**Request body**: [`ImportRequest`](#importrequest)

#### 🟢 `GET` `/export`

Export all memories as JSON. Accepts `?namespace=...` query param.

#### 🟡 `POST` `/importance`

Recompute importance scores for all memories.

**Request body**: [`ImportanceRequest`](#importancerequest)


## 🔧 Maintenance

#### 🟡 `POST` `/prune`

Remove orphaned memories (no room, no graph edges).

**Request body**: [`PruneRequest`](#prunerequest)

#### 🟡 `POST` `/consolidate`

Merge similar/duplicate memories automatically.

**Request body**: [`ConsolidateRequest`](#consolidaterequest)

#### 🟡 `POST` `/consolidate/pair`

Consolidate a single caller-chosen duplicate pair: keep id_keep, deprecate (or hard-delete) id_remove.

**Request body**: [`ConsolidatePairRequest`](#consolidatepairrequest)

*Related: `1076`*

#### 🟡 `POST` `/aging`

Run aging cleanup — deprioritize or remove old/stale memories.

**Request body**: [`AgingRequest`](#agingrequest)

#### 🟡 `POST` `/orphans`

List orphaned memories (not in any room, no edges).

**Request body**: [`OrphansRequest`](#orphansrequest)

#### 🟡 `POST` `/extract`

Extract entities and relationships from memory content.

**Request body**: [`ExtractRequest`](#extractrequest)

#### 🟡 `POST` `/rebuild-backlinks`

Rebuild backlink indices for memory graph.


## 🔴 Health & Info

#### 🟢 `GET` `/health`

Health check — tokenless requests get {status} only; a valid bearer token (or auth-disabled server) returns status, version, memory counts, and update info

**Response**: [`HealthResponse`](#healthresponse)

#### 🟢 `GET` `/routes`

Machine-readable API route introspection: returns this registry (method, path, tier, description, request/response types) as JSON — the same source of truth that generates docs/api-reference.md (#1289).

*Related: `#1289`*


## 🔵 Documents

#### 🟡 `POST` `/doc/create`

Create a new document with slug, title, content, tags.

**Request body**: [`DocCreateRequest`](#doccreaterequest)

#### 🟡 `POST` `/doc/get`

Get a document by slug.

**Request body**: [`DocGetRequest`](#docgetrequest)

#### 🟡 `POST` `/doc/list`

List documents with optional namespace/limit/roots_only/parent filters.

**Request body**: [`DocListParams`](#doclistparams)

#### 🟡 `POST` `/doc/search`

Search documents by query with optional mode/namespace/limit. `hybrid` (default) and `semantic` embed the query, so 429 when two embedding requests already run (`fts` never does). The query is at most 2000 characters in every mode (400 otherwise).

**Request body**: [`DocSearchRequest`](#docsearchrequest)

#### 🟡 `POST` `/doc/similar`

Documents close to a stored document (`slug` or `id`) or to a draft `text` that is not stored, or, with `report: true`, the near-duplicate pairs and groups of a namespace or a `subtree`. Compares document vectors (the mean of the chunk vectors): `possible_duplicate` at 0.92 and above, `similar_to` at 0.80 and above (`[doc_similarity]` in uteke.toml), `threshold` overrides the floor, `limit` caps the hits (default 10, max 100; a report lists 50 pairs, max 500). Documents without vectors are skipped and counted in `documents_without_vector`. Nothing is written; `text` (at most 32768 characters) needs the embedding model (503 without; 429 when two embedding requests already run). Read-only.

**Request body**: [`DocSimilarRequest`](#docsimilarrequest)

*Related: `1730`*

#### 🟡 `POST` `/doc/update`

Update an existing document (content, title, tags, parent).

**Request body**: [`DocUpdateRequest`](#docupdaterequest)

#### 🟡 `POST` `/doc/move`

Move a document to a different parent.

**Request body**: [`DocMoveRequest`](#docmoverequest)

#### 🔴 `DELETE` `/doc/delete`

Delete a document by slug or ID.

#### 🟡 `POST` `/doc/mem-refs`

Get memories that reference a specific document.

**Request body**: JSON object (see handler source for fields)


## 🟠 Graph

#### 🟡 `POST` `/graph/edge`

Add a directed edge between two memories. Accepts memory IDs (a linked graph node is ensured automatically, #1180) or existing graph node IDs. Returns `{ok, source_node, target_node}`.

**Request body**: [`GraphEdgeRequest`](#graphedgerequest)

*Related: `1180`*

#### 🔴 `DELETE` `/graph/edge`

Remove an edge between two nodes. Accepts memory IDs or graph node IDs via `?source=...&target=...` query params (#1180).

*Related: `1180`*

#### 🟢 `GET` `/edges`

List all `memory_edges` of one memory (outgoing and incoming, including derived backlinks). Not an alias of `/graph`. Accepts `?id=...` query param.

#### 🟢 `GET` `/timeline`

Get timeline of memory events for a memory. Accepts `?id=...` query param.


## 🟡 Core Memory

#### 🟡 `POST` `/remember`

Store a new memory. Accepts content, tags, namespace, type, metadata. Explicit `type` is honored as-is (no silent normalization, #1302); unknown types are rejected with 400 listing the valid vocabulary. Omit `type` to auto-infer.

**Request body**: [`RememberRequest`](#rememberrequest)

**Response**: [`Memory`](#memory)

#### 🟡 `POST` `/recall`

Semantic search — recall memories by meaning. Returns ranked results. Set `explain: true` (#1160) to include per-result ranking signals (vector similarity/rank, RRF contributions, boosts); memory-only recall.

*Excludes deprecated memories from results.*

**Request body**: [`RecallRequest`](#recallrequest)

#### 🟡 `POST` `/search`

Keyword search — find memories by matching words in content/tags.

*Excludes deprecated memories from results.*

**Request body**: [`SearchRequest`](#searchrequest)

#### 🟡 `POST` `/list`

List memories with optional filters (namespace, tags, sort, limit, offset). Set `include_meta: true` (#1188) for a pagination envelope `{memories, total, has_more, next_offset}` instead of the bare array (not supported with `at`).

*Excludes deprecated memories from results.*

**Request body**: [`ListParams`](#listparams)

#### 🔴 `DELETE` `/forget`

Deprecate a memory by ID. Returns 404 if ID doesn't exist.

#### 🟢 `GET` `/recent`

Get recently added memories. Accepts `?limit=N&namespace=X` query params.

*Excludes deprecated memories from results.*


## 🟢 Rooms

#### 🟡 `POST` `/room/create`

Create a new memory room. Accepts `{"name": "..."}`. Room ids are meant to have the shape `category:title` (for example `project:uteke`, `research:docker`; lowercase, one colon, at most 64 characters; the category is a recommendation, any category of that shape is accepted). Another id is still created, and the response carries `warnings` with one message (#1715); with `[rooms] require_category_title` on it is refused with 400 `invalid_room_id`.

**Request body**: JSON object (see handler source for fields)

#### 🟡 `POST` `/room/remember`

Store a memory linked to a room. Accepts room_id, content, tags, type, author. A room that does not exist is created; when its id is not of the shape `category:title` the response carries `warnings` with one message, as on `/room/create` (#1715), or the write is refused with 400 `invalid_room_id` (nothing stored) when `[rooms] require_category_title` is on.

**Request body**: [`RoomRememberRequest`](#roomrememberrequest)

**Response**: [`Memory`](#memory)

*Related: `#789`*

#### 🟡 `POST` `/room/recall`

Semantic search within a room. Empty query returns all memories chronologically.

*Excludes deprecated memories from results.*

**Request body**: [`RoomRecallRequest`](#roomrecallrequest)

*Related: `#785`*

#### 🟡 `POST` `/room/summary`

Get room summary with memory clusters and statistics.

*Excludes deprecated memories from results.*

**Request body**: JSON object (see handler source for fields)

#### 🟡 `POST` `/room/summary-document`

Get room summary focused on document-type memories.

*Excludes deprecated memories from results.*

**Request body**: JSON object (see handler source for fields)

#### 🟢 `GET` `/room/list`

List all rooms.

#### 🟡 `POST` `/room/consolidate`

Plan or execute segment-level LLM consolidation of room memories (#1088). Dry-run by default; `apply: true` executes with a hard budget cap. Write op — blocked for read-only tokens.

**Request body**: JSON object (see handler source for fields)

*Related: `#1088`*

#### 🟡 `POST` `/room/stats`

Get memory count for a room. Includes deprecated memories (known discrepancy vs /room/summary).

**Request body**: JSON object (see handler source for fields)

*Related: `#784`*

#### 🟡 `POST` `/room/search`

Search rooms by query (#1704). Fuses a metadata match (room id, title, description) with the rooms of the memories a recall of the query finds. Body: `{query, limit?, namespace?}`; `limit` defaults to 10, max 100; a blank query is a 400. Returns `[{room_id, room_name, description?, memory_count, updated_at, score, matches?}]`, best first; deprecated memories are not counted. With `namespace`, only that namespace's rooms are returned.

*Excludes deprecated memories from results.*

**Request body**: JSON object (see handler source for fields)

*Related: `#1704`*

#### 🟢 `GET` `/room/memories`

List all memories in a room (chronological). Accepts `?room_id=...` query param; optional `?author=...` and `?namespace=...` filters (#1288). Without `namespace`, returns ALL namespaces that contributed to the room.

*Excludes deprecated memories from results.*

#### 🔴 `DELETE` `/room/delete`

Delete a room (unlink-only): room links are removed, memories and documents are preserved. Response: `{ deleted, unlinked_memories }`. Accepts `?room_id=...` query param.

#### 🟡 `POST` `/room/rename`

Rename a room: the registry row and every member/document link move in one transaction; title, description, and namespace are preserved (#1202). The old id stays valid as an alias of the renamed room (#1714): every room operation resolves it and answers with the canonical id, and a room cannot be created under an alias. Accepts `{ from, to }` (`from` may itself be an alias). A `to` outside the `category:title` shape is accepted with a `warnings` entry in the response (#1715), or refused with 400 `invalid_room_id` when `[rooms] require_category_title` is on. Write op — blocked for read-only tokens.

**Request body**: JSON object (see handler source for fields)

*Related: `#1202`*

#### 🟡 `POST` `/room/update`

Update a room's title and/or description; omitted fields stay unchanged (#1202). Accepts `{ room_id, title?, description? }`; 404 when the room is missing. Write op — blocked for read-only tokens.

**Request body**: JSON object (see handler source for fields)

*Related: `#1202`*

#### 🟡 `POST` `/room/memory`

Add an existing memory to a room without copying it (#1589). Accepts `{ memory_id, room, author? }` (`memory_id` is a full id or an unambiguous prefix; `author` defaults to `anonymous`). Idempotent: linking twice returns `already_linked: true` and keeps the first link. 404 when the room or the memory does not exist. Write op — blocked for read-only tokens.

**Request body**: JSON object (see handler source for fields)

*Related: `#1589`*

#### 🟡 `POST` `/room/memory/move`

Move a memory from one room to another, preserving the link's author/role/joined_at provenance (#1202). Accepts `{ memory_id, from_room, to_room }`; 404 when the memory has no link in `from_room`. Write op — blocked for read-only tokens.

**Request body**: JSON object (see handler source for fields)

*Related: `#1202`*

#### 🟡 `POST` `/room/document`

Store a reference document in a room (large content >500 chars).

**Request body**: JSON object (see handler source for fields)

#### 🟡 `POST` `/room/document/list`

List documents in a room.

**Request body**: JSON object (see handler source for fields)

#### 🔵 `PUT` `/room/document/add`

Add a reference to an existing document in a room.

**Request body**: JSON object (see handler source for fields)

#### 🔴 `DELETE` `/room/document/remove`

Remove a document reference from a room.

**Request body**: JSON object (see handler source for fields)

#### 🟡 `POST` `/doc/room/list`

List rooms that reference a specific document.

**Request body**: JSON object (see handler source for fields)


## 🟣 Memory Management

#### 🟡 `POST` `/memory/pin`

Pin a memory so it won't be removed by aging/cleanup operations.

**Request body**: [`MemoryPinRequest`](#memorypinrequest)

#### 🟡 `POST` `/memory/importance`

Get or set the importance score of a memory.

**Request body**: [`MemoryImportanceRequest`](#memoryimportancerequest)

#### 🟡 `POST` `/memory/feedback`

Submit positive/negative feedback on a memory for ranking signals.

**Request body**: [`MemoryFeedbackRequest`](#memoryfeedbackrequest)

#### 🟡 `POST` `/memory/doc-refs`

Get documents that reference a specific memory.

**Request body**: JSON object (see handler source for fields)


## 🤖 AI Integration

#### 🟡 `POST` `/context`

Get context window for a query (for LLM prompt enrichment).

**Request body**: JSON object (see handler source for fields)

#### 🟡 `POST` `/dream`

Generate new memories/insights from existing memory corpus.

**Request body**: JSON object (see handler source for fields)

#### 🟡 `POST` `/mcp`

MCP (Model Context Protocol) bridge endpoint for AI agent tool calls.

**Request body**: JSON object (see handler source for fields)


## Request/Response Schemas

Detailed field definitions for each request type.

### `AgingRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `action` | `string` | No |  |
| `dry_run` | `boolean` | No |  |
| `max_access_count` | any | No | Max access count threshold for preview/cleanup (default: 1). |
| `namespace` | any | No |  |
| `older_than_days` | any | No | Days threshold for preview/cleanup (default: warm_days from config, fallback 90). |


### `CaptureRequest`

One turn for the capture queue (`POST /capture`, LAB).

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | `string` | Yes |  |
| `namespace` | any | No | Namespace the extracted facts are stored in. |
| `role` | `string` | Yes | `user`, `assistant` or `tool` (a tool output keeps its first 2000 characters). |
| `session_id` | `string` | Yes | Conversation the turn belongs to; its turns are extracted together. |
| `source` | any | No | Optional source label (e.g. the client name). |


### `ConsolidateRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `dry_run` | `boolean` | No |  |
| `namespace` | any | No |  |
| `threshold` | `number` | No |  |


### `DocCreateRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | `string` | Yes |  |
| `parent` | any | No |  |
| `slug` | `string` | Yes |  |
| `tags` | ``string``[] | No |  |
| `title` | any | No |  |


### `DocGetRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | any | No |  |
| `slug` | any | No |  |


### `DocMoveRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | any | No |  |
| `new_parent` | any | No |  |
| `new_sort_order` | any | No | Optional sort order for the moved document (#sort-order). |
| `slug` | any | No |  |


### `DocSimilarRequest`

`POST /doc/similar` (#1730): documents close to a document or to a draft
text, or (with `report`) the near-duplicates of a namespace or subtree.

Similar documents: give `slug` (or `id`) OR `text`, plus optional
`threshold`, `limit`, `namespace`. Duplicate report: `report: true` plus
optional `subtree`, `namespace`, `threshold`, `limit`; no `slug`/`text`.
Nothing is written either way.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | any | No | Id (or slug) of a stored document. |
| `limit` | any | No | Maximum hits (default 10, at most 100), or pairs for a report (default 50, at most 500). |
| `namespace` | any | No | Only compare with documents of this namespace (default: all). |
| `report` | `boolean` | No | `true`: report the near-duplicate pairs and groups of the scope. |
| `slug` | any | No | Slug of a stored document (alias of `id`). |
| `subtree` | any | No | Report only: the slug or id of the document whose subtree to scan. |
| `text` | any | No | Candidate text that is not stored: the near-duplicates "before it is stored". Needs the embedding model. |
| `threshold` | any | No | Minimum cosine similarity, 0 to 1. Default: the configured `similar_to` threshold (0.80), or `possible_duplicate` (0.92) for a report. |


### `ErrorResponse`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `error` | `string` | Yes |  |


### `ExtractRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | `string` | Yes |  |
| `max_facts` | any | No | Override max facts per document. |
| `model` | any | No | Override extraction model (else config default). |
| `namespace` | any | No |  |
| `tags` | ``string``[] | No |  |
| `type` | any | No |  |


### `GraphEdgeRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `edge_type` | any | No |  |
| `source` | `string` | Yes |  |
| `target` | `string` | Yes |  |
| `weight` | any | No |  |


### `HealthResponse`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `api_latest` | any | No | Latest API version (#737). |
| `api_versions` | any | No | Supported API versions (#737). |
| `memories` | `integer` | Yes |  |
| `namespaces` | `integer` | Yes |  |
| `status` | `string` | Yes |  |
| `update_available` | any | No | Latest version available on GitHub, if newer than current. Populated from cache (24h TTL) — may be `None` if cache is stale. |
| `version` | `string` | Yes | Server version (uteke-server crate version), so HTTP clients can gate features on the actual server capability rather than a local CLI probe. |


### `ImportRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | `string` | Yes | JSONL content to import. |
| `defer_embedding` | `boolean` | No | Store the rows without a vector and return at once (#1634, default false). The rows are keyword-searchable (`/search`) immediately and found by `/recall` after `uteke repair --reembed` adds the vectors. Duplicates are caught by exact content only, not by the cosine check. The response then carries `deferred_embedding`: the rows waiting for a vector. Use it for bulk imports, which otherwise embed every row before the request returns. |
| `namespace` | any | No |  |
| `skip_deprecated_copies` | `boolean` | No | Skip a live row whose content matches only a deprecated copy (forgotten, superseded, consolidated or pruned) instead of importing it as a new live memory (#1716, default false). A skipped row counts in `duplicates_skipped`. Rows that carry the id of a deprecated memory are skipped either way. |
| `tags` | ``string``[] | No |  |


### `ImportanceRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `namespace` | any | No |  |


### `ListParams`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `at` | any | No | Time-travel: list memories that existed at this RFC3339 timestamp. |
| `include_meta` | `boolean` | No | Pagination metadata (#1188): when true, respond with an envelope `{memories, total, has_more, next_offset}` instead of the bare array. Default false — the bare-array shape is unchanged for existing clients. Not supported with `at` (point-in-time listing returns the bare array regardless). |
| `limit` | `integer` | No |  |
| `namespace` | any | No |  |
| `offset` | `integer` | No |  |
| `tag` | any | No |  |


### `MemoryFeedbackRequest`

Request for memory feedback / trust scoring (#718).

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `feedback` | `string` | Yes | "helpful" or "unhelpful" |
| `id` | `string` | Yes |  |


### `MemoryImportanceRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | `string` | Yes |  |
| `importance` | `number` | Yes |  |


### `MemoryPinRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | `string` | Yes |  |
| `pinned` | `boolean` | Yes |  |


### `MemoryUpdateRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | any | No | New content. Triggers embedding regeneration. |
| `id` | `string` | Yes | UUID of the memory to update (required). |
| `importance` | any | No | Set importance score (0.0–1.0). |
| `memory_type` | any | No | Set memory type (fact, procedure, preference, decision, context, note, insight, reference, event). |
| `metadata` | any | No | Replace metadata entirely with this object. |
| `namespace` | any | No | Move the memory to this namespace (#1181). Plain column update — no re-embed. |
| `pinned` | any | No | Set pinned state. |
| `tags` | any | No | Replace tags entirely with this list. |
| `valid_from` | any | No | When the fact starts to hold (RFC 3339, #1598). Stored in the `valid_from` column that `at` reads. Must be before `valid_until`. |
| `valid_until` | any | No | When the fact stops holding (RFC 3339, #1598). Stored in the `valid_until` column. Refused on a deprecated memory. |


### `OrphansRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `limit` | `integer` | No |  |
| `namespace` | any | No |  |
| `threshold` | `number` | No |  |


### `PinRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | `string` | Yes |  |


### `PruneRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `dry_run` | `boolean` | No |  |
| `namespace` | any | No |  |
| `ttl_days` | `integer` | No |  |


### `RecallRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `after` | any | No | Temporal range filter: only return memories created at or after this RFC3339 timestamp (#902). |
| `at` | any | No | Time-travel: query memories that existed at this RFC3339 timestamp. |
| `before` | any | No | Temporal range filter: only return memories created at or before this RFC3339 timestamp (#902). |
| `budget_chars` | any | No | Character budget for `pack` (default 4000). |
| `category` | any | No | Filter by category metadata. |
| `enrich` | `boolean` | No | Enrich results with cross-entity links (doc↔memory) (#689). When true, populates `linked_doc_slugs` on memory results and `linked_memory_ids` on document results. |
| `entity` | any | No | Filter by entity metadata. |
| `exclude_ids` | any | No | Memory IDs already injected this turn; excluded from pack results (reported as `skipped` with reason `excluded`). |
| `explain` | `boolean` | No | Explain mode (#1160): return per-result ranking signals alongside each memory. Memory-only recall — rejected (400) together with search_type/unified, at, before/after. |
| `limit` | `integer` | No |  |
| `min_score` | any | No | Minimum similarity score (0.0-1.0). Results below are filtered. Default: 0.0 (no filtering). Compares against `score`; `strict` compares against `similarity`. |
| `min_trust` | any | No | Lowest provenance tier to return (#1651): `human`, `agent`, `external` or `untrusted` (lowercase). Filters the tier (author_type + source_type) only, before the limit is applied. author_type is self-declared, so this is a convenience filter, not a security boundary, and it is not a trust state. |
| `namespace` | any | No |  |
| `pack` | `boolean` | No | Budgeted context pack (#1281 Phase 1): return a ContextPack (selected/skipped/budget_used) instead of a bare ranked list. Deterministic, LLM-free, rank-order preserving. |
| `query` | `string` | Yes |  |
| `search_type` | any | No | Search type filter: "all" (default, unified), "memory", or "doc" (#531). |
| `strategy` | any | No | Recall strategy: "fusion" (default since 0.16.0), "vector", "fts5", "hybrid", or "graph" (#900, #1034, #1123). When absent, the server falls back to `[recall] default_strategy` from uteke.toml, then to "fusion" — matching the CLI default. Invalid values return HTTP 400. |
| `strict` | `boolean` | No | Confident matches only (#1650): keep a hit when its `similarity` (the vector-leg cosine) reaches `[recall] min_score_strict` (default 0.5). Hits without a similarity (found by full-text search only) are dropped. `score` and `min_score` keep their meaning. |
| `tags` | ``string``[] | No |  |


### `RememberRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `author` | any | No | Who wrote the memory (#1427 phase 2): free text, trimmed, 1 to 128 characters. Optional, with or without a room; returned on the memory. |
| `author_type` | any | No | Author type: "human" | "agent" (#1083). Defaults to "agent" when omitted. |
| `category` | any | No | Category — stored as metadata key "category". |
| `content` | `string` | Yes |  |
| `detect_contradiction` | `boolean` | No |  |
| `entity` | any | No | Entity name — stored as metadata key "entity". |
| `metadata` | any | No | Extra metadata key=value pairs, merged into the metadata map. Accepts an object (e.g. {"project": "uteke"}). |
| `namespace` | any | No |  |
| `source` | any | No | Source provenance — set via set_source() after storage. |
| `source_type` | any | No | Source type (defaults to "user"). |
| `tags` | ``string``[] | No |  |
| `type` | any | No | Memory type — validated against the MemoryType vocabulary and stored as-is (no silent normalization, #1302). Unknown values are rejected with 400 listing the valid types. Omit to let auto-inference decide. |
| `valid_from` | any | No |  |
| `valid_until` | any | No |  |


### `RoomRecallRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `at` | any | No | Time-travel: recall room state as of this RFC3339 timestamp (#1082). Memories created after `at` (or invalidated before it) are excluded. |
| `author` | any | No |  |
| `limit` | `integer` | No |  |
| `min_score` | any | No |  |
| `query` | any | No | Semantic search query. When `None` or empty, falls back to chronological recall (equivalent to `GET /room/memories`) (#785). |
| `room_id` | `string` | Yes |  |


### `RoomRememberRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `author` | any | No | Author — stored as participant role in room link. |
| `content` | `string` | Yes |  |
| `metadata` | any | No |  |
| `namespace` | any | No |  |
| `room_id` | `string` | Yes |  |
| `tags` | ``string``[] | No |  |
| `type` | any | No |  |


### `SearchRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `limit` | `integer` | No |  |
| `namespace` | any | No |  |
| `query` | `string` | Yes |  |
| `tags` | ``string``[] | No |  |


### `SkillListRequest`

`POST /skill/list` (#1731): the skills of the library, by slug. Read-only.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `limit` | any | No | Maximum skills, 1 to 200 (default 50). |
| `status` | any | No | `draft` or `approved`: the status of the current version. |


### `SkillSearchRequest`

`POST /skill/search` (#1731): the skills whose files match a query. Read-only.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `approved_only` | any | No | Only skills whose current version is approved. |
| `limit` | any | No | Maximum skills, 1 to 50 (default 5). |
| `mode` | any | No | `hybrid` (default), `semantic` or `fts`. |
| `query` | `string` | Yes | What to look for (at most 2000 characters). |


### `SkillShowRequest`

`POST /skill/show` (#1731): one skill with its current version, usage,
version history and mutation ledger. Read-only.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `slug` | `string` | Yes | The skill (at most 64 characters). |


### `SkillSimilarRequest`

`POST /skill/similar` (#1731): skills close to a skill or to a draft text
that is not stored. Give `slug` OR `text`. Read-only.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `limit` | any | No | Maximum hits, 1 to 100 (default 10). |
| `slug` | any | No | A skill of the library. |
| `text` | any | No | A draft text that is not stored (at most 32768 characters; 429 when two embeddings already run); needs the embedding model. |
| `threshold` | any | No | Minimum cosine similarity, 0 to 1 (default 0.80). |


### `TagDeleteRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `namespace` | any | No |  |
| `tag` | `string` | Yes |  |


### `TagRenameRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `namespace` | any | No |  |
| `new` | `string` | Yes |  |
| `old` | `string` | Yes |  |


