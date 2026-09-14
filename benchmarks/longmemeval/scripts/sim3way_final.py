#!/usr/bin/env python3
"""Final 3-way RRF simulation on full 500Q (fts5) x fast50 (vec, hyb).

Question: does adding fts5-only as a third RRF arm beat the shipped 2-way
fusion (vec*1.7 + hyb*1.0, k=60)?

Data availability reality:
  - fts5 rankings: 500 Q (just collected, Modal x86)
  - vector + hybrid rankings: 50 Q (fast50, Modal x86)
=> 3-way grid runs on the 50-Q intersection; the fts5 arm at 500 Q is used
   for the fts5-only baseline and failure analysis.
"""
import json
import os
from collections import defaultdict

BASE = os.path.dirname(os.path.abspath(__file__))


def load_jsonl(p):
    with open(p) as f:
        return [json.loads(l) for l in f if l.strip()]


def rrf(rankings, weights, k=60, topn=45):
    s = defaultdict(float)
    for ids, w in zip(rankings, weights):
        for rank, sid in enumerate(ids[:topn]):
            s[sid] += w / (k + rank + 1)
    return [x for x, _ in sorted(s.items(), key=lambda t: -t[1])]


def r_at(ids, t, k):
    return len(set(ids[:k]) & set(t)) / len(t)


def main():
    fts = {}
    for s in range(10):
        for r in load_jsonl(os.path.join(BASE, 'results_modal_fts5_partial', f'shard_{s:02d}.jsonl')):
            fts[r['question_id']] = r['retrieval_results']['retrieved_session_ids']
    vec = {r['question_id']: r['retrieval_results']['retrieved_session_ids']
           for r in load_jsonl(os.path.join(BASE, 'results_modal_vector', 'retrieval_results.jsonl'))}
    hyb = {r['question_id']: r['retrieval_results']['retrieved_session_ids']
           for r in load_jsonl(os.path.join(BASE, 'results_modal_hybrid_fresh_post1130', 'retrieval_results.jsonl'))}

    with open(os.path.join(BASE, 'data', 'longmemeval_s_cleaned.json')) as f:
        ds = json.load(f)
    gt = {q['question_id']: q['answer_session_ids'] for q in ds}
    qtype = {q['question_id']: q.get('question_type', '?') for q in ds}

    # fts5-only baseline on full 500
    qids500 = [q for q in fts if gt.get(q)]
    tot5 = sum(r_at(fts[q], gt[q], 5) for q in qids500)
    tot10 = sum(r_at(fts[q], gt[q], 10) for q in qids500)
    print(f'fts5-only 500Q: n={len(qids500)} R@5={tot5/len(qids500):.4f} R@10={tot10/len(qids500):.4f}')

    # 3-way grid on 50-Q intersection
    qids = [q for q in vec if q in hyb and q in fts and gt.get(q)]
    n = len(qids)
    print(f'\n3-way grid on intersection n={n}')
    base2 = {q: r_at(rrf([vec[q], hyb[q]], (1.7, 1.0)), gt[q], 5) for q in qids}
    b2 = sum(base2.values()) / n
    print(f'  2-way shipped (1.7,1.0)        R@5 = {b2:.4f}')

    best = (None, -1.0)
    for wv in (1.3, 1.4, 1.7, 2.0):
        for wf in (0.1, 0.2, 0.4, 0.6, 1.0):
            w = (wv, 1.0, wf)
            tot = sum(r_at(rrf([vec[q], hyb[q], fts[q]], w), gt[q], 5) for q in qids)
            if tot > best[1]:
                best = (w, tot)
    print(f'  best 3-way w={best[0]}          R@5 = {best[1]/n:.4f}  (delta {best[1]/n - b2:+.4f})')

    # per-question delta at shipped 2-way vs best 3-way
    wv, _, wf = best[0]
    flips = []
    for q in qids:
        d = r_at(rrf([vec[q], hyb[q], fts[q]], best[0]), gt[q], 5) - base2[q]
        if abs(d) > 1e-9:
            flips.append((q[:8], qtype.get(q, '?'), round(d, 2)))
    print(f'  flipped questions: {len(flips)}')
    for f in flips:
        print('   ', f)

    # failure overlap analysis on 500Q fts5
    fts_fail = {q for q in qids500 if r_at(fts[q], gt[q], 5) < 1.0}
    print(f'\nfts5 fails R@5<1 @500Q: {len(fts_fail)}/{len(qids500)}')
    by_type = defaultdict(int)
    for q in fts_fail:
        by_type[qtype.get(q, '?')] += 1
    for t, c in sorted(by_type.items(), key=lambda t: -t[1]):
        print(f'  {t}: {c}')


if __name__ == '__main__':
    main()
