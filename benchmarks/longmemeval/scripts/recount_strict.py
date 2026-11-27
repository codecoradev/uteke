#!/usr/bin/env python3
"""Count any@5 and strict@5 from a saved per-question ranking file, with the official gold labels.

Why this exists: a project that publishes only recall_any@5 leaves strict
recall_all@5 (every gold session in the top 5) unknown, but its committed ranking
file has everything needed to count it once the gold labels are joined in.

  curl -L -o /tmp/longmemeval_oracle.json \\
    https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_oracle.json
  python3 benchmarks/longmemeval/scripts/recount_strict.py /tmp/longmemeval_oracle.json RANKINGS.jsonl ...

GOLD is longmemeval_oracle.json of the cleaned dataset: it lists `answer_session_ids`
for each of the 500 questions (the same questions as longmemeval_s_cleaned.json).
Each RANKINGS file is JSON lines in one of two shapes, detected per line:

  Uteke raw output   retrieval_results.retrieved_session_ids  (session ids, best first)
  MemPalace output   retrieval_results.ranked_items[].corpus_id

Counts are given for the 470 answerable questions (the official retrieval scorer
leaves out the 30 `_abs` abstention questions), the 30 abstention questions and all 500.
Cross-check used when this was written: the Uteke v0.24.0 raw file gives 462/470
any@5 and 413/470 strict@5, the numbers in its receipt.
"""
import json
import sys


def ranking(rec):
    res = rec["retrieval_results"]
    if "retrieved_session_ids" in res:
        return res["retrieved_session_ids"]
    return [item["corpus_id"] for item in res["ranked_items"]]


def count(gold, path):
    rows = {}
    for line in open(path):
        rec = json.loads(line)
        rows[rec["question_id"]] = ranking(rec)[:5]
    missing = [q for q in gold if q not in rows]
    if missing:
        sys.exit(f"{path}: no ranking for {len(missing)} questions, e.g. {missing[:3]}")
    out = {}
    for label, keep in (
        ("470 answerable", lambda q: not q.endswith("_abs")),
        ("30 abstention", lambda q: q.endswith("_abs")),
        ("500 all", lambda q: True),
    ):
        qs = [q for q in gold if keep(q)]
        any5 = sum(bool(gold[q] & set(rows[q])) for q in qs)
        strict = sum(gold[q] <= set(rows[q]) for q in qs)
        out[label] = (len(qs), any5, strict)
    return out


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    gold = {e["question_id"]: set(e["answer_session_ids"]) for e in json.load(open(sys.argv[1]))}
    print(f"gold labels for {len(gold)} questions")
    for path in sys.argv[2:]:
        print(path)
        for label, (n, a, s) in count(gold, path).items():
            print(f"  {label:15} n={n:3}  any@5 {a:3}/{n} = {100*a/n:5.1f}%   strict@5 {s:3}/{n} = {100*s/n:5.1f}%")


if __name__ == "__main__":
    main()
