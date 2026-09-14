#!/usr/bin/env python3
"""
Print retrieval metrics from LongMemEval evaluation results.

Usage:
    python print_metrics.py results/retrieval_results.jsonl
"""

import json
import sys
from collections import defaultdict

import numpy as np


def main():
    if len(sys.argv) != 2:
        print("Usage: python print_metrics.py <results.jsonl>")
        sys.exit(1)

    in_file = sys.argv[1]
    data = []
    with open(in_file) as f:
        for line in f:
            line = line.strip()
            if line:
                data.append(json.loads(line))

    # Filter out abstention questions
    data = [x for x in data if "_abs" not in x["question_id"]]

    if not data:
        print("No results to display.")
        sys.exit(1)

    # Aggregate by question type
    type_metrics = defaultdict(list)
    for entry in data:
        qtype = entry.get("question_type", "unknown")
        metrics = entry.get("retrieval_results", {}).get("metrics", {})
        type_metrics[qtype].append(metrics)

    # Print session-level metrics
    print("=" * 60)
    print("Session-Level Retrieval Metrics")
    print("=" * 60)
    print()

    sess_metrics = ["recall_all@5", "ndcg_any@5", "recall_all@10", "ndcg_any@10"]

    # Overall
    all_sess = [m for entry in data for m in [entry.get("retrieval_results", {}).get("metrics", {}).get("session", {})]]
    print(f"Overall ({len(data)} questions):")
    for name in sess_metrics:
        vals = [s.get(name, 0.0) for s in all_sess]
        if vals:
            print(f"  {name:20s} = {np.mean(vals):.4f}")
    print()

    # Per type
    for qtype, entries in sorted(type_metrics.items()):
        sess_vals = [e.get("session", {}) for e in entries]
        print(f"{qtype} ({len(entries)} questions):")
        for name in sess_metrics:
            vals = [s.get(name, 0.0) for s in sess_vals]
            if vals:
                print(f"  {name:20s} = {np.mean(vals):.4f}")
        print()

    # Print turn-level metrics (extended)
    print("=" * 60)
    print("Turn-Level Retrieval Metrics")
    print("=" * 60)
    print()
    all_turn = [entry.get("retrieval_results", {}).get("metrics", {}).get("turn")
                for entry in data]
    all_turn = [t for t in all_turn if t is not None]  # filter missing turn metrics
    if not all_turn:
        print("(Not measured — this harness indexes sessions, not individual turns.)")
    else:
        turn_metrics = ["recall_all@5", "ndcg_any@5", "recall_all@10", "ndcg_any@10",
                        "recall_all@50", "ndcg_any@50"]
        print(f"Overall ({len(data)} questions):")
        for name in turn_metrics:
            vals = [t.get(name, 0.0) for t in all_turn]
            if vals:
                print(f"  {name:20s} = {np.mean(vals):.4f}")
    print()

    # Summary table for README
    print("=" * 60)
    print("Summary Table (for README)")
    print("=" * 60)
    print()
    print("| Question Type | Count | Recall@5 | Recall@10 | NDCG@5 | NDCG@10 |")
    print("|---------------|-------|----------|-----------|--------|---------|")

    # Overall row
    r5 = np.mean([s.get("recall_all@5", 0.0) for s in all_sess]) if all_sess else 0
    r10 = np.mean([s.get("recall_all@10", 0.0) for s in all_sess]) if all_sess else 0
    n5 = np.mean([s.get("ndcg_any@5", 0.0) for s in all_sess]) if all_sess else 0
    n10 = np.mean([s.get("ndcg_any@10", 0.0) for s in all_sess]) if all_sess else 0
    print(f"| **Overall** | {len(data)} | {r5:.3f} | {r10:.3f} | {n5:.3f} | {n10:.3f} |")

    for qtype, entries in sorted(type_metrics.items()):
        sess_vals = [e.get("session", {}) for e in entries]
        r5 = np.mean([s.get("recall_all@5", 0.0) for s in sess_vals])
        r10 = np.mean([s.get("recall_all@10", 0.0) for s in sess_vals])
        n5 = np.mean([s.get("ndcg_any@5", 0.0) for s in sess_vals])
        n10 = np.mean([s.get("ndcg_any@10", 0.0) for s in sess_vals])
        print(f"| {qtype} | {len(entries)} | {r5:.3f} | {r10:.3f} | {n5:.3f} | {n10:.3f} |")


if __name__ == "__main__":
    main()
