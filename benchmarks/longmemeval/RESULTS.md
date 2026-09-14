# LongMemEval-S Benchmark Results

**Dataset:** `longmemeval_s_cleaned.json` — 500 questions = 470 answerable + 30 abstention (`_abs`), 6+1 question types
**Metric:** Session-level retrieval (Recall@k, NDCG@k). Deterministic — no LLM in the retrieval path.

**TL;DR headline (v0.17.0 re-validation, full 500 questions — see section below):**
recall_any@5 = **0.984** · strict recall_all@5 = **0.880** · strict recall_all@10 = **0.954** · coverage@5 = **0.944**
(Original v0.16.0 validation run: 0.982 / 0.880 / 0.954 / 0.943 — raw committed alongside the re-validation run.)

Every number below can be recomputed from the committed raw artifacts in
[`results/`](results/) — instructions at the bottom. See also the
[metric definitions](README.md#what-it-measures).

---

## Pure-default run — 500 questions, Modal x86_64, binary v0.16.0 from git `bfbc296` (2026-08-29)

Pre-release validation of the shipped default: `--strategy default` (no flag at all)
against the full 500-question dataset. Binary built in-image from exact SHA `bfbc296`
(PR #1137/#1138); the image build prints `uteke 0.16.0`.

Raw artifact (committed): [`results/default-500q.jsonl`](results/default-500q.jsonl).

| Question type | n | R@5 (strict) | R@10 (strict) | any@5 | any@10 | NDCG@5 |
|---|---|---|---|---|---|---|
| knowledge-update | 72 | 1.000 | 1.000 | 1.000 | 1.000 | 0.972 |
| single-session-assistant | 56 | 0.982 | 1.000 | 0.982 | 1.000 | 0.959 |
| single-session-user | 64 | 0.969 | 0.969 | 0.969 | 0.969 | 0.913 |
| single-session-preference | 30 | 0.967 | 0.967 | 0.967 | 0.967 | 0.839 |
| temporal-reasoning | 127 | 0.920 | 0.962 | 0.984 | 0.992 | 0.849 |
| multi-session | 121 | 0.906 | 0.976 | 0.983 | 0.992 | 0.880 |
| abstention | 30 | 0.906 | 0.950 | 0.967 | 0.967 | 0.844 |
| **Overall (470 non-abstention)** | 470 | **0.946** | 0.977 | 0.983 | 0.989 | 0.897 |
| **Abstention (30)** | 30 | **0.906** | 0.950 | 0.967 | 0.967 | 0.844 |
| **Overall (full 500)** | 500 | **0.943** | 0.975 | **0.982** | 0.988 | 0.894 |

Strict-family headline on the full 500: recall_all@5 = **0.880** (binary), coverage@5 = **0.943**
(partial credit). Mathematical ceiling for strict@5 is **0.994** — 3 questions have more than
5 gold sessions.

### Baselines & deltas (corrected labels, 2026-09-09)

- **Vector-only baseline (full 500, 2026-08-13):** R@5 = 0.854 / R@10 = 0.885 / NDCG@5 = 0.810.
  Raw artifact lives on the benchmark Modal volume (`results_vector`), not committed.
  Fusion default vs this baseline: **+8.9pp on the same full-500 basis** (0.854 → 0.943),
  **+9.2pp** against the 470-non-abstention aggregate (0.854 → 0.946) — zero configuration.
- **Correction note:** this baseline was previously labeled "v0.15.0 hybrid" in this file.
  The 0.854/0.885/2026-08-13 figures are the **vector-only** run (see Strategy Comparison
  below). A hybrid 500-question run has never been executed; hybrid is validated at 50
  questions (0.980 R@5). The vector-only comparison is the honest zero-config story:
  fusion adds the lexical channel's wins on top of semantic-only retrieval.

### Where the remaining failures live (structural facts)

- Ranking-side optimization is **closed**: fusion is shipped, 3-way RRF is a measured NO-GO
  (see below), and the weight sweep sits mid-plateau.
- 51 questions miss partially at @5 (strict family); **44 of them miss exactly one gold
  session** — the lever is insert-side granularity (multi-session grouping/diversity),
  not ranking.
- The two weakest categories (temporal 0.920, multi-session 0.906) are also the two
  largest (53% of questions) — the aggregate is honestly weighted. Counterfactual:
  dropping temporal-reasoning lifts the 470-aggregate by +0.95pp.
- Abstention questions (evidence does not exist in the haystack) are scored by the same
  retrieval-metric family and reported as a separate group; end-to-end QA accuracy
  (reader/answering layer, incl. abstention behavior) is **unmeasured**.

## FTS5-only ablation — 500 questions, Modal x86_64 (2026-08-29)

Raw artifact (committed): [`results/fts5-500q.jsonl`](results/fts5-500q.jsonl).

| Metric | Value |
|---|---|
| recall_any@5 | **0.914** |
| coverage@5 | 0.821 |

Lexical-only retrieval is strong (BM25/FTS5 phrasing matches) but fails hardest on
temporal-reasoning and multi-session — the exact classes fusion's vector channel covers.

## Independent Reproduction (2026-09-01)

The published 500Q run was produced on Modal x86. To test whether the result depends on that infrastructure, a 108-question subset (pref: 30, kupd: 78) was re-run locally on a 4-core ARM desktop (Oracle Ampere A1, aarch64) — same v0.16.0 binary, same harness (`scripts/run_eval.py`), different CPU architecture.

| Subset | Questions | Identical per-question rankings | R@5 published | R@5 re-run |
|---|---|---|---|---|
| pref | 30 | 30/30 | 96.7% | 96.7% |
| kupd | 78 | 77/78 | 100.0% | 99.4% |
| **Total** | **108** | **107/108** | — | — |

**The one divergence** (question `0977f2af`, knowledge-update, 2 gold sessions): both runs retrieved the identical top-10 session set; one gold session sits at rank 5 (published) vs rank 6 (re-run), moving that question's recall_all@5 from 1.0 to 0.5. The harness persists rankings rather than raw scores, so the exact score gap cannot be shown, but the identical top-10 set identifies this as cross-architecture floating-point noise on an RRF near-tie, not a retrieval failure. R@10 = 1.0 in both runs.

**Reproduce:**

```bash
python3 scripts/run_eval.py --data data/subset_kupd.json --output results_rerun --strategy default --resume
```

Raw artifacts are kept on the benchmark Modal volume (`uteke-longmemeval`, `default/` and rerun prefixes), consistent with the published run — datasets and exploratory result files are not committed to git (see `.gitignore` here; `scripts/download_data.sh` fetches the dataset). **Canonical published artifacts ARE committed** under `results/`.

## v0.17.0 re-validation — ✅ DONE (2026-09-09)

Full 500-question re-run on the v0.17.0 binary (built in-image from exact release SHA
`a2ec81a` via `UTEKE_GIT_REF`, same 10-shard Modal fan-out, shards isolated under
`default-v017/` on the volume so the v0.16.0 artifacts could not be resume-skipped).

Raw artifact (committed): [`results/default-500q-v017.jsonl`](results/default-500q-v017.jsonl).

| Metric | v0.16.0 | v0.17.0 | Δ |
|---|---|---|---|
| strict recall_all@5 | 0.8800 | **0.8800** | ±0.00pp |
| strict recall_all@10 | 0.9540 | **0.9540** | ±0.00pp |
| coverage@5 | 0.9433 | 0.9443 | +0.09pp |
| recall_any@5 | 0.9820 | **0.9840** | +0.20pp |
| recall_any@10 | 0.9880 | 0.9880 | ±0.00pp |

Per-question comparison (n=500): 238 identical rankings, 254 reorder-only (identical
top-k sets, near-tie swaps), **8** questions with a different top-10 set — of the 3
questions whose strict@5 changed, all stayed within partial credit (none flipped
perfect↔imperfect). The retrieval scoring path is confirmed unchanged in 0.17.0: the
headline numbers above are now **re-validated on v0.17.0**, not merely attributed to
the v0.16.0 run.

```bash
# reproduce
UTEKE_GIT_REF=a2ec81a0915a242cee6e1de8811491e4fad1d4da modal run scripts/modal_fanout.py --strategy default --tag v017
```

## Strategy Comparison (historical runs)

### Vector (semantic only)

| Questions | R@5 | R@10 | R@50 | NDCG@5 | NDCG@10 | Embedding | Date |
|-----------|-----|------|------|--------|---------|-----------|------|
| 500 | 85.4% | 88.5% | — | 0.810 | — | EmbeddingGemma Q4 (ONNX local) | 2026-08-13 |

### Hybrid (RRF: vector + FTS5 fusion)

| Questions | R@5 | R@10 | R@50 | NDCG@5 | NDCG@10 | Embedding | Date |
|-----------|-----|------|------|--------|---------|-----------|------|
| 50 | 98.0% | 100.0% | 100.0% | 0.960 | 0.967 | EmbeddingGemma 768d (API) | 2026-08-13 |

**Improvement (Hybrid vs Vector, 50Q): +12.6pp R@5**

### Fusion (weighted RRF of vector + hybrid rankings, #1123) — default since 0.16.0

| Questions | R@5 | R@10 | NDCG@5 | NDCG@10 | Embedding | Date |
|-----------|-----|------|--------|---------|-----------|------|
| 50 (mean) | 98.0% | 100.0% | 0.893 | 0.901 | EmbeddingGemma Q4 (ONNX local) | 2026-08-28 |
| 50 (43 evaluable*) | 97.7% | 100.0% | 0.893 | 0.901 | EmbeddingGemma Q4 (ONNX local) | 2026-08-28 |

*print_metrics.py filters to 43 evaluable questions; mean over all 50 is the comparable figure. Run: `results_modal_vector_fusion17` (Modal x86, harness-level fusion). Remaining fails @5: 3 questions (92a0aa75, 60472f9c, a3838d2b) — all partial-recall, R@10 = 1.0.

#### Zero-config validation (local ARM, 0.16.0 binary, no flags)

| Check | Result |
|-------|--------|
| Parity: implicit default vs explicit `--strategy fusion` | **identical rankings** (top-10 equal) |
| Divergence: implicit default vs explicit `--strategy hybrid` | rankings differ (as expected) |
| fast10, `--strategy default` (flag omitted → core default) | **R@5 0.9667, R@10 1.0, NDCG@5 0.9643** |

fast10 detail: 1 partial miss @5 (60472f9c multi-session, R@5 0.67 → R@10 1.0) — same known hard question as the Modal x86 fusion run. A fresh install with zero config gets benchmark-grade recall out of the box.

**Why fusion works:** vector-only and hybrid-only fail on DISJOINT question sets (fast50: 7 questions vector wins, 5 hybrid wins, no overlap in failures). RRF fusing both rankings captures each side's wins.

**Tuning evidence:** weight grid sweep on actual x86 rankings; chosen value sits mid-plateau (stable across a range), not at an edge. Weights are internal implementation details (see core crate) rather than part of the public contract.

## 50Q Hybrid — Per Question Type Breakdown

| Question Type | Count | R@5 Hits | R@5 |
|---------------|-------|----------|-----|
| multi-session | 15 | 15 | 100% |
| single-session-user | 7 | 6 | 86% |
| knowledge-update | 7 | 7 | 100% |
| single-session-assistant | 7 | 7 | 100% |
| single-session-preference | 7 | 7 | 100% |
| temporal-reasoning | 7 | 7 | 100% |

**1 miss at R@5** — single-session-user (QID: 5d3d2817). R@10 recovers to 100%.

## 3-way RRF (vec + hyb + fts5-only) — NO-GO (2026-08-29)

Simulation over saved rankings; fts5-only rankings collected on Modal x86
(500Q, ~4.5h, resume-safe via volume). Cross-check: sim fts5-only R@5=0.8211
vs harness-reported 0.820 — parsing validated. (Committed raw: `results/fts5-500q.jsonl`.)

- 2-way shipped fusion (tuned weights) R@5 = 0.9800 (fast50)
- best 3-way (2.0/1.0/0.1) R@5 = 0.9800 — delta +0.0000, **0 flipped questions**
- adding fts5 as third arm changes nothing at any grid weight tried
- fts5-only fails: temporal-reasoning (63) + multi-session (55) dominate —
  same classes already covered (partially) by hybrid's own FTS5 component

Conclusion: ranking-side optimization via more RRF arms is exhausted.
Remaining headroom is insert-side granularity (partial-recall fails) —
separate project, uncertain payoff.

## Contradiction-resolution segment (#1172 Fase 3) — 2026-09-06

**Active-store knowledge-update segment**: 40 topics × (stale fact + winner fact + 3 distractors),
queries ask "which {thing} does {topic} use now?" (semantic, no keyword echo of the answer).
Baseline ranks with BOTH facts active; resolved ranks after `supersede(stale → winner)` —
baseline is measured for every strategy BEFORE any resolution, then the store is resolved once.
Binary: local release build (0.16.0 + #1185 ledger), local ONNX EmbeddingGemma, ARM64.

Harness: `scripts/contradiction_segment.py`. Raw metrics (committed): [`results/contradiction-f3-metrics.json`](results/contradiction-f3-metrics.json).

| Strategy | Stage | winner@1 | winner@5 | winner MRR | stale@1 | stale@5 |
|---|---|---|---|---|---|---|
| fusion (default) | baseline (unresolved) | 0.850 | 1.000 | 0.925 | 0.150 | **1.000** |
| fusion (default) | resolved (superseded) | **1.000** | 1.000 | **1.000** | 0.000 | 0.000 |
| hybrid | baseline | 0.025 | 1.000 | 0.469 | 0.975 | 1.000 |
| hybrid | resolved | 0.225 | 1.000 | 0.588 | 0.000 | 0.000 |
| vector | baseline | 0.950 | 1.000 | 0.975 | 0.050 | 1.000 |
| vector | resolved | 1.000 | 1.000 | 1.000 | 0.000 | 0.000 |

Findings:

- **Unresolved conflicts pollute every strategy's top-5**: with both facts active, the stale
  fact sat in top-5 for 100% of topics on all strategies (hybrid's BM25 even ranks the stale
  fact top-1 for 97.5% of topics — the old fact's "uses X" phrasing matches "use now?"
  queries lexically). After `supersede`, stale@1 and stale@5 drop to **0.000** everywhere
  (deprecated memories are excluded from recall).
- **Supersede lifts the default surface**: fusion winner@1 0.850 → 1.000, MRR 0.925 → 1.000;
  vector 0.950 → 1.000. Hybrid stays weakest on winner@1 (lexical BM25 keeps the new fact's
  "switched to" phrasing behind distractors) but its stale pollution is fully cleared.
- **Ledger integrity**: `contradictions list` listed all 40 resolutions; every stale fact is
  restorable via `contradictions undo` (auditable conflict resolution, #1172 F2).

Interpretation: ranking alone often picks the winner, but only explicit conflict resolution
guarantees stale facts leave the retrieval surface — the difference between "usually right"
(85–95% top-1) and deterministic freshness (100% top-1, zero stale). For agent memory, where
"use now" queries are the norm, resolution is what keeps top-1 trustworthy. This segment is
synthetic and deterministic (fixed topic list); it measures the conflict-resolution pipeline,
not LongMemEval dataset recall.

## Methodology

- **Vector strategy:** Embedding similarity search via usearch index.
- **Hybrid strategy:** Reciprocal Rank Fusion (RRF k=60) of vector search + SQLite FTS5 keyword search.
- **Embedding (vector 500Q):** EmbeddingGemma 300M Q4 ONNX, 768d, local inference.
- **Session-level:** Retrieval evaluated at session granularity (not per-turn).
- **Throttled runs:** 2 CPU cores, nice 19 (production-safe benchmark); Modal fan-out keeps cpu=2 per shard for per-question latency comparability.
- **Determinism:** no LLM in the retrieval path; identical input → identical rankings (see Independent Reproduction).

## Reproduce

```bash
# Vector (500Q, ONNX local)
python3 scripts/run_eval.py --data data/longmemeval_s_cleaned.json --output results_vector --strategy vector

# Pure-default full run on Modal (10 shards, resume-safe)
UTEKE_GIT_REF=<exact-sha> modal run scripts/modal_fanout.py

# Aggregates (per-type + 470/30/full-500 decomposition)
python3 scripts/print_metrics.py results/default-500q.jsonl
```

## Audit the headline numbers from the committed raw artifacts

```python
import json
entries = [json.loads(l) for l in open("results/default-500q.jsonl")]
s = [e["retrieval_results"]["metrics"]["session"] for e in entries]
print("strict@5  :", sum(m["recall_all@5"] == 1.0 for m in s) / len(s))   # 0.880
print("coverage@5:", sum(m["recall_all@5"] for m in s) / len(s))          # 0.943
```

`recall_any@k` recomputes from `retrieved_session_ids` vs the dataset's
`answer_session_ids` (0.982 @5 over the full 500). For the 470/30 split, group by the
`_abs` suffix in `question_id`.
