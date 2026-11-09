---
title: Benchmark Positioning & Reporting Policy
---

# Benchmark Positioning & Reporting Policy

> **Status: PROPOSED.** Reporting and positioning policy for benchmark numbers.
> Not normative until approved by the owner. Nothing here changes the README, the
> benchmark pages or the CORE/LAB contract by itself.

## Context

Presentation decisions already locked by the owner (Sep 2026):

- the hero angle is **efficiency**, not a bare "#1";
- the framework is **"publishable if reproducible"**;
- competitor numbers are **always labeled cross-source** and never share a column
  with numbers from our own harness;
- LoCoMo end-to-end (e2e) QA goes to the README only in a no-number format (promoted 2026-11-09, see the gate below).

Two events in Nov 2026 reinforce that policy:

1. **Regression fix (#1690).** After the fix, LongMemEval full-500 was re-validated
   on 2026-11-08: any@5 98.2%, strict@5 identical (88.0%), strict@10 +1 question (95.4%), zero misses@50; four questions shifted partial gold ordering without losing all evidence.
2. **LoCoMo e2e correction.** The first e2e QA run credited abstentions on answerable
   questions. On 2026-11-09 the haiku-judged headline was corrected from 80.4% to
   74.5% (91 rows reclassified). The same day a sonnet-5-5-judged run (same answerer)
   gave **72.2%** (1,112/1,540, 0 reclassified), which the owner made canonical; the
   haiku run is cross-judge reference data (see
   [`benchmarks/locomo/RESULTS.md`](../benchmarks/locomo/RESULTS.md)). The 8.2-point
   as-run gap from swapping only the judge is the evidence behind principle 1.

This document turns those decisions into a reusable policy and lists concrete
recommendations for the owner.

## Principles

### 1. Judge-strictness disclosure

Every e2e QA number must name the judge model and state the rubric's leniency
(strict or lenient, and how abstentions are handled). Judge choice alone moves scores
far more than most system differences: Mnemoverse (Jun 2026) showed that changing only
the judge rubric shifted one system's score from 0.901 to 0.480 on the same answers,
and Mem0's answers from 0.741 to 0.343, a swing of roughly 40 points. A number without
its judge label cannot be compared with anything, including our own earlier runs.

### 2. Cross-source separation

A competitor number is always quoted with its source, date and whether it is
self-reported. It is never placed in the same column as a number from the Uteke
harness and never worded so that it implies the same harness, dataset split, judge or
prompt. Published figures also move: Mem0 reports 92.5 self-reported against 66.9 in
its April 2025 paper, and Zep went from a corrected 75.14 (2025) to 94.7 (Aug 2026).
Without a harness, a vendor number is a claim, not a measurement.

### 3. Retrieval-first honesty

Lead with recall@k. It is judge-free, deterministic given the dataset and binary, and
rarely published by competitors, so it is the axis where a reproducible number is
also a differentiating one. e2e QA is a second layer, reported with the disclosure
from principle 1 and never as the headline.

### 4. Corrections as assets

A correction note is first-class content, kept next to the number it corrects, not
removed or buried. The LoCoMo e2e correction (80.4% to 74.5%, 2026-11-09) is the
reference case: the as-run number stays visible as a disclosure, the reclassified
count is stated, and the corrected number is the one that gets cited (superseded
as canonical by the sonnet-judged 72.2%, with the haiku run kept as reference). A project that
shows how it caught its own error is easier to trust than one that has never needed to.

### 5. Reproducibility

A number is publishable only if its receipts are committed and a reader can reproduce
it from a documented path (for example
[`benchmarks/longmemeval/REPRODUCING.md`](../benchmarks/longmemeval/REPRODUCING.md)).
Binary version and commit, dataset version, run conditions and raw per-question
results go with the number. If a run cannot be reproduced, it is not cited.

### 6. Adversarial and abstention behavior is measured and disclosed

Questions that have no answer in the corpus, and the rate at which the system
correctly abstains, are reported alongside accuracy on answerable questions. A
system that rarely abstains scores well on answerable questions and badly on
adversarial ones; showing only one side hides that trade-off. The abstention figure
is an estimate after correction and is labeled as such.

## Recommendations

| Area | Current status | Recommendation | Owner decision needed? |
|---|---|---|---|
| README hero | Efficiency-led angle locked (Sep 2026) | Keep efficiency as the hero. Do not use a bare "#1" claim. | No (already locked) |
| README stability note | LME full-500 re-validated 2026-11-08 after #1690: any@5 98.2%, strict@5 88.0%, misses@50 0, strict@5 identical, strict@10 +1 (95.4%) | Add a short note: retrieval quality re-validated on the current line, with a link to the receipts. State the date and the comparison baseline. | Yes (wording and placement) |
| LoCoMo e2e in README | README has a LoCoMo section with no number; canonical e2e 72.2% (sonnet-5-5 judged) lives in `benchmarks/locomo/` | **Promoted 2026-11-09, no-number format**: the README explains that e2e QA depends on answerer and judge LLMs and links `benchmarks/locomo/RESULTS.md`. No LoCoMo e2e figure in the README. | No (decided 2026-11-09) |
| Benchmark page | Retrieval and internal benchmarks on `docs/benchmarks.md` | Retrieval recall first, e2e QA second with judge disclosure, abstention and the correction note visible. Link receipts. | Yes (page structure) |
| Blog / announcement angle | Not yet written | Angle: efficiency plus honesty about measurement (including the 80.4 to 74.5 correction and the judge-swap effect). No "#1" claim, no leaderboard framing. | Yes (angle and publish go-ahead) |
| Competitor comparison table | Competitor material lives in `docs/comparison.md` | Any competitor figure is a separate, labeled cross-source table: vendor, figure, source, date, self-reported or not, judge if known. Never in the same column as Uteke harness numbers. | Yes (whether to include one at all) |
| Corrections policy | One correction note on LoCoMo (category mapping, 2026-09-10) and one on e2e (2026-11-09) | Adopt "correction note stays next to the number" as a standing rule for all benchmark files. | Yes (adopt as policy) |

### Gate for LoCoMo e2e in the README

> **PROMOTED 2026-11-09 (owner decision), in a no-number format with an
> LLM-dependence explanation.** The criteria below stay as the bar for ever
> printing a LoCoMo e2e figure in the README; until then the README section
> carries no number.

Conditions for printing a number, all of which would have to hold:

1. **Strict judge.** e2e scoring uses a strict rubric, and the judge model and
   rubric are named next to the number.
2. **Abstention measured.** Abstention behavior on adversarial and answerable
   questions is measured and reported, with credited-abstention cases excluded
   from the accuracy figure.
3. **Clean reproduction.** A second run from the committed receipts and the
   documented path reproduces the headline within a stated tolerance.
4. **Disclosure in place.** The README figure carries the judge label, a
   "not comparable to other vendors' LoCoMo figures" note and a link to the
   correction history.

The tolerance in item 3 and whether any item is missing are open for the owner.

## Uteke figures that may be quoted

Only these figures are verified. Do not quote others without a committed receipt.

| Figure | Value | Notes |
|---|---|---|
| LongMemEval full-500, any@5 | **98.2%** | Re-validated 2026-11-08, receipts committed |
| LongMemEval full-500, strict@5 | **88.0%** | Same run |
| LongMemEval full-500, misses@50 | **0** | Same run; strict@10 +1 vs v0.20.0 (95.4%) |
| LoCoMo retrieval, any@5 | **0.840** | v0.17.0, judge-free |
| LoCoMo retrieval, strict@5 | **0.716** | v0.17.0, judge-free |
| LoCoMo e2e QA (canonical) | **72.2%** (1,112 / 1,540 answerable) | Answerer and judge `claude-sonnet-5-5`, temp 1; 0 reclassified, 27 hedged rows audited and kept. Not for the README. |
| LoCoMo e2e QA (haiku-judged, cross-judge reference) | 74.5% corrected / 80.4% as run | Judge `claude-haiku-4-5`, same answerer; 91 rows reclassified, 13 hedged. Reference only; not comparable to the canonical figure. |
| LoCoMo adversarial abstention | ~61% | Corrected estimate |

Suggested citation form for e2e:

> LoCoMo e2e QA: 72.2% (1,112/1,540 answerable), answerer and judge
> claude-sonnet-5-5 (2026-11-09); 27 hedged rows disclosed. Judge-dependent (the
> same answers scored 80.4% as run under a haiku-4-5 judge); not comparable to
> other vendors' LoCoMo figures.

## Cross-source reference points (not Uteke measurements)

For context only. None of these were produced in the Uteke harness. They illustrate
why e2e figures are not comparable across vendors.

| Item | Figure | Source type |
|---|---|---|
| Mem0, paper | 66.9 (Apr 2025) | arXiv:2504.19413 |
| Mem0, published | 92.5 | Vendor, self-reported |
| Zep, corrected | 75.14 (2025) | Zep blog |
| Zep, later | 94.7 (Aug 2026) | Vendor, self-reported |
| Hindsight | 92 | Neutral AMB harness (vectorize-io leaderboard) |
| Judge-rubric swing | 0.901 to 0.480 (one system); Mem0 answers 0.741 to 0.343 | Mnemoverse (Jun 2026) |

If a comparison table is ever published it must carry these labels, and must not be
ranked against Uteke harness figures.

## Sources

- Mem0 paper, arXiv:2504.19413
- Zep paper, arXiv:2501.13956
- Zep, "lies, damn lies" benchmark post: blog.getzep.com
- Mem0 research page: mem0.ai/research
- AutoMem benchmarks: automem.ai/benchmarks
- Mnemoverse judge-variance docs: mnemoverse.com
- Vectorize AMB leaderboard: vectorize-io

Internal: [`benchmarks/locomo/RESULTS.md`](../benchmarks/locomo/RESULTS.md),
[`benchmarks/longmemeval/RESULTS.md`](../benchmarks/longmemeval/RESULTS.md),
[`benchmarks/longmemeval/REPRODUCING.md`](../benchmarks/longmemeval/REPRODUCING.md),
[`docs/benchmarks.md`](benchmarks.md).
