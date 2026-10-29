# Internal performance benchmarks

Measures speed (not retrieval quality; that is `benchmarks/longmemeval/`).

| Script | What it measures |
|---|---|
| `uteke bench` (CLI) | insert throughput, recall latency, storage footprint at several store sizes |
| `scripts/serve_load.py` | `uteke-serve` as a black box under concurrent load: throughput and p50/p95/p99 of `/recall` and `/remember` per client count (stdlib only) |
| `scripts/modal_perf.py` | both of the above for one exact commit, on Modal, with the real ONNX model, and a result file with a manifest |

## Why on Modal

The local machine may not have the ONNX runtime, and `uteke bench` uses the built-in embedder, not the configured one. The Modal image builds `uteke` and `uteke-serve` from an exact SHA, reuses the ONNX runtime library of a release, and bakes the embedding model in, so a number for a commit is reproducible and comparable with another commit.

```bash
cd benchmarks/internal/scripts
UTEKE_GIT_REF=<exact sha> UTEKE_RELEASE=v0.21.1 \
LMEVAL_MODEL_DIR=$HOME/.codecora/uteke/models/embeddinggemma-q4 \
modal run modal_perf.py --clients 1,4,16 --duration 20 --counts 100,1000 --seed 200 --tag baseline
```

The result is written to `results/perf-<sha7>[-tag]-<timestamp>.json`: a `manifest` (commit, ONNX runtime release, spec, Modal CPU request, timestamp) and the `results`. Each client level runs on a fresh store. To compare two commits, run it for both with the same flags and compare the files.

## Reading the numbers

- `nproc` in a result is what the container sees (the host's cores); the Modal reservation is `cpu=2`, a request, not a cap. Compare runs of the same script, not against other machines.
- The server serializes requests behind one lock today (#1518). Throughput that stays flat while latency grows with the client count is that lock: more clients queue instead of running in parallel.
- Run each configuration more than once before drawing a conclusion from a difference of a few percent.

## Local use of the load generator

`serve_load.py` works against any running server (local, Docker, remote with `--token`); it seeds the store first, so point it at a throwaway store.

## Receipts

| File | Commit | What it shows |
|---|---|---|
| `results/perf-aa75441-baseline-20261029T013121Z.json` | `aa75441` (develop, 2026-10-29) | Baseline for #1518 and #1522. `uteke-serve`, 2 vCPU: recall about 32 to 36 ops/s and remember about 13 to 16 ops/s at 1, 4 and 16 clients (throughput does not grow), while p50 latency grows with the client count: recall 20 ms, 85 ms, 301 ms. That is one request at a time behind the server's global lock. |
