# LoCoMo — Very Long-Term Conversational Memory

External retrieval benchmark #2: [LoCoMo](https://github.com/snap-research/locomo)
(Snap Research, ACL 2024) — very long multi-session conversations (avg ~300 turns,
up to ~35 sessions) with QA pairs spanning information extraction, temporal reasoning,
and adversarial unanswerable questions.

**Status: ✅ validated.** Retrieval (v0.17.0, 1,536 QA, full run on Modal) and
end-to-end QA accuracy (1,540 answerable QA, 2026-11-08 and 2026-11-09) are both
measured, with the raw per-question artifacts committed in [`results/`](results/).
All numbers, scope notes and audit snippets live in [`RESULTS.md`](RESULTS.md); this
file does not repeat them.

| Claim | Where | Judge needed |
|---|---|---|
| Retrieval recall (any@5 0.840, strict@5 0.716, fusion default) | [`RESULTS.md`](RESULTS.md#headline-fusion-default-v0170) | no |
| End-to-end QA accuracy (canonical 72.2%, sonnet-5-5 answerer and judge) | [`RESULTS.md`](RESULTS.md#end-to-end-qa-canonical-run-2026-11-09-sonnet-5-5-answerer-and-judge) | yes |
| Adversarial abstention (~61%, estimated) | [`RESULTS.md`](RESULTS.md#end-to-end-qa-accuracy-answer--judge) | yes |

Retrieval recall and QA accuracy are different claims and are reported separately.
QA accuracy depends on the LLM answerer and judge: the same answers scored 8.2 points
apart (80.4% vs 72.2%) when only the judge changed, so no single QA number is quoted
outside `RESULTS.md` (policy: [`docs/benchmark-positioning.md`](../../docs/benchmark-positioning.md)).

## Why a second external benchmark

LongMemEval-S alone is a single point of evidence. LoCoMo is the other benchmark the
memory-engine ecosystem publishes on (mem0, Zep, and friends), so reporting both makes
uteke's numbers comparable with the field — and keeps us honest when a claim only holds
on one dataset.

## How it is run

1. **Adapter** — [`scripts/convert_locomo.py`](scripts/convert_locomo.py) maps LoCoMo
   conversations to the same session-insert + recall pipeline used by the LongMemEval
   harness (`../longmemeval/scripts/run_eval.py`): each LoCoMo session becomes a uteke
   memory; QA `evidence` lists become gold session sets. Same metric family
   (recall_any@k / recall_all@k / coverage@k / NDCG@k), so results are comparable
   across benchmarks.
2. **Retrieval run** — full 10 conversations on Modal, CPU-only local ONNX embedding,
   default strategy (zero config), plus vector-only and fts5-only ablations on the
   same data.
3. **End-to-end QA** — an external answer + judge harness layered on top of the
   retrieval output; artifacts stay out of the product repo (#1230/#1235).
4. **Publish** — numbers plus committed raw artifacts in this directory, same audit
   rules as `../longmemeval/` (raw committed, claims traceable).

## Still open

- LoCoMo has no `receipts/` directory in the [`benchmarks/RECEIPTS.md`](../RECEIPTS.md)
  format yet (manifest with measured SHA, dataset sha256, recompute command). That
  format is LongMemEval-only today; it is required before a LoCoMo number becomes a
  README headline, which is deliberately not the case now.
- The -M variant of LongMemEval is also still open, tracked in
  [`../longmemeval/RESULTS.md`](../longmemeval/RESULTS.md).
