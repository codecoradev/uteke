# Benchmarks

This directory is the single source of truth for Uteke benchmarks: what we measure,
how, and what the numbers are.

| Directory | Kind | What it measures | Status |
|---|---|---|---|
| [`internal/`](internal/) | Performance | Insert throughput, recall latency, storage footprint (`uteke bench`) | ✅ Maintained — re-verified on v0.17.0 (2026-09-09) |
| [`longmemeval/`](longmemeval/) | External #1 | Retrieval quality on [LongMemEval-S](https://arxiv.org/abs/2410.10813) (500 questions, session-level) | ✅ Validated on v0.16.0 (pure-default run); -M variant **not** measured (transparent) |
| [`locomo/`](locomo/) | External #2 | Retrieval quality on [LoCoMo](https://github.com/snap-research/locomo) (very long multi-session conversations) | 🚧 Planned — adapter + pilot |

Friendly overview page: [docs/benchmarks.md](../docs/benchmarks.md).

## Layout

```
benchmarks/
├── README.md          ← you are here: index & policies
├── internal/          ← `uteke bench` (perf) — methodology + results
├── longmemeval/       ← LongMemEval-S harness
│   ├── README.md      ← dataset, metric definitions, methodology
│   ├── RESULTS.md     ← all published numbers + provenance
│   ├── results/       ← canonical raw artifacts (committed — audit them!)
│   ├── scripts/       ← harness, metrics, Modal fan-out infra
│   └── data/          ← dataset (gitignored; scripts/download_data.sh)
└── locomo/            ← LoCoMo (planned)
```

## Reproduce

```bash
# Internal performance
uteke bench --counts 100,1000,10000 --json

# LongMemEval-S (local small run)
cd benchmarks/longmemeval
./scripts/download_data.sh
python3 scripts/run_eval.py --data data/longmemeval_s_cleaned.json --output results_local --limit 50
python3 scripts/print_metrics.py results_local/retrieval_results.jsonl
```

Full 500-question runs are executed on Modal (deterministic, fan-out over 10 shards,
resume-safe volume) — see `longmemeval/scripts/modal_fanout.py`.

## Artifact policy

- **Canonical raw artifacts are committed** under `longmemeval/results/` so every
  published headline number can be recomputed straight from the repo.
- Datasets (`data/`) and exploratory run outputs (`results_*/`) are **not** committed —
  they are reproducible via download scripts and the Modal volume.
- Claim discipline: every number in RESULTS.md files must trace to a committed artifact
  or a documented Modal volume path. Retrieval runs are deterministic (no LLM in the
  retrieval path), so independent recomputation must match to the last question.
