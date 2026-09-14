#!/usr/bin/env python3
"""MMR (Maximal Marginal Relevance) diversity for session ranking (#1120).

Zero-model post-processing: reorders an already-ranked candidate list to
penalise redundancy among top results. Relevance is approximated from the
input rank order (RRF-style 1/(60+rank)); similarity between candidates is
a TF-IDF cosine over their raw texts, computed over the candidate pool
itself — no external model, no runtime dependencies.

Usage:
    from mmr import mmr_rerank
    out = mmr_rerank(ids, texts, lam=0.7)      # MMR active
    out = mmr_rerank(ids, texts, lam=None)     # no-op (flag off)

Unit tests: test_mmr.py
"""
from __future__ import annotations

import math
import re
from collections import Counter

_RRF_K = 60  # same constant as the RRF fusion used elsewhere in the harness
_WORD = re.compile(r"[a-z0-9']+")


def _tokens(text: str) -> list:
    return _WORD.findall(text.lower())


def _tfidf_vectors(texts: list) -> list:
    """TF-IDF vectors over the candidate pool. Pure python, pool-sized idf."""
    tokenized = [_tokens(t) for t in texts]
    n_docs = len(tokenized) or 1
    df = Counter()
    for toks in tokenized:
        df.update(set(toks))
    idf = {w: math.log((n_docs + 1) / (c + 1)) + 1.0 for w, c in df.items()}
    vecs = []
    for toks in tokenized:
        tf = Counter(toks)
        vec = {w: (1.0 + math.log(c)) * idf[w] for w, c in tf.items()}
        norm = math.sqrt(sum(v * v for v in vec.values())) or 1.0
        vecs.append({w: v / norm for w, v in vec.items()})
    return vecs


def _cosine(a: dict, b: dict) -> float:
    if len(b) < len(a):
        a, b = b, a
    return sum(v * b.get(w, 0.0) for w, v in a.items())


def mmr_rerank(session_ids: list, session_texts: list, lam: float | None,
               relevance: list | None = None) -> list:
    """Greedy MMR reorder of session_ids.

    lam=None  → no-op (returns input order; flag off).
    lam=1.0   → pure relevance, reproduces input order.
    lam<1.0   → trades relevance for diversity.

    session_texts[i] is the full text of session_ids[i]. Optional
    `relevance` overrides the default rank-based scores (len must match).
    """
    if lam is None or len(session_ids) < 2:
        return list(session_ids)
    if len(session_ids) != len(session_texts):
        raise ValueError("session_ids/session_texts length mismatch")

    rel = relevance if relevance is not None else [
        1.0 / (_RRF_K + i + 1) for i in range(len(session_ids))
    ]
    # Normalise relevance to [0,1] so it is scale-comparable with the cosine
    # similarity term. Raw RRF scores span ~0.016 with a spread of ~0.002 —
    # without normalisation the diversity term (~0.1-1.0) dominates by ~50x
    # and MMR degenerates into pure diversity (measured: R@5 1.0 -> 0.33
    # on the mini smoke dataset).
    rmin, rmax = min(rel), max(rel)
    if rmax > rmin:
        rel = [(r - rmin) / (rmax - rmin) for r in rel]
    else:
        rel = [1.0] * len(rel)
    vecs = _tfidf_vectors(session_texts)

    selected: list = []
    remaining = list(range(len(session_ids)))
    # Track max similarity of each remaining candidate to the selected set.
    max_sim = {i: 0.0 for i in remaining}

    while remaining:
        best_i = remaining[0]
        best_score = lam * rel[best_i] - (1.0 - lam) * max_sim[best_i]
        for i in remaining[1:]:
            score = lam * rel[i] - (1.0 - lam) * max_sim[i]
            if score > best_score:
                best_i, best_score = i, score
        selected.append(best_i)
        remaining.remove(best_i)
        bv = vecs[best_i]
        for j in remaining:
            s = _cosine(bv, vecs[j])
            if s > max_sim[j]:
                max_sim[j] = s

    return [session_ids[i] for i in selected]
