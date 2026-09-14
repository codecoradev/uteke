#!/usr/bin/env python3
"""Partial preview: 2-way fusion scores from already-collected vec+hyb rankings.

Also joins whatever fts5 shards are already on disk (results_modal_fts5/) for
an early 3-way peek, question-subset only.
"""
import glob
import json
import os
from collections import defaultdict

BASE = os.path.dirname(os.path.abspath(__file__))


def load(p):
    return [json.loads(l) for l in open(p) if l.strip()]


def rrf(rankings, weights, k=60, topn=45):
    s = defaultdict(float)
    for ids, w in zip(rankings, weights):
        for rank, sid in enumerate(ids[:topn]):
            s[sid] += w / (k + rank + 1)
    return [x for x, _ in sorted(s.items(), key=lambda t: -t[1])]


def r_at(ids, t, k):
    return len(set(ids[:k]) & set(t)) / len(t)


def main():
    vec = {r['question_id']: r['retrieval_results']['retrieved_session_ids']
           for r in load(os.path.join(BASE, 'results_modal_vector', 'retrieval_results.jsonl'))}
    hyb = {r['question_id']: r['retrieval_results']['retrieved_session_ids']
           for r in load(os.path.join(BASE, 'results_modal_hybrid_fresh_post1130', 'retrieval_results.jsonl'))}

    fts = {}
    fts_files = sorted(glob.glob(os.path.join(BASE, 'results_modal_fts5_partial/shard_*.jsonl')))
    if not fts_files:
        merged = os.path.join(BASE, 'results_modal_fts5', 'retrieval_results.jsonl')
        if os.path.exists(merged):
            fts_files = [merged]
    for fp in fts_files:
        for r in load(fp):
            fts[r['question_id']] = r['retrieval_results']['retrieved_session_ids']

    with open(os.path.join(BASE, 'data', 'longmemeval_fast50.json')) as f:
        ds = json.load(f)
    gt = {q['question_id']: q['answer_session_ids'] for q in ds}

    qids2 = [q for q in vec if q in hyb and gt.get(q)]
    print(f'== 2-way preview (full fast50, n={len(qids2)}) ==')
    rows = []
    for name, src in [('vector-only', 'v'), ('hybrid-only', 'h')]:
        tot = sum(r_at(vec[q] if src == 'v' else hyb[q], gt[q], 5) for q in qids2)
        rows.append((name, tot / len(qids2)))
    for name, w in [('fusion 1.7/1.0 (shipped)', (1.7, 1.0)), ('fusion 1.0/1.0', (1.0, 1.0))]:
        tot = sum(r_at(rrf([vec[q], hyb[q]], w), gt[q], 5) for q in qids2)
        rows.append((name, tot / len(qids2)))
    for name, v in rows:
        print(f'  {name:26s} R@5 = {v:.4f}')

    qids3 = [q for q in qids2 if q in fts]
    if qids3:
        print(f'\n== 3-way EARLY peek (subset with fts5, n={len(qids3)}/50) ==')
        base2 = sum(r_at(rrf([vec[q], hyb[q]], (1.7, 1.0)), gt[q], 5) for q in qids3)
        print(f'  fusion 1.7/1.0 on subset   R@5 = {base2 / len(qids3):.4f}')
        fts_only = sum(r_at(fts[q], gt[q], 5) for q in qids3)
        print(f'  fts5-only on subset        R@5 = {fts_only / len(qids3):.4f}')
        best = (None, -1.0)
        for wv in (1.4, 1.7, 2.0):
            for wf in (0.2, 0.4, 0.6, 0.8, 1.0):
                w = (wv, 1.0, wf)
                tot = sum(r_at(rrf([vec[q], hyb[q], fts[q]], w), gt[q], 5) for q in qids3)
                if tot > best[1]:
                    best = (w, tot)
        print(f'  best 3-way on subset: w={best[0]} R@5 = {best[1] / len(qids3):.4f} '
              f'(delta vs 2way {best[1] / len(qids3) - base2 / len(qids3):+.4f})')
        print('  (ANGKA SUBSET — bisa berubah saat 50Q lengkap, jangan dipakai putus weight)')
    else:
        print('\n(fts5 rankings belum ada di disk — 3-way peek menyusul)')


if __name__ == '__main__':
    main()
