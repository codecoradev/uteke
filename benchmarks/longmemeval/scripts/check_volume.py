#!/usr/bin/env python3
"""Quick check: shard mana saja yang sudah tersimpan di Modal Volume."""
import modal

vol = modal.Volume.from_name("uteke-longmemeval", create_if_missing=True)

app = modal.App("check-lmeval-volume")

@app.function(volumes={"/root/vol": vol}, timeout=120)
def check(strategy: str) -> list:
    import pathlib
    base = pathlib.Path("/root/vol") / strategy
    out = []
    if base.exists():
        for f in sorted(base.glob("shard_*.jsonl")):
            n = sum(1 for l in f.read_text().splitlines() if l.strip())
            out.append((f.name, n))
    return out

@app.local_entrypoint()
def main():
    for strategy in ["hybrid", "vector"]:
        res = check.remote(strategy)
        total = sum(n for _, n in res)
        print(f"{strategy}: {len(res)}/20 shard selesai, total {total} pertanyaan")
        for name, n in res:
            print(f"  {name}: {n} Q")
