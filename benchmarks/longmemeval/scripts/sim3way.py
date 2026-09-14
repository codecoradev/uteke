#!/usr/bin/env python3
"""3-way RRF simulation: vector + hybrid + fts5-only rankings.

Offline replay over saved rankings (fast50, Modal x86) — no uteke calls, no
embedding burns. Question: does adding fts5-only as a third RRF arm lift
R@5 over the shipped 2-way fusion (vec*1.7 + hyb*1, k=60)?

Inputs (relative to this dir):
  results_modal_vector/retrieval_results.jsonl           — vector-only rankings
  results_modal_hybrid_fresh_post1130/...jsonl            — hybrid rankings
  results_modal_fts5/retrieval_results.jsonl              — fts5-only rankings
  data/longmemeval_fast50.json                            — ground truth

Usage:
  python3 sim3way.py rankings   # fail-overlap analysis (2-way)
  python3 sim3way.py fusion     # 3-way RRF grid search (weights, k)
"""
import json
import os
import sys
from collections import defaultdict

BASE = os.path.dirname(os.path.abspath(__file__))
K_DEFAULT = 60


def load_jsonl(path):
    with open(path) as f:
        return [json.loads(line) for line in f if line.strip()]


def r_at_k(ids, truth, k):
    if not truth:
        return None
    return len(set(ids[:k]) & truth) / len(truth)


def rrf_fuse(rankings, weights, k=K_DEFAULT, topn=45):
    """Weighted RRF over multiple id lists. Mirrors rrf_fuse_weighted semantics."""
    scores = defaultdict(float)
    for ids, w in zip(rankings, weights):
        for rank, sid in enumerate(ids[:topn]):
            scores[sid] += w / (k + rank + 1)
    return [sid for sid, _ in sorted(scores.items(), key=lambda x: -x[1])]


def evaluate(fused, truth):
    m = {}
    m['recall_all@5'] = r_at_k(fused, truth, 5)
    m['recall_all@10'] = r_at_k(fused, truth, 10)
    return m


def load_all():
    vec = load_jsonl(os.path.join(BASE, 'results_modal_vector', 'retrieval_results.jsonl'))
    hyb = load_jsonl(os.path.join(BASE, 'results_modal_hybrid_fresh_post1130', 'retrieval_results.jsonl'))
    fts = load_jsonl(os.path.join(BASE, 'results_modal_fts5', 'retrieval_results.jsonl'))
    ds = json.load(open(os.path.join(BASE, 'data', 'longmemeval_fast50.json')))
    if isinstance(ds, dict):
        ds = [ds]
    gt = {q['question_id']: set(q['answer_session_ids']) for q in ds}

    by_qid = defaultdict(dict)
    for name, rows in (('vec', vec), ('hyb', hyb), ('fts', fts)):
        for r in rows:
            by_qid[r['question_id']][name] = r['retrieval_results']['retrieved_session_ids']

    out = []
    for qid, d in by_qid.items():
        t = gt.get(qid)
        if not t or len(d) < 3:
            continue
        out.append((qid, t, d['vec'], d['hyb'], d['fts']))
    return out


def mode_rankings():
    """Fail-overlap between the three base rankings."""
    data = load_all()
    print(f'questions with all 3 rankings + truth: {len(data)}')
    # simpler: recompute directly
    res = {}
    for name, idx in (('vec', 2), ('hyb', 3), ('fts', 4)):
        fails = []
        total = 0.0
        for qid, t, *rankings in data:
            r5 = r_at_k(rankings[idx - 2], t, 5)
            assert r5 is not None
            total += r5
            if r5 < 1.0:
                fails.append((qid[:12], r5))
        res[name] = (total / len(data), fails)
        print(f'{name}: mean R@5 {total / len(data):.4f}, fails {len(fails)}: {fails}')

    v = {q for q, _ in res["vec"][1]}
    h = {q for q, _ in res["hyb"][1]}
    f = {q for q, _ in res["fts"][1]}
    print()
    print(f'vec∩hyb fails: {len(v & h)} {sorted(v & h)}')
    print(f'vec∩fts fails: {len(v & f)} {sorted(v & f)}')
    print(f'hyb∩fts fails: {len(h & f)} {sorted(h & f)}')
    print(f'all three fail: {len(v & h & f)} {sorted(v & h & f)}')
    print(f'union of fails: {len(v | h | f)}')


def mode_fusion():
    data = load_all()
    n = len(data)
    print(f'questions: {n}')

    def run(weights, k=K_DEFAULT):
        r5 = r10 = 0.0
        fails = []
        for qid, t, *rk in data:
            fused = rrf_fuse(rk, weights, k)
            m = evaluate(fused, t)
            r5 += m['recall_all@5']
            r10 += m['recall_all@10']
            if m['recall_all@5'] < 1.0:
                fails.append((qid[:12], m['recall_all@5']))
        return r5 / n, r10 / n, fails

    # Baseline: shipped 2-way (vec*1.7, hyb*1)
    b = run((1.7, 1.0, 0.0))
    print(f'2-way shipped (1.7/1.0/0)  R@5 {b[0]:.4f}  R@10 {b[1]:.4f}  fails {len(b[2])}: {b[2]}')

    # 3-way grid: fts weight 0.2–1.0, vec weight around 1.7
    print()
    print('3-way grid (vec / hyb / fts):')
    best = (None, -1)
    for wv in (1.4, 1.7, 2.0):
        for wf in (0.2, 0.4, 0.6, 0.8, 1.0):
            w = (wv, 1.0, wf)
            r5, r10, fails = run(w)
            mark = ''
            if r5 > best[1]:
                best = (w, r5)
                mark = ' <-- best so far'
            print(f'  {wv:.1f}/{1.0:.1f}/{wf:.1f}  R@5 {r5:.4f}  R@10 {r10:.4f}  fails {len(fails)}{mark}')
    print()
    print(f'BEST: weights={best[0]} R@5={best[1]:.4f} (shipped 2-way: {b[0]:.4f}, delta {best[1]-b[0]:+.4f})')


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else 'rankings'
    if mode == 'rankings':
        mode_rankings()
    elif mode == 'fusion':
        mode_fusion()
    else:
        print(f'unknown mode: {mode}', file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
