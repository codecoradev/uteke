#!/usr/bin/env python3
"""Pull completed LongMemEval shards from the Modal Volume.

Prints a JSON dict {shard_file: content} on the last line (parsed by
watch_and_pull.py).
"""
import json

import modal

vol = modal.Volume.from_name("uteke-longmemeval", create_if_missing=True)

app = modal.App("lmeval-pull")


@app.function(volumes={"/root/vol": vol}, timeout=120)
def pull(strategy: str) -> dict:
    import pathlib
    base = pathlib.Path("/root/vol") / strategy
    out = {}
    if base.exists():
        for f in sorted(base.glob("shard_*.jsonl")):
            out[f.name] = f.read_text()
    return out


@app.local_entrypoint()
def main(strategy: str = "hybrid"):
    print(json.dumps(pull.remote(strategy)))
