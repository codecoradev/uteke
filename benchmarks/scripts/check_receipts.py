#!/usr/bin/env python3
"""Fail when a published benchmark number has no receipts (#1468).

    python3 benchmarks/scripts/check_receipts.py

Checks, from the repository root:

1. Every `benchmarks/*/receipts/<run>/manifest.json` (the TEMPLATE excluded) is
   complete: it names a raw-scores file that exists and matches its sha256, a
   summary and a scope file that exist, and a dataset sha256; and each of its
   `headline_claims` equals the value found in its summary.
2. The README benchmark section links to every receipts directory.
3. The README headline (`NN.N% LongMemEval-S recall@5`) equals the
   `recall_any@5_full500` claim of a receipt: changing the number without a
   receipt fails here.

Standard library only. Exit status 1 with one line per problem.
"""
import hashlib
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
REQUIRED = ("benchmark", "release", "commit", "dataset", "raw_scores", "summary", "scope", "headline_claims")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def dig(doc, pointer):
    """`file.json#/a/b` style lookup inside an already loaded summary."""
    node = doc
    for part in pointer.split("/"):
        node = node[part]
    return node


def check_receipt(manifest_path, problems):
    base = manifest_path.parent
    name = str(base.relative_to(ROOT))
    manifest = json.loads(manifest_path.read_text())
    for key in REQUIRED:
        if key not in manifest:
            problems.append(f"{name}: manifest.json has no '{key}'")
    if problems and any(p.startswith(name) for p in problems):
        return None
    if not re.fullmatch(r"[0-9a-f]{64}", manifest["dataset"].get("sha256", "")):
        problems.append(f"{name}: dataset.sha256 is not a sha256")
    raw = (base / manifest["raw_scores"]["file"]).resolve()
    if not raw.is_file():
        problems.append(f"{name}: raw scores file {manifest['raw_scores']['file']} is missing")
    elif sha256(raw) != manifest["raw_scores"]["sha256"]:
        problems.append(f"{name}: raw scores file does not match its sha256")
    for key in ("summary", "scope"):
        if not (base / manifest[key]).is_file():
            problems.append(f"{name}: {key} file {manifest[key]} is missing")
    claims = {}
    summary_file = base / manifest["summary"]
    if summary_file.is_file():
        summary = json.loads(summary_file.read_text())
        for claim in manifest["headline_claims"]:
            pointer = claim["path"].split("#/", 1)[1]
            try:
                found = dig(summary, pointer)
            except (KeyError, TypeError):
                problems.append(f"{name}: claim {claim['id']} points at {claim['path']}, which is not in the summary")
                continue
            if abs(found - claim["value"]) > 1e-9:
                problems.append(f"{name}: claim {claim['id']} says {claim['value']} but the summary has {found}")
            claims[claim["id"]] = claim["value"]
    return name, claims


def benchmark_section(readme):
    start = readme.find("## 📊 Benchmarks")
    if start < 0:
        return None
    end = readme.find("\n## ", start + 1)
    return readme[start : end if end > 0 else len(readme)]


def main():
    problems = []
    receipts = {}
    for manifest in sorted(ROOT.glob("benchmarks/*/receipts/*/manifest.json")):
        if manifest.parent.name == "TEMPLATE":
            continue
        got = check_receipt(manifest, problems)
        if got:
            receipts[got[0]] = got[1]
    if not receipts:
        problems.append("no receipts found under benchmarks/*/receipts/")

    readme = (ROOT / "README.md").read_text()
    section = benchmark_section(readme)
    if section is None:
        problems.append("README.md has no '## 📊 Benchmarks' section")
    else:
        for name in receipts:
            if name not in section:
                problems.append(f"README.md benchmark section does not link {name}/")
    headline = re.search(r"(\d+\.\d)% LongMemEval-S recall@5", readme)
    if headline is None:
        problems.append("README.md has no 'NN.N% LongMemEval-S recall@5' headline to check")
    else:
        shown = headline.group(1)
        backed = {f"{c['recall_any@5_full500'] * 100:.1f}" for c in receipts.values() if "recall_any@5_full500" in c}
        if shown not in backed:
            problems.append(
                f"README.md headline {shown}% LongMemEval-S recall@5 has no receipt "
                f"(receipts say: {', '.join(sorted(backed)) or 'none'})"
            )

    for p in problems:
        print(f"::error::{p}" if len(sys.argv) > 1 and sys.argv[1] == "--github" else f"error: {p}")
    if problems:
        sys.exit(1)
    print(f"ok: {len(receipts)} receipt(s) verified; README headline is backed")


if __name__ == "__main__":
    main()
