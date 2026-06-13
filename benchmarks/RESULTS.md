# Benchmark Results

## Performance Benchmark (`uteke bench`)

Run `uteke bench` to reproduce. Results below are indicative — actual numbers depend on hardware.

| Memories | Insert (ops/sec) | Recall avg (ms) | Recall p95 (ms) | DB size | Index size |
|----------|-----------------|-----------------|-----------------|---------|------------|
| 100      | ~800            | ~0.3            | ~0.5            | ~45 KB  | ~12 KB     |
| 1,000    | ~800            | ~1.2            | ~2.0            | ~450 KB | ~120 KB    |
| 10,000   | ~750            | ~5.0            | ~8.0            | ~4.5 MB | ~1.2 MB    |

Benchmarked on Oracle Cloud ARM (Ampere Altra), CPU-only.

## LongMemEval Retrieval Accuracy

Run `cd benchmarks/longmemeval && ./download_data.sh && python run_eval.py --data data/longmemeval_oracle.json` to reproduce.

### Uteke v0.1.0 (EmbeddingGemma Q4, 768d)

| Question Type | Count | Recall@5 | Recall@10 | NDCG@5 | NDCG@10 |
|---------------|-------|----------|-----------|--------|---------|
| *Pending run* | —     | —        | —         | —      | —       |

### Comparison with Other Memory Systems

| System | LongMemEval Score | Embedding Model | Notes |
|--------|-------------------|-----------------|-------|
| Hindsight | 94.6% | Proprietary | Commercial |
| Mem0 v3 (Pro) | 91.6% | Proprietary | Commercial |
| Mem0 (Free) | 49.0% | Proprietary | Open source |
| **Uteke v0.1.0** | *TBD* | EmbeddingGemma 300M Q4 | Open source, zero-dep |

> **Note**: LongMemEval scores above are answer-correctness scores (using GPT-4o as judge), while uteke's harness measures retrieval accuracy. The two are correlated but not directly comparable. We report retrieval metrics (Recall@k, NDCG@k) for transparency.
