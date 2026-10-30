# Benchmark receipts

Every benchmark number we publish (README, blog posts, docs) has a **receipts
directory**: enough to recompute the number and to see what it does and does not
claim. The README benchmark section links to it, and `check_receipts.py` fails CI
when a headline number has no receipt.

```
benchmarks/<benchmark>/receipts/<release>-<strategy>-<questions>/
├── manifest.json   commit (measured SHA, dirty state, tag), dependencies, dataset
│                   sha256 + revision, raw-scores file + sha256, recompute command,
│                   and the headline claims with a pointer into summary.json
├── summary.json    the metrics, recomputed from the raw scores, one block per basis
└── SCOPE.md        what the metric is and is not
```

The raw per-question scores (NDJSON) stay where they already are
(`longmemeval/results/`) and the manifest names them with their sha256, so there is
one copy of each artifact.

## Rules

- **One basis per table.** `full500`, `answerable` (470) and `abstention` (30) are
  separate blocks in `summary.json` and are never mixed in one table.
- **Say what you did not record.** A field that was not captured at run time (a
  dataset revision, a dependency list) says so in the manifest; it is not filled in
  afterwards from memory.
- **Receipts do not approve claims.** A headline number stays an owner-approved claim;
  the receipt makes it checkable.
- **No receipt, no headline.** Changing a README headline without a receipt that says
  the same number fails `check_receipts.py`.

## Adding a receipt

1. Copy `longmemeval/receipts/TEMPLATE/` to the new run's directory.
2. Fill `manifest.json` (every field; "not recorded" is an allowed value for a missing
   fact, an empty one is not).
3. Generate the summary:
   `python3 benchmarks/scripts/score_longmemeval.py --data <dataset> --results <raw.jsonl> --write <dir>/summary.json`
4. Set each `headline_claims` value to what `summary.json` says.
5. Link the directory from the README benchmark section and run
   `python3 benchmarks/scripts/check_receipts.py`.

Backfilled example: [`longmemeval/receipts/v0.19.0-default-500q/`](longmemeval/receipts/v0.19.0-default-500q/).
