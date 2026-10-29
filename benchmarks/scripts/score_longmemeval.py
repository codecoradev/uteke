#!/usr/bin/env python3
"""Recompute the LongMemEval-S headline numbers from a raw results file.

    score_longmemeval.py --data data/longmemeval_s_cleaned.json \
        --results results/default-500q-v0.19.0.jsonl [--write summary.json]

Reads the dataset (gold session ids) and the raw per-question rankings
(`retrieved_session_ids`, best first) and prints the session-level metrics for
each basis separately. Bases are never mixed:

  full500      all 500 questions
  answerable   the 470 questions that are not abstention (`_abs` ids)
  abstention   the 30 abstention questions

Deterministic, standard library only. Definitions:
  recall_any@k    a gold session is in the top k
  recall_all@k    every gold session is in the top k (strict)
  coverage@k      mean fraction of the gold sessions in the top k
  miss@50         questions with no gold session in the top 50 (a count)
"""
import argparse
import json
import sys

KS = (5, 10)


def per_question(gold, ranked):
    row = {}
    for k in KS:
        top = set(ranked[:k])
        hit = [g in top for g in gold]
        row[f"recall_any@{k}"] = 1.0 if any(hit) else 0.0
        row[f"recall_all@{k}"] = 1.0 if all(hit) else 0.0
        row[f"coverage@{k}"] = sum(hit) / len(hit)
    row["miss@50"] = 0.0 if any(g in set(ranked[:50]) for g in gold) else 1.0
    return row


def summarise(rows):
    n = len(rows)
    out = {"n": n}
    for key in sorted(rows[0]):
        total = sum(r[key] for r in rows)
        out[key] = int(total) if key == "miss@50" else round(total / n, 4)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--data", required=True)
    ap.add_argument("--results", required=True)
    ap.add_argument("--write", help="write the summary JSON here")
    args = ap.parse_args()

    with open(args.data) as f:
        gold = {q["question_id"]: q["answer_session_ids"] for q in json.load(f)}
    rows = {}
    with open(args.results) as f:
        for line in f:
            d = json.loads(line)
            qid = d["question_id"]
            if qid not in gold:
                sys.exit(f"question {qid} is not in the dataset")
            rows[qid] = per_question(gold[qid], d["retrieval_results"]["retrieved_session_ids"])
    missing = set(gold) - set(rows)
    if missing:
        sys.exit(f"{len(missing)} dataset questions have no result")

    summary = {
        "full500": summarise(list(rows.values())),
        "answerable": summarise([r for q, r in rows.items() if not q.endswith("_abs")]),
        "abstention": summarise([r for q, r in rows.items() if q.endswith("_abs")]),
    }
    text = json.dumps(summary, indent=2, sort_keys=True) + "\n"
    if args.write:
        with open(args.write, "w") as f:
            f.write(text)
    print(text, end="")


if __name__ == "__main__":
    main()
