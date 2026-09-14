#!/usr/bin/env python3
"""Unit tests for mmr.py (#1120). Run: python3 test_mmr.py"""
import sys

from mmr import mmr_rerank, _tfidf_vectors, _cosine

PASS = 0
FAIL = 0


def check(name, cond, detail=""):
    global PASS, FAIL
    if cond:
        PASS += 1
        print(f"  ok  {name}")
    else:
        FAIL += 1
        print(f" FAIL {name} {detail}")


# --- no-op semantics (flag off) ---
ids = ["s1", "s2", "s3"]
texts = ["alpha beta", "gamma delta", "epsilon zeta"]
check("lam=None returns input unchanged", mmr_rerank(ids, texts, None) == ids)
check("lam=1.0 pure relevance keeps order", mmr_rerank(ids, texts, 1.0) == ids)
check("single item passthrough", mmr_rerank(["s1"], ["text"], 0.5) == ["s1"])
check("empty list passthrough", mmr_rerank([], [], 0.5) == [])

# --- diversity behavior ---
# Pool: A and A2 near-duplicates, B distinct. Rank order: A, A2, B.
dup = ["cat sat mat", "cat sat mat cat", "dogs bark loudly"]
out = mmr_rerank(["A", "A2", "B"], dup, lam=0.5)
check("near-duplicate demoted below distinct", out[0] == "A" and out[1] == "B",
      f"got {out}")

# With lam=1.0 the duplicate stays at rank 2 (pure relevance).
out = mmr_rerank(["A", "A2", "B"], dup, lam=1.0)
check("lam=1.0 keeps duplicate at rank 2", out[:2] == ["A", "A2"], f"got {out}")

# --- relevance override respected ---
# Force B's relevance to dominate: even at lam=0.9 B must come first.
rel = [0.01, 0.01, 0.99]
out = mmr_rerank(["A", "A2", "B"], dup, lam=0.9, relevance=rel)
check("relevance override dominates", out[0] == "B", f"got {out}")

# --- similarity helpers ---
v = _tfidf_vectors(["same same same", "same same same"])
check("identical texts cosine=1.0", abs(_cosine(v[0], v[1]) - 1.0) < 1e-9,
      f"got {_cosine(v[0], v[1])}")
v2 = _tfidf_vectors(["cat dog", "bird fish"])
check("disjoint texts cosine=0.0", _cosine(v2[0], v2[1]) == 0.0)

# --- length mismatch raises ---
try:
    mmr_rerank(["A", "B"], ["only one text"], 0.5)
    check("length mismatch raises", False)
except ValueError:
    check("length mismatch raises", True)

# --- deterministic ---
r1 = mmr_rerank(ids, texts, 0.7)
r2 = mmr_rerank(ids, texts, 0.7)
check("deterministic output", r1 == r2)

# --- pool sizes: sanity on a bigger pool ---
big_ids = [f"s{i}" for i in range(30)]
big_texts = [f"doc {i} topic {i % 5}" for i in range(30)]
big_out = mmr_rerank(big_ids, big_texts, 0.6)
check("big pool: permutation valid", sorted(big_out) == sorted(big_ids))
check("big pool: top item is rank-1", big_out[0] == "s0")

print(f"\n{PASS} passed, {FAIL} failure(s)")
sys.exit(1 if FAIL else 0)
