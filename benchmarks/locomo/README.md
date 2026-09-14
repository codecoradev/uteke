# LoCoMo — Very Long-Term Conversational Memory (planned)

External retrieval benchmark #2: [LoCoMo](https://github.com/snap-research/locomo)
(Snap Research, ACL 2024) — very long multi-session conversations (avg ~300 turns,
up to ~35 sessions) with QA pairs spanning information extraction, temporal reasoning,
and adversarial unanswerable questions.

**Status: 🚧 planned.** Scope below; numbers land here after the first validated run.

## Why a second external benchmark

LongMemEval-S alone is a single point of evidence. LoCoMo is the other benchmark the
memory-engine ecosystem publishes on (mem0, Zep, and friends), so reporting both makes
uteke's numbers comparable with the field — and keeps us honest when a claim only holds
on one dataset.

## Plan

1. **Adapter** — map LoCoMo conversations to the same session-insert + recall pipeline
   used by the LongMemEval harness (`../longmemeval/scripts/run_eval.py`): each LoCoMo
   session becomes a uteke memory; QA `evidence` lists become gold session sets. Same
   metric family (recall_any@k / recall_all@k / coverage@k / NDCG@k) so results are
   directly comparable across benchmarks.
2. **Pilot** — 2 conversations end-to-end (insert → recall → metrics), CPU-only local
   ONNX embedding, default strategy (zero config), then Modal for the full 10.
3. **Full run + ablations** — default (fusion), vector-only, fts5-only on the same data,
   so the comparison chart gets internal ablation bars on a second dataset.
4. **Publish** — numbers + committed raw artifacts in this directory, same audit rules
   as `../longmemeval/` (raw committed, claims traceable).

## Non-goals (for now)

- LoCoMo end-to-end QA accuracy (LLM judge) — retrieval-layer only, same boundary as
  our LongMemEval reporting. Unmeasured layers are stated as unmeasured.
- -M-variant LongMemEval — also still open, tracked in
  [`../longmemeval/RESULTS.md`](../longmemeval/RESULTS.md).
