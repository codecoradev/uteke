# LongMemEval-S: like-for-like comparison on the 470 answerable questions

The chart under "Benchmarks" in the README is drawn from this table
(`scripts/make_comparison_chart.mjs`; edit the data there, not the images).
Read 2026-11-27. We did not run any other system's software; where a count is ours,
it is a recount of that project's own saved output.

## What is compared

- **Questions.** The 470 answerable questions of `longmemeval_s_cleaned.json`. The
  official retrieval scorer leaves out the 30 abstention (`_abs`) questions
  ([`print_retrieval_metrics.py`](https://github.com/xiaowu0162/LongMemEval/blob/main/src/evaluation/print_retrieval_metrics.py),
  line 12), so this is the official basis; the README headline of 98.2% is the same Uteke
  run over all 500 and 98.3% is its 470-question figure.
- **Metrics.** `any@5`: at least one gold session in the top 5 (`recall_any`). `strict@5`:
  every gold session in the top 5 (`recall_all`). The official script prints `recall_all`
  and `ndcg_any`, not `recall_any`, so strict is the official headline and any is the
  more lenient variant most projects publish. 65% of questions have more than one gold session.
- **Unit.** Sessions. Uteke, MemPalace and agentmemory return five sessions; gbrain returns
  five *chunks* (the sessions behind them), so its row is not the same unit.
- **No answer is generated.** This is retrieval only, not question-answering accuracy.

## The numbers

| System and mode | any@5 | strict@5 | LLM or paid API in retrieval | Where the count comes from |
|---|---|---|---|---|
| **Uteke v0.24.0** (fusion, local EmbeddingGemma Q4) | 462/470 (98.3%) | 413/470 (87.9%) | none | Our receipt, [`receipts/v0.24.0-default-500q/summary.json`](receipts/v0.24.0-default-500q/summary.json); `recount_strict.py` gives the same |
| MemPalace, raw mode | 454/470 (96.6%) | 403/470 (85.7%) | none | **Our recount** of the ranking file MemPalace committed, `benchmarks/results_mempal_raw_session_20260414_1629.jsonl` |
| agentmemory, BM25 component only | 452/470 (96.2%) | 398/470 (84.7%) | none | Their receipt, `docs/benchmarks/receipts/2026-10-28/longmemeval/summary.json` (index only: no vectors, daemon or graph); not recounted |
| MemPalace, hybrid v4 + LLM rerank | 467/470 (99.4%) | 423/470 (90.0%) | an LLM reranks the top 20 | **Our recount** of `benchmarks/results_mempal_hybrid_v4_llmrerank_session_20260414_1659.jsonl` |
| gbrain, `balanced` + Voyage `rerank-2.5` | 470/470 (100.0%) | 451/470 (96.0%) | OpenAI `text-embedding-3-large` and a Voyage reranker (paid) | [Their report](https://github.com/garrytan/gbrain-evals/blob/main/docs/benchmarks/2026-10-04-longmemeval-opaque-followups.md), run on gbrain `109b992`; not recounted |

Our two recounts agree with the ones gbrain-evals published for the same files (403/470
and 423/470). The script is checked against Uteke's own receipt: it returns 462/470 and
413/470 for the v0.24.0 raw file.

## Reproduce the recounts

```bash
curl -L -o /tmp/longmemeval_oracle.json \
  https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_oracle.json
curl -L -O https://raw.githubusercontent.com/MemPalace/mempalace/HEAD/benchmarks/results_mempal_raw_session_20260414_1629.jsonl
python3 benchmarks/longmemeval/scripts/recount_strict.py /tmp/longmemeval_oracle.json \
  benchmarks/longmemeval/results/default-500q-v0.24.0.jsonl \
  results_mempal_raw_session_20260414_1629.jsonl
```

The gold labels are the `answer_session_ids` of the cleaned dataset; the question ids of the
MemPalace files are the same 500 as ours. Uteke and agentmemory record the same dataset file,
`longmemeval_s_cleaned.json` with sha256 `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`;
gbrain's README shows the prefix `d6f21ea9`; MemPalace's files record no hash.

## What this does not show

- **The LLM and paid-API rows are higher.** MemPalace with an LLM rerank and gbrain beat
  Uteke on strict@5 (90.0% and 96.0% against 87.9%). Uteke's claim is the local,
  no-LLM row: highest of the three in both metrics, not highest overall.
- **None of these is held out.** gbrain says its configuration was chosen on these 470
  questions. Uteke's fusion weights came from a grid sweep over LongMemEval-S rankings
  (the 50-question `fast50` basket in [RESULTS.md](RESULTS.md)), which is part of the 500.
  MemPalace's raw mode has no tuning; its hybrid mode was tuned on 50 questions.
- **Self-reported.** Rows are each project's own output. We did not run their code.
- **agentmemory is a component.** Its full hybrid scored 95.2% any@5 on all 500 in an older run
  its own page labels historical, so it is not in the table.

## Systems we did not put in the chart

- **ai-memory** ([alphaonedev/ai-memory-mcp](https://github.com/alphaonedev/ai-memory-mcp), 0.10.0):
  publishes R@5 on all 500 questions only: 96.4% for its keyword (FTS5) mode in its binary-faithful run
  (v0.7.0) and 97.0% in its README headline, from a different, faster harness; its results page says
  numbers are only comparable within one harness. It does not state which recall variant R@5 is and
  publishes no strict figure, so there is nothing to put in the strict column.
- **ContextFit** reports 87.45% strict and 98.3% any@5 on the 470 with embedding fusion, from its own
  harness and whitepaper; Lethe reports 93.8% R@5 over all 500 without naming the variant
  ([gbrain-evals' table](https://github.com/garrytan/gbrain-evals/blob/main/docs/comparison-systems.md)).
  We did not read their code.
- **Mem0, Zep, Hindsight, Supermemory, Mastra and others** publish answer accuracy with an LLM reader and
  judge. That is a different metric from retrieval recall and cannot be placed on this chart.
