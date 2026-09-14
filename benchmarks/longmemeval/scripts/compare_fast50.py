#!/usr/bin/env python3
"""Compare baseline vs rerank fast-eval results (#1118).

Usage: python3 compare_fast50.py <baseline_dir> <rerank_dir>
"""
import json
import sys
from pathlib import Path
from collections import defaultdict


def load(d):
    rows = {}
    p = Path(d) / "retrieval_results.jsonl"
    for line in p.read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        m = (r.get("retrieval_results", {}) or {}).get("metrics") or {}
        sm = (m.get("session") or {})
        rows[r["question_id"]] = {
            "type": r["question_type"],
            "r5": sm.get("recall_all@5"),
            "r10": sm.get("recall_all@10"),
        }
    return rows


def agg(rows):
    by_type = defaultdict(list)
    for v in rows.values():
        if v["r5"] is not None:
            by_type[v["type"]].append(v["r5"])
    out = {}
    allv = []
    for t, vals in by_type.items():
        out[t] = sum(vals) / len(vals)
        allv += vals
    out["OVERALL"] = sum(allv) / len(allv) if allv else 0.0
    return out


def main():
    base = load(sys.argv[1])
    rerank = load(sys.argv[2])
    a, b = agg(base), agg(rerank)

    print(f"{'type':30s} {'n':>3s} {'baseline':>9s} {'rerank':>9s} {'delta':>8s}")
    types = sorted(set(a) | set(b))
    for t in types:
        d = b.get(t, 0) - a.get(t, 0)
        n = sum(1 for v in rerank.values() if v["type"] == t and v["r5"] is not None)
        flag = " <<<" if d >= 0.03 and t != "OVERALL" else ""
        print(f"{t:30s} {n:3d} {a.get(t, 0):9.4f} {b.get(t, 0):9.4f} {d:+8.4f}{flag}")

    dd = b["OVERALL"] - a["OVERALL"]
    print()
    print(f"OVERALL: {a['OVERALL']:.4f} -> {b['OVERALL']:.4f}  ({dd:+.4f})")
    gate = dd >= 0.03
    print(f"GATE (+3pp): {'PASS ✅' if gate else 'FAIL ❌'}")

    # per-Q movers
    up = sum(1 for q in base if q in rerank and rerank[q]["r5"] is not None
             and base[q]["r5"] is not None and rerank[q]["r5"] > base[q]["r5"])
    down = sum(1 for q in base if q in rerank and rerank[q]["r5"] is not None
               and base[q]["r5"] is not None and rerank[q]["r5"] < base[q]["r5"])
    print(f"Q improved: {up}  Q regressed: {down}")

    return 0 if gate else 1


if __name__ == "__main__":
    sys.exit(main())
