# LongMemEval Benchmark Results

**Uteke version:** 0.7.2
**Embedding model:** EmbeddingGemma Q4 (768d, local ONNX)
**Dataset:** `longmemeval_oracle.json` (500 questions, oracle retrieval subset)
**Date:** 2026-07-11

## Validation Run (Diverse Sample — 12 questions, 2 per type)

| Question Type | Count | Recall@5 | Recall@10 | NDCG@5 | NDCG@10 |
|---------------|-------|----------|-----------|--------|---------|
| **Overall** | 12 | **0.958** | **0.958** | **1.000** | **1.000** |
| multi-session | 2 | 1.000 | 1.000 | 1.000 | 1.000 |
| single-session-assistant | 2 | 1.000 | 1.000 | 1.000 | 1.000 |
| single-session-preference | 2 | 1.000 | 1.000 | 1.000 | 1.000 |
| single-session-user | 2 | 1.000 | 1.000 | 1.000 | 1.000 |
| temporal-reasoning | 2 | 1.000 | 1.000 | 1.000 | 1.000 |
| knowledge-update | 2 | 0.750 | 0.750 | 1.000 | 1.000 |

## Quick Test (5 questions, temporal-reasoning only)

| Metric | Score |
|--------|-------|
| Recall@5 | 0.933 |
| NDCG@5 | 1.000 |

## Notes

- **Knowledge-update** is the hardest question type (75%) — expected, since it tests recall of information that changed over time.
- **Session-level metrics only.** Turn-level retrieval requires per-turn indexing (not measured by this harness).
- Full 500-question run in progress; results will be added when complete.
