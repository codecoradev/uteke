#!/usr/bin/env python3
"""Reclassify judge-credited abstentions in the LoCoMo e2e QA receipts.

Convention (an abstention on an ANSWERABLE question counts as WRONG, consistent
with the adversarial analysis, which counts abstention as right only for
unanswerable questions):
- ABSTAIN = the system offers no candidate answer to the question asked (refuses
  or says there is no information, no guess). -> correct=false, `reclassified: true`.
- HEDGED-ANSWER = opens with an uncertainty phrase but THEN offers a candidate,
  guess or inference (incl. "inferred from general knowledge"). The judge credit
  stands (LoCoMo convention): correct stays true, tagged `hedged_answer: true`
  (a separate disclosure bucket).
- BORDERLINE = points at "closest" evidence without committing to it as the
  answer. Not flipped: correct stays true, tagged `borderline: true`; the reasons
  are in abstention_labels.json and the summary reports the headline both ways.

Detection is a deterministic regex on the answer only (TIER-1 = ABSTAIN patterns
in the first LEAD=50 chars; TIER-2 = strong openers in the first LEAD2=140 chars:
"context has no", "can't/cannot confirm", or an answer that STARTS with "The context
only says" and then contains "does not say" within those 140 chars). The regex
only NOMINATES rows; the class of each nominated row comes from the manual labels
in scripts/abstention_labels.json (query_id -> ABSTAIN|HEDGED|BORDERLINE), and the
script asserts the labels cover exactly the nominated rows. Input is never
modified; stdlib only.

Usage: python3 scripts/reclassify_abstention.py   (run from benchmarks/locomo)
"""
import json
import re
from collections import Counter
from pathlib import Path

SRC = Path("results/qa-e2e-2026-11-08.jsonl")
DST = Path("results/qa-e2e-2026-11-08-reclassified.jsonl")
# Matched on the answer's first LEAD chars only: abstentions open the answer; later hedges ("the exact date isn't
# specified") qualify a concrete answer and must not count as abstention. The
# judge_reason is NOT searched: it often quotes "not specified" while praising a
# concrete answer.
LEAD = 50
ABSTAIN = re.compile(
    r"does not say|doesn't say|not mentioned|cannot determine|cannot be determined"
    r"|no information|not specified|unclear from|context does not|context doesn't"
    r"|context contains no|memories don't say|memories do not say|not available in the conversation|cannot answer",
    re.I,
)

LEAD2 = 140
TIER2 = [
    re.compile(r"^(the )?context has no|can'?t confirm|cannot confirm", re.I),
    re.compile(r"^The context only says.*does not say", re.I | re.S),
]


def abstains(ans):
    return bool(ABSTAIN.search(ans[:LEAD])) or any(p.search(ans[:LEAD2]) for p in TIER2)


LAB = json.load(open("scripts/abstention_labels.json"))
LABELS = LAB["labels"]

rows = [json.loads(l) for l in SRC.open()]
nominated = {r["query_id"] for r in rows if r["correct"] and abstains(r["answer"])}
assert nominated == set(LABELS), (nominated ^ set(LABELS))
total, before, after, cls = Counter(), Counter(), Counter(), Counter()
with DST.open("w") as out:
    for r in rows:
        t = r["category_axes"]["Question Type"][0]
        total[t] += 1
        before[t] += r["correct"]
        c = LABELS.get(r["query_id"])
        cls[c] += 1
        if c == "ABSTAIN":
            r["correct"], r["reclassified"] = False, True
        elif c == "HEDGED":
            r["hedged_answer"] = True
        elif c == "BORDERLINE":
            r["borderline"] = True
        after[t] += r["correct"]
        out.write(json.dumps(r, ensure_ascii=False) + "\n")

n, b, a = len(rows), sum(before.values()), sum(after.values())
assert n == 1540 == sum(total.values())
print(f"rows: {n}")
print(f"correct as-run: {b} ({b / n:.1%})")
print(f"ABSTAIN (flipped): {cls['ABSTAIN']}   HEDGED (kept): {cls['HEDGED']}   BORDERLINE (kept): {cls['BORDERLINE']}")
bl = sorted(k for k, v in LABELS.items() if v == "BORDERLINE")
print("BORDERLINE ids:", ", ".join(bl))
print(f"correct fixed:  {a} ({a / n:.1%})   [if BORDERLINE flipped: {a - cls['BORDERLINE']} ({(a - cls['BORDERLINE']) / n:.1%})]")
for t in sorted(total, key=lambda t: -total[t]):
    print(f"  {t:12s} n={total[t]:4d}  as-run {before[t] / total[t]:.1%}  "
          f"corrected {after[t]}/{total[t]} = {after[t] / total[t]:.1%}")
