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

#### 🟢 `GET` `/namespaces`

List all namespaces in the memory store

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

#### 🟢 `GET` `/graph`

Get graph edges for a memory. Accepts `?id=...` query param.

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

Import memories from a JSON array.

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

Health check — returns server status and version

**Response**: [`HealthResponse`](#healthresponse)


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

Search documents by query with optional mode/namespace/limit.

**Request body**: [`DocSearchRequest`](#docsearchrequest)

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

Add a directed edge between two memories.

**Request body**: [`GraphEdgeRequest`](#graphedgerequest)

#### 🔴 `DELETE` `/graph/edge`

Remove an edge between two memories. Accepts `?from=...&to=...` query params.

#### 🟢 `GET` `/edges`

List edges for a memory (alias for /graph). Accepts `?id=...` query param.

#### 🟢 `GET` `/timeline`

Get timeline of memory events for a memory. Accepts `?id=...` query param.


## 🟡 Core Memory

#### 🟡 `POST` `/remember`

Store a new memory. Accepts content, tags, namespace, type, metadata.

**Request body**: [`RememberRequest`](#rememberrequest)

**Response**: [`Memory`](#memory)

#### 🟡 `POST` `/recall`

Semantic search — recall memories by meaning. Returns ranked results.

*Excludes deprecated memories from results.*

**Request body**: [`RecallRequest`](#recallrequest)

#### 🟡 `POST` `/search`

Keyword search — find memories by matching words in content/tags.

*Excludes deprecated memories from results.*

**Request body**: [`SearchRequest`](#searchrequest)

#### 🟡 `POST` `/list`

List memories with optional filters (namespace, tags, sort, limit, offset).

*Excludes deprecated memories from results.*

**Request body**: [`ListParams`](#listparams)

#### 🔴 `DELETE` `/forget`

Deprecate a memory by ID. Returns 404 if ID doesn't exist.

#### 🟢 `GET` `/recent`

Get recently added memories. Accepts `?limit=N&namespace=X` query params.

*Excludes deprecated memories from results.*


## 🟢 Rooms

#### 🟡 `POST` `/room/create`

Create a new memory room. Accepts `{"name": "..."}`.

**Request body**: JSON object (see handler source for fields)

#### 🟡 `POST` `/room/remember`

Store a memory linked to a room. Accepts room_id, content, tags, type, author.

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

#### 🟡 `POST` `/room/stats`

Get memory count for a room. Includes deprecated memories (known discrepancy vs /room/summary).

**Request body**: JSON object (see handler source for fields)

*Related: `#784`*

#### 🟢 `GET` `/room/memories`

List all memories in a room (chronological). Accepts `?room_id=...` query param.

*Excludes deprecated memories from results.*

#### 🔴 `DELETE` `/room/delete`

Delete a room and all its memories. Accepts `?room_id=...` query param.

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
| `update_available` | any | No | Latest version available on GitHub, if newer than current.
Populated from cache (24h TTL) — may be `None` if cache is stale. |
| `version` | `string` | Yes | Server version (uteke-server crate version), so HTTP clients can gate
features on the actual server capability rather than a local CLI probe. |


### `ImportRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | `string` | Yes | JSONL content to import. |
| `namespace` | any | No |  |
| `tags` | ``string``[] | No |  |


### `ImportanceRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `namespace` | any | No |  |


### `ListParams`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `at` | any | No | Time-travel: list memories that existed at this RFC3339 timestamp. |
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
| `pinned` | any | No | Set pinned state. |
| `tags` | any | No | Replace tags entirely with this list. |


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
| `after` | any | No | Temporal range filter: only return memories created at or after this
RFC3339 timestamp (#902). |
| `at` | any | No | Time-travel: query memories that existed at this RFC3339 timestamp. |
| `before` | any | No | Temporal range filter: only return memories created at or before this
RFC3339 timestamp (#902). |
| `category` | any | No | Filter by category metadata. |
| `enrich` | `boolean` | No | Enrich results with cross-entity links (doc↔memory) (#689).
When true, populates `linked_doc_slugs` on memory results and
`linked_memory_ids` on document results. |
| `entity` | any | No | Filter by entity metadata. |
| `limit` | `integer` | No |  |
| `min_score` | any | No | Minimum similarity score (0.0-1.0). Results below are filtered.
Default: 0.0 (no filtering). Use `strict=true` for 0.5 default (#995). |
| `namespace` | any | No |  |
| `query` | `string` | Yes |  |
| `search_type` | any | No | Search type filter: "all" (default, unified), "memory", or "doc" (#531). |
| `strategy` | any | No | Recall strategy: "hybrid" (default), "vector", "fts5", or "graph" (#900, #1034).
When absent, the server falls back to `[recall] default_strategy` from
uteke.toml, then to "hybrid" — matching the CLI default.
Invalid values return HTTP 400. |
| `strict` | `boolean` | No | Use strict threshold (defaults to 0.5 if min_score not set). |
| `tags` | ``string``[] | No |  |


### `RememberRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `author_type` | any | No | Author type: "human" | "agent" (#1083). Defaults to "agent" when omitted. |
| `category` | any | No | Category — stored as metadata key "category". |
| `content` | `string` | Yes |  |
| `detect_contradiction` | `boolean` | No |  |
| `entity` | any | No | Entity name — stored as metadata key "entity". |
| `metadata` | any | No | Extra metadata key=value pairs, merged into the metadata map.
Accepts an object (e.g. {"project": "uteke"}). |
| `namespace` | any | No |  |
| `source` | any | No | Source provenance — set via set_source() after storage. |
| `source_type` | any | No | Source type (defaults to "user"). |
| `tags` | ``string``[] | No |  |
| `type` | any | No |  |
| `valid_from` | any | No |  |
| `valid_until` | any | No |  |


### `RoomRecallRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `author` | any | No |  |
| `limit` | `integer` | No |  |
| `min_score` | any | No |  |
| `query` | any | No | Semantic search query. When `None` or empty, falls back to
chronological recall (equivalent to `GET /room/memories`) (#785). |
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


