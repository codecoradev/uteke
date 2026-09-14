# LongMemEval Benchmark Harness for Uteke

This harness evaluates uteke's retrieval quality against the [LongMemEval](https://github.com/xiaowu0162/LongMemEval) benchmark (ICLR 2025), session-level retrieval, no LLM anywhere in the retrieval path (deterministic — an independent recompute of the committed raw artifacts must match to the last question).

**Published numbers: [`RESULTS.md`](RESULTS.md). Canonical raw artifacts: [`results/`](results/).**

## Quick Start

```bash
# 1. Download LongMemEval dataset
./scripts/download_data.sh

# 2. Install Python deps
pip install -r scripts/requirements.txt

# 3. Build uteke (ensure `uteke` binary is on PATH)
cargo build --release

# 4. Run retrieval evaluation (small local run)
python3 scripts/run_eval.py --data data/longmemeval_oracle.json --output results_local --limit 50

# 5. Print metrics
python3 scripts/print_metrics.py results_local/retrieval_results.jsonl
```

Full 500-question runs run on Modal with fan-out over 10 shards (resume-safe volume):
`scripts/modal_fanout.py`. Set `UTEKE_GIT_REF=<exact-sha>` to build the image from a
specific commit for pre-release validation.

## What It Measures

**Retrieval accuracy** — how well does uteke recall the correct evidence sessions?

| Metric | Definition |
|---|---|
| `recall_any@k` | A question passes when **at least one** gold session appears in top-k — the metric competitor benchmarks publish |
| `recall_all@k` (strict) | **Every** gold session must appear in top-k (binary per question). Harder; bounded below `recall_any`. Reported as binary mean and as partial-credit coverage |
| `ndcg_any@k` | Ranking quality over gold sessions |
| `coverage@k` | Partial credit per question (fraction of gold sessions retrieved in top-k) |

> Turn-level metrics require per-turn indexing and are not measured by this harness.

Question types evaluated:

| Type | Description |
|------|-------------|
| `single-session-user` | Single-session user info extraction |
| `single-session-assistant` | Single-session assistant info extraction |
| `single-session-preference` | User preference extraction |
| `multi-session` | Cross-session reasoning |
| `knowledge-update` | Updated information recall |
| `temporal-reasoning` | Time-based reasoning |
| `abstention` | Unanswerable questions — the evidence does **not** exist in the haystack; the metric family is computed per the harness and reported as a separate group |

### The 470 + 30 decomposition

The cleaned -S set has 500 questions: **470 answerable + 30 abstention** (`question_id`
suffix `_abs`). Aggregates in RESULTS.md are labeled explicitly:
`Overall (470 non-abstention)`, `Abstention (30)`, and `Overall (full 500)`. `recall_any`
numbers quoted against competitor figures are full-500; strict-family numbers are reported
on both bases. No questions are silently dropped.

## Dataset

Download via `scripts/download_data.sh` from
[HuggingFace](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned):

- `longmemeval_oracle.json` — oracle retrieval (evidence sessions only, ~5MB), quick iteration
- `longmemeval_s_cleaned.json` — short variant, **what we publish on** (~115k tokens history, 500 questions)
- `longmemeval_m_cleaned.json` — medium variant (~500 sessions, ~200MB)

The -M variant is **unmeasured** — stated transparently rather than extrapolated.

## How It Works

1. For each question: extract `haystack_sessions` (chat history with timestamps), insert each session as a uteke memory (content = session text, timestamp = session date)
2. Run `uteke recall` with the question as query (default strategy — what a fresh install gets, zero config)
3. Check whether gold sessions appear in top-k
4. Report `recall_any/all@k`, `coverage@k`, `ndcg_any@k`

Retrieval uses uteke's built-in EmbeddingGemma Q4 (768d) via local ONNX. No external API,
no network round-trip.

## Layout

```
longmemeval/
├── README.md      ← this file: dataset + metric definitions
├── RESULTS.md     ← all published numbers + provenance
├── results/       ← canonical raw artifacts (committed — recompute them!)
├── scripts/       ← harness, metrics, Modal fan-out infra
└── data/          ← dataset (gitignored; download_data.sh)
```
