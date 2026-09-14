#!/usr/bin/env python3
"""Pull fast-eval shards from Modal Volume uteke-lmeval-fast → local merged jsonl.

Usage: modal run pull_fast.py --out results_fast50_baseline
Writes: <out>/retrieval_results.jsonl (merged, dedup by question_id)
Prints: single summary line (shards, questions).
"""
import json
import pathlib
import modal

vol = modal.Volume.from_name("uteke-lmeval-fast", create_if_missing=True)

app = modal.App("uteke-lmeval-pull-fast")


@app.function(volumes={"/root/vol": vol}, timeout=300)
def pull(strategy: str) -> dict:
    base = pathlib.Path("/root/vol") / strategy
    out = {}
    if base.exists():
        for f in sorted(base.glob("shard_*.jsonl")):
            out[f.name] = f.read_text()
    vol.commit()
    return out


@app.local_entrypoint()
def main(out: str = "results_fast50_baseline", strategy: str = "hybrid"):
    shards = pull.remote(strategy)
    seen, rows = set(), []
    for fname in sorted(shards):
        for line in shards[fname].splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                qid = json.loads(line).get("question_id")
            except json.JSONDecodeError:
                continue
            if qid and qid not in seen:
                seen.add(qid)
                rows.append(line)
    p = pathlib.Path(out) / "retrieval_results.jsonl"
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text("\n".join(rows) + ("\n" if rows else ""))
    print(f"PULL_FAST: {len(shards)} shards, {len(rows)} Q → {p}")
