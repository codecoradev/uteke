#!/usr/bin/env python3
"""Regenerate the fast-eval datasets from the parent LongMemEval-S dataset.

The fast-eval JSON files (fast50 / temporal15 / multisession15 / fast10) are
derived subsets of ``data/longmemeval_s_cleaned.json`` — each question object
is byte-identical to its parent entry. Committing ~45MB of derived JSON bloats
the repository and triggers secret-scanning false positives (synthetic chat
text occasionally matches token-shaped patterns). Git therefore tracks only
this generator plus the per-file question_id lists; the JSON files themselves
are regenerated locally on demand:

    python3 make_fast_eval.py            # regenerate all four files
    python3 make_fast_eval.py --verify   # check existing files match the lists

The id lists live in fast_eval_ids.json. Selection was stratified per
question_type when first built (2026-08-27); the lists are the source of
truth, so regeneration is fully deterministic — no randomness.

Requires the parent dataset. If missing, run download_data.sh first and fetch
longmemeval_s_cleaned.json manually (see the note in that script).
"""
import argparse
import json
import sys
from pathlib import Path

HERE = Path(__file__).parent.parent  # benchmarks/longmemeval (scripts live in scripts/)
DATA = HERE / "data"
PARENT = DATA / "longmemeval_s_cleaned.json"
IDS = HERE / "fast_eval_ids.json"

# id-list file name -> output dataset file name
NAME_MAP = {
    "fast50": "longmemeval_fast50.json",
    "fast10": "longmemeval_fast10.json",
    "temporal15": "longmemeval_temporal15.json",
    "multisession15": "longmemeval_multisession15.json",
}


def load_parent():
    if not PARENT.exists():
        sys.exit(f"Parent dataset not found: {PARENT}\nRun download_data.sh and fetch longmemeval_s_cleaned.json first (see script notes).")
    with open(PARENT) as f:
        return json.load(f)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--verify", action="store_true", help="verify existing files instead of regenerating")
    args = ap.parse_args()

    with open(IDS) as f:
        id_lists = json.load(f)
    parent = load_parent()
    by_id = {q["question_id"]: q for q in parent}

    ok = True
    for key, fname in NAME_MAP.items():
        ids = id_lists[key]
        missing = [i for i in ids if i not in by_id]
        if missing:
            print(f"[FAIL] {fname}: {len(missing)} ids not in parent (first: {missing[0]})")
            ok = False
            continue
        rebuilt = [by_id[i] for i in ids]
        out_path = DATA / fname
        if args.verify:
            if not out_path.exists():
                print(f"[FAIL] {fname}: missing (run without --verify to regenerate)")
                ok = False
                continue
            with open(out_path) as f:
                cur = json.load(f)
            match = json.dumps(cur) == json.dumps(rebuilt)
            print(f"[{'OK' if match else 'FAIL'}] {fname}: {'identical' if match else 'DIFFERS from id-list rebuild'}")
            ok &= match
        else:
            with open(out_path, "w") as f:
                json.dump(rebuilt, f)
            print(f"[OK] wrote {out_path} ({len(rebuilt)} questions)")

    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
