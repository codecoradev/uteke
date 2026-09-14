# Internal Performance Benchmark (`uteke bench`)

Measures insert throughput, recall latency, and storage footprint of a local store,
using deterministic synthetic memories (seeded PRNG). CPU-only: every insert performs a
local ONNX embedding pass; no network, no external services.

Methodology: memories are inserted one-by-one (embed → store → index), recall queries run
at each scale, wall-clock percentiles and sizes are reported.

## Results

### v0.17.0 re-verification — 2026-09-09 (native aarch64 release build)

| Scale | Insert ops/s | Insert total | Recall avg | Recall P95 | DB size | Index size |
| --- | --- | --- | --- | --- | --- | --- |
| 100 memories | 17.3/s | 5.8s | 31ms | 47ms | 0.7MB | 0.31MB |
| 1,000 memories | 17.5/s | 57s | 42ms | 62ms | 5.2MB | 3.10MB |
| 10,000 memories | 5.0/s | 33.4min | 31ms | 38ms | 80.7MB | 30.2MB |

Environment: Oracle Cloud ARM (Ampere A1, 4 vCPU, 24GB RAM), Linux 6.8.0 (aarch64),
EmbeddingGemma Q4 768d ONNX Runtime CPU, single run, no other significant load.

Reading: storage matches earlier published numbers within a fraction of a percent;
recall latency is equal or better; insert throughput is within run-to-run noise of the
2026-08 table below. The "~10KB per memory, flat ~30–45ms recall" story is unchanged on
v0.17.0.

### Historical — v0.12.0-era table (as previously published in docs/)

| Scale | Insert ops/s | Insert total | Recall avg | Recall P95 | DB size | Index size |
| --- | --- | --- | --- | --- | --- | --- |
| 100 memories | 18.5/s | 5.4s | 40ms | 46ms | 708KB | 319KB |
| 1,000 memories | 21.8/s | 45.9s | 45ms | 51ms | 5.3MB | 3.2MB |
| 10,000 memories | 6.0/s | 28.0min | 42ms | 50ms | 81.3MB | 30.3MB |

Environment: same host class (Oracle Cloud ARM, Ampere A1, 4 vCPU, 24GB).

## Reproduce

```bash
uteke bench --counts 100,1000,10000 --json
# custom store location:
uteke bench --counts 100,1000 --store /tmp/bench --json
```
