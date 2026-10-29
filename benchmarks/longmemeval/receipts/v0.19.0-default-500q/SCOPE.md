# Scope of the numbers in this receipt

**What it measures.** Session-level retrieval on LongMemEval-S: for each question,
whether the gold evidence *session* appears in the top 5 or 10 sessions that
`uteke` returns from the question's haystack. `recall_any@k` needs one gold session
in the top k; `recall_all@k` (strict) needs every gold session; `coverage@k` is the
mean fraction of gold sessions found.

**What it does not measure.**
- It is **not** question-answering accuracy. No answer is generated and no LLM is in
  the retrieval path; a retrieved session can still fail to yield the right answer.
- It is not turn-level retrieval and not the `-M` (long haystack) variant, which has
  not been measured.
- It is not a latency or throughput number (see `benchmarks/internal/`).

**Bases (never mixed in one table).** `full500` = all 500 questions; `answerable` =
the 470 non-abstention questions; `abstention` = the 30 `_abs` questions. The README
headline uses `full500`.

**Determinism.** Retrieval is deterministic; recomputing `summary.json` from the raw
file must match to the last question. A different CPU architecture can reorder
near-tied rankings (documented in `RESULTS.md`).

**Approval.** A number published from this receipt is still an owner-approved claim;
the receipt makes it checkable, it does not change who approves it.
