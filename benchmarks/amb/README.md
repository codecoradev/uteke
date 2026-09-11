# AMB — Agent Memory Benchmark run kit

Reproduction kit for running [AMB](https://github.com/vectorize-io/agent-memory-benchmark)
(external benchmark #3) against uteke. The upstream harness has no license file, so
this directory ships **patches against upstream** rather than a vendored copy.

Status: 🚧 run in progress — RESULTS pending (full-1540 run + Gemini re-judge).

## Provenance

| Artifact | Value |
|---|---|
| Upstream repo | `vectorize-io/agent-memory-benchmark` |
| Upstream base commit | `347f551b1014d8f8d5fab9eb38175f457a6ed195` |
| Local branch | `feat/uteke-provider-and-compat` (4 commits, `16f940e`..`7b76a0e`) |
| Uncommitted work | session-date ingest anchors + harness fixes → `patches/0005-*.patch` |
| Snapshot date | 2026-09-11 (issue #1226: the only copy lived in a throwaway clone) |

## Contents

```
amb/
├── README.md                          ← you are here: provenance + run notes
├── patches/
│   ├── 0001-…  uteke memory provider + OpenAI-compat fixes
│   ├── 0002-…  ingest via temp file + `uteke import`, not argv
│   ├── 0003-…  unique temp file per ingest item (concurrency)
│   ├── 0004-…  retry on missing required keys in LLM JSON responses
│   └── 0005-…  UNCOMMITTED upstream work: session-date ingest anchors,
│               raw-response full-payload fix, system-role JSON contract,
│               repair-retry, graceful per-query degrade, AMB_TEMPERATURE,
│               AMB_DEBUG_DUMP (see "Load-bearing fixes" below)
└── scripts/
    └── apply_patches.sh               ← clone upstream + apply all patches
```

## Reproduce

```bash
# 1. Clone upstream and apply the uteke provider patches
./scripts/apply_patches.sh /path/to/amb-uteke

# 2. Configure (see "Run notes" for exact env)
export AMB_PROVIDER=uteke
export OPENAI_BASE_URL=https://api.z.ai/api/coding/paas/v4   # GLM via Z.AI coding endpoint
export OPENAI_MODEL=glm-5.3
export AMB_THINKING_DISABLED=1          # thinking eats the token budget on GLM
export AMB_TEMPERATURE=0                # deterministic runs
# export AMB_DEBUG_DUMP=/tmp/amb-debug.jsonl   # capture full message payloads

# 3. Run the harness per upstream README (locomo dataset), then judge.
```

## Load-bearing fixes (why the patches matter)

Measured on conv-30 (LoCoMo), uteke 0.17.0, embeddinggemma-q4, GLM-5.3 judge:

1. **Raw response must be the FULL recall payload** — locomo's prompt template
   `json.dumps()`es the provider's raw response INSTEAD of the assembled context.
   A hit-count stub silently produces empty-context prompts.
   Artifact: 8.6% accuracy on the full run; canary fix trajectory
   4.9% / 11.1% (empty context) → 69.1% (rich context, no dates) → **93.8%** (dated).
2. **Session dates must be prepended at ingest** (`[Session date/time: …]` header) —
   LoCoMo temporal QA demands absolute-date anchors; without them conv-30 temporal
   = 15%, with them = 100% (26/26).
3. **JSON contract via SYSTEM role, not a user-message prefix** — with thinking
   disabled, GLM-5.3 (z.ai coding endpoint) degenerates on the combined single
   user message: echoes the schema or mass-abstains even when the answer is in
   the context.
4. **Repair-retry conversation** after repeated format failures beats blind re-rolls.
5. **Graceful per-query degrade** — one unparseable response must not kill a
   multi-hour run; a sentinel-marked row still lands in the output.

## RESULTS

Pending — will be filled after the full-1540 run completes and the Gemini
flash-lite re-judge (judge uniformity with other benchmark entries) is done.
