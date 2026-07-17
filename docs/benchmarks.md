---
title: Benchmarks
---

# Benchmarks

Real numbers from `uteke bench` on Oracle Cloud ARM (Ampere A1, 4 vCPU, 24GB RAM).
Embedding model: EmbeddingGemma Q4 (768d, ONNX Runtime, CPU-only).

## Results

| Scale | Insert ops/s | Insert Total | Recall Avg | Recall P95 | DB Size | Index Size |
|-------|-------------|-------------|------------|------------|---------|------------|
| 100 memories | 18.5/s | 5.4s | **40ms** | 46ms | 708KB | 319KB |
| 1,000 memories | 21.8/s | 45.9s | **45ms** | 51ms | 5.3MB | 3.2MB |
| 10,000 memories | 6.0/s | 28.0 min | **42ms** | 50ms | 81.3MB | 30.3MB |

## Key Takeaways

### Recall Latency: Flat ~40-45ms from 100 to 10K memories

The killer stat: recall latency barely changes as the store grows.

- 100 memories → **40ms**
- 1,000 memories → **45ms**
- 10,000 memories → **42ms** ← actually *faster* than 1K (warm ONNX cache)

HNSW search is O(log N), so even at 10K memories, the vector index adds <1ms.
The ~40ms floor is dominated by ONNX embedding inference, not search.

The full pipeline:
1. Query → ONNX embedding generation
2. HNSW vector search
3. FTS5 full-text search
4. Reciprocal Rank Fusion (k=60)

No network round-trip. No API call. Everything in-process.

### Insert Throughput: 6-22 ops/s (CPU-bound)

Each insert requires an ONNX embedding pass (CPU inference). Throughput drops at scale because HNSW graph traversal grows as the index expands:

- 100 memories → **18.5 ops/s**
- 1,000 memories → **21.8 ops/s**
- 10,000 memories → **6.0 ops/s**

At 6 ops/s, inserting 10K memories takes ~28 minutes. For bulk ingestion, use `uteke import` (batch mode) which pipelines embeddings.

### Storage Efficiency

- 100 memories → 708KB DB + 319KB index = **~10KB per memory**
- 1,000 memories → 5.3MB DB + 3.2MB index = **~8.5KB per memory**
- 10,000 memories → 81.3MB DB + 30.3MB index = **~11.2KB per memory**

Storage scales linearly (~10KB/memory). SQLite + HNSW both grow predictably.

## How to Reproduce

```bash
uteke bench --counts 100,1000,10000 --json
```

Or with a custom store path:

```bash
uteke bench --counts 100,1000 --store /tmp/bench --json
```

## External Evaluation

See [LongMemEval retrieval harness](https://github.com/codecoradev/uteke/tree/develop/benchmarks/longmemeval) for accuracy evaluation against standard benchmarks.

## Environment

| Component | Details |
|-----------|---------|
| Hardware | Oracle Cloud ARM (Ampere A1, 4 vCPU, 24GB RAM) |
| OS | Linux 6.8.0 (aarch64) |
| Rust | 1.75+ |
| Embedding | EmbeddingGemma Q4, 768d, ONNX Runtime CPU |
| Uteke | v0.7.2 |

## Methodology

The benchmark uses `uteke bench` which:
1. Generates deterministic synthetic memories (seeded PRNG)
2. Inserts them one-by-one with embedding
3. Runs recall queries at each scale
4. Measures wall-clock time for insert and recall
5. Reports ops/s, latency percentiles, and storage footprint

No external services. No network. No Docker. Just the binary.
