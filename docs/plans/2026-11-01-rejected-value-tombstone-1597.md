# Rejected-value tombstone (#1597)

Status: **decided by the owner on 2026-11-01, implementing.** CORE contract issue: uteke-cloud#437 (Gitea, see §6).

## 1. Problem

Forgetting is not durable. A value that was forgotten because it was wrong can come back:

- the duplicate gate (`check_duplicate`, `operations/write.rs`) skips deprecated rows, so the same wrong text is stored again as a new memory;
- nothing records "this value was rejected";
- `prune` deletes deprecated rows after the TTL, so even the deprecated row disappears;
- scheduled extraction (#1466) would re-derive the same wrong fact from the same raw input.

## 2. Decisions (owner, 2026-11-01)

| Question | Decision |
|---|---|
| What creates a tombstone | Only a forget or supersede whose reason is `wrong` (`--reason wrong`, or `wrong: <note>`). An ordinary forget (outdated, done) does not block writing the value again. |
| A write that matches a tombstone | Refused with a typed error, `rejected_value`. Never stored, never silent. |
| Import of a backup | The row is skipped and counted as `rejected_tombstone` (like `rejected_secret`); the import goes on. A `--full` bundle carries the tombstones. |
| Matching | Exact text after normalisation: NFKC, case folding, whitespace collapsed and trimmed, invisible characters (zero-width, soft hyphen, BOM) removed. No vector similarity, so no false positives; a paraphrase is not caught. |

## 3. Design

### 3.1 Storage (schema v23, additive)

```
rejected_values(
  hash        TEXT PRIMARY KEY,   -- sha256 of the normalised text
  memory_id   TEXT,               -- the memory that was rejected (may be pruned later)
  reason      TEXT NOT NULL,      -- the reason as given, e.g. "wrong: the port is 8767"
  preview     TEXT NOT NULL,      -- first 120 characters of the text, for `tombstone list`
  created_at  TEXT NOT NULL
)
```

One store-wide set (namespaces are being removed, #1427). The table is not touched by `prune`, TTL, aging or `forget`, so a tombstone outlives the row it came from. The text is stored after the privacy filter, like every memory, so a preview never holds a secret.

### 3.2 Creating a tombstone

- `uteke forget <id> --reason wrong[: note]`, `DELETE /forget?id=..&reason=..` (LAB) and the MCP `uteke_forget` tool (`reason`): soft-deprecate as today, plus the tombstone. Other reasons only deprecate.
- `uteke supersede <old> <new> --reason wrong[: note]` (and its HTTP/MCP forms): the old value gets a tombstone.
- Forgetting by tag never creates tombstones (too broad to be a judgement about each value).

### 3.3 Checking

The check runs in `commit_memory`, the single write tail of `remember`, room remember, import and consolidation (#1514), so every current and future write path is covered, including extraction (it stores through `remember`). `update_memory` checks new content too, and the structural import (`import --full`) checks each memory row. A hit returns `Error::RejectedValue { reason }`:

- HTTP: `409` with `{"error": "rejected_value", "reason": "..."}`; the stored text is not echoed.
- MCP: a tool error starting with `rejected_value:`.
- CLI: exit code 1 with the reason and the way out (`uteke tombstone remove`).
- Import: the row is skipped and counted in `rejected_tombstone`.

### 3.4 Restoring

- `uteke tombstone list` shows hash prefix, reason, preview and date.
- `uteke tombstone remove <hash-prefix | memory-id>` deletes one.
- `uteke promote <id>` (restore a deprecated memory) also removes that memory's tombstone: restoring a value is the explicit statement that it is not wrong.

### 3.5 Export / import

`export --full` writes a `rejected_values` section; `import --full` restores it before the memories, so a moved store keeps refusing the same values. Older bundles have no section and import as before. The JSONL export (`/export`, CORE) is unchanged.

## 4. Out of scope

- Similarity matching (paraphrases). Can be added later as an opt-in warning.
- A reason vocabulary beyond `wrong` for tombstones. Other reasons stay free text on the deprecation.
- #1466 (scheduled extraction) gets the check for free through `commit_memory`.

## 5. Tests

Normalisation (case, NFKC width forms, zero-width characters, whitespace); forget `wrong` versus an ordinary forget; refusal on remember, room remember, update, import (counted) and `import --full`; survival after `prune`; restore by `tombstone remove` and by `promote`; `--full` round trip of the tombstones; HTTP 409 body; MCP error text.

## 6. CORE contract

`POST /remember`, `POST /room/remember` and `POST /import` are CORE. A new refusal (409 `rejected_value`) and a new count in the `/import` response (`rejected_tombstone`, only when non-zero) are contract-visible, so a uteke-cloud contract issue is opened before the code lands. Uteke Cloud keeps its own write path; it needs to answer whether it adopts the same tombstone semantics or treats the new status as OSS-only.
