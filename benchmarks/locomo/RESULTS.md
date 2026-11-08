# LoCoMo Benchmark Results — Uteke

**Dataset:** [LoCoMo](https://github.com/snap-research/locomo) (Snap Research, ACL 2024) —
10 very long multi-session conversations, **1,536 evaluable QA** (see scope note below).
**Binary:** v0.17.0, built from exact release SHA `a2ec81a` (UTEKE_GIT_REF pin).
**Runs:** Modal x86_64, fan-out ×10 shards, fusion = zero-config default strategy, CPU-only,
no LLM in the retrieval path. Wall clock: 23 min (fusion).
**Raw artifacts (committed):** [`results/`](results/) — audit snippet at the bottom.

## Headline (fusion default, v0.17.0)

| Metric | Value |
|---|---|
| **recall_any@5** | **0.840** |
| recall_any@10 | 0.939 |
| strict recall_all@5 | 0.716 |
| strict recall_all@10 | 0.850 |
| coverage@5 | 0.776 |
| NDCG@5 | 0.652 |

## Strategy comparison (same run conditions)

| Strategy | strict@5 | strict@10 | cov@5 | any@5 | any@10 | NDCG@5 |
|---|---|---|---|---|---|---|
| **fusion (default)** | **0.716** | **0.850** | 0.776 | **0.840** | **0.939** | 0.652 |
| vector-only | 0.680 | 0.833 | 0.737 | 0.799 | 0.928 | 0.611 |
| fts5-only | 0.710 | 0.812 | 0.767 | 0.829 | 0.924 | 0.668 |

**Fusion wins on the strict family on a second external benchmark** — replicating the
LongMemEval-S story (see [`../longmemeval/RESULTS.md`](../longmemeval/RESULTS.md)): the two
channels win on *different* questions, and the union is stronger than either arm.

Per-category any@5 shows the complementarity directly:

| Category | n | vector | fts5-only | fusion |
|---|---|---|---|---|
| single-hop | 282 | 0.848 | 0.759 | **0.894** |
| temporal | 321 | 0.779 | **0.841** | 0.816 |
| multi-hop | 92 | 0.630 | 0.641 | 0.630 |
| open-domain | 841 | 0.810 | **0.868** | 0.855 |

Multi-hop is the weakest category under every strategy (~0.63 any@5 across the board):
connecting facts scattered across sessions is hard for pure session retrieval. Fusion
wins single-hop outright (0.894 vs vector 0.848), where the vector channel earns its
keep over lexical (0.848 vs 0.759); lexical wins phrasing-heavy temporal (0.841) and
open-domain (0.868) QA, and fusion lands at-or-near the best arm per category while
also taking the overall strict family.

> **Correction (2026-09-10):** earlier revisions of this file used a rotated category
> mapping (LoCoMo categories 1/2/3 were labeled multi-hop/single-hop/temporal instead
> of the official single-hop/temporal/multi-hop). Overall numbers were never affected
> (labels do not enter any metric); the tables above are recomputed from the same
> committed retrieval results (`results/*.jsonl`) with `question_type` corrected in
> place and `scripts/convert_locomo.py` fixed.

## Full fusion breakdown

| Category | n | strict@5 | strict@10 | any@5 | NDCG@5 |
|---|---|---|---|---|---|
| single-hop | 282 | 0.604 | 0.810 | 0.894 | 0.544 |
| temporal | 321 | 0.796 | 0.914 | 0.816 | 0.659 |
| multi-hop | 92 | 0.519 | 0.682 | 0.630 | 0.434 |
| open-domain | 841 | 0.854 | 0.944 | 0.855 | 0.709 |

Single-hop shows the strict-vs-any gap clearly (0.604 vs 0.894): these questions cite
~2.7 gold sessions on average, so "all gold in top-5" is much harder than "at least
one". Temporal QA cites almost exactly one evidence session (avg 1.1), which is why its
strict and any@5 sit close together. The earlier "temporal is weakest, consistent with
LongMemEval-S" reading was an artifact of the rotated labels: LoCoMo's temporal
category is date-lookup over a single evidence turn and retrieves fine here (0.816
any@5); the genuine weak spot is multi-hop.

## Scope notes (read before quoting)

- **Adversarial QA (446) excluded from the retrieval dataset** — by design they have no
  evidence in the haystack, so R@k is undefined. Handling them well is a QA/reader-layer
  capability (abstention), which we report as **unmeasured** here.
- 4 QA (category 3) have evidence that parses to zero sessions — also excluded.
- **Cross-benchmark comparison caution:** LoCoMo numbers from other systems are NOT
  directly comparable to ours unless their setup matches (many publish with LLM
  summarizers / different chunking / oracle evidence sets). Our numbers are
  deterministic retrieval-layer only: sessions inserted verbatim, EmbeddingGemma Q4
  local ONNX, zero-config default strategy. The ablation rows above are the apples-to-
  apples comparison.
- Dataset converted to the harness schema by
  [`scripts/convert_locomo.py`](scripts/convert_locomo.py) (sessions → `{role, content}`
  turns; `D1:3` evidence → `answer_session_ids`). Conversion is deterministic and
  committed; raw LoCoMo JSON ships from the upstream repo.

## Pilot consistency check

Pilot run (conversation 26, 150 QA, local ARM aarch64, v0.17.0) before the full Modal run:
overall strict@5 0.787 vs 0.753 for the same conversation inside the full x86 run — a
3.4pp gap consistent with a 150-question sample plus cross-architecture float noise on
near-ties (see the LongMemEval independent-reproduction section for the same effect
characterized at 107/108 identical rankings). No systematic pilot-vs-full deviation.

## Audit from the committed raw artifacts

```python
import json
entries = [json.loads(l) for l in open("results/default-1536.jsonl")]
s = [e["retrieval_results"]["metrics"]["session"] for e in entries]
print("strict@5:", sum(m["recall_all@5"] == 1.0 for m in s) / len(s))  # 0.716
print("cov@5   :", sum(m["recall_all@5"] for m in s) / len(s))         # 0.776
```

`recall_any@k` recomputes from `retrieved_session_ids` vs the dataset's
`answer_session_ids` (0.840 @5). Same commands work for `vector-1536.jsonl` /
`fts5-1536.jsonl`.

## End-to-end QA accuracy (answer + judge)

First end-to-end QA run (2026-11-08, tracked as #1691) — layers ON TOP of the
retrieval benchmark above: adapter (full recall payload as context, session-date
anchors) → answerer → judge. Retrieval metrics and QA accuracy are different
claims and are reported separately.

**Setup.** Binary built from develop `3bc72c1c` (v0.23.0 + the #1690
import-tombstone fix — exercised by this harness's per-question forget+re-import
cycle). Harness: external answer+judge runkit (agent-memory-benchmark) via the
internal uteke provider patch series; artifacts stay out of the product repo per
the #1230/#1235 governance. Answerer `claude-sonnet-5-5`, judge
`claude-haiku-4-5`, Anthropic OpenAI-compat layer, strict JSON schema, temp 1
(Sonnet 5.x rejects <1), max_tokens 4000. Local box, zero LLM in the retrieval
path; wall ~1.5 h for the full run.

**Result — 1,540/1,540 answerable QA: 80.4% (1,238 correct).**

| Category | n | QA accuracy | retrieval any@5 (ref) |
|---|---|---|---|
| open-domain | 841 | 0.854 | 0.855 |
| single-hop | 282 | 0.770 | 0.894 |
| temporal | 321 | 0.748 | 0.816 |
| multi-hop | 92 | 0.656 | 0.630 |

Per-conversation range 73.8%–95.1% (best conv-30 95.1%, conv-26 92.8%) — no
single-unit anomaly.

**Reading.** Multi-hop and temporal QA land at-or-above their retrieval
any@5 — answers are often recoverable even when not all gold sessions reach
top-5. Single-hop shows the largest QA-vs-retrieval gap (0.770 vs 0.894): most
single-hop questions cite ~2.7 gold sessions and the judge is strict about
enumeration completeness. Not apples-to-apples with any single-conversation or
different-provider runs (e.g. earlier GLM answerer experiments at 86.0–86.8%).

**Adversarial abstention (446 no-evidence QA, excluded above).** Custom pass
reusing the run's store: regex-classified refusals 211/446 (47.3%); judge audit
of 75 sampled answers (seed 42) — 80% regex precision, and 44% of
regex-"concrete" answers were also refusals — yielding a corrected estimate of
**~61% proper abstention (~272/446)**. The remaining ~39% fabricated answers are
the abstention-layer headroom (prompt v2 teaches qualified abstention but does
not enforce it).

**Raw artifacts (committed):** `results/qa-e2e-2026-11-08.jsonl` (per-question
receipts), `results/adversarial-results.json` (446 answers + classification),
`results/adversarial-audit.json` (75 judge audits).

Audit snippet:

```python
import json
rows = [json.loads(l) for l in open("results/qa-e2e-2026-11-08.jsonl")]
print(len(rows), sum(r["correct"] for r in rows) / len(rows))  # 1540 0.804
```

