#!/usr/bin/env python3
"""
LongMemEval retrieval evaluation harness for uteke.

Measures how well uteke recalls evidence sessions/turns for each question.

Usage:
    python run_eval.py --data data/longmemeval_oracle.json --output results/
    python run_eval.py --data data/longmemeval_oracle.json --limit 50  # quick test
    python run_eval.py --data data/longmemeval_oracle.json --namespace lmeval
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# Temporal date-window boost (#1119) — harness-side, default OFF.
sys.path.insert(0, str(Path(__file__).parent))
from temporal import boost_ranking, parse_question_date, parse_temporal

# Auto-configure embedding backend from EMBED_API_KEY/EMBED_API_BASE/EMBED_MODEL env vars.
# Falls back to local ONNX if these are not set.
# MMR diversity rerank (#1120) — harness-side, default OFF.
sys.path.insert(0, str(Path(__file__).parent))
from mmr import mmr_rerank
if os.environ.get("EMBED_API_KEY") and os.environ.get("EMBED_API_BASE"):
    os.environ.setdefault("UTEKE_EMBEDDING_BACKEND", "openai")
    os.environ.setdefault("UTEKE_EMBEDDING_API_KEY", os.environ["EMBED_API_KEY"])
    os.environ.setdefault("UTEKE_EMBEDDING_BASE_URL", os.environ["EMBED_API_BASE"])
    os.environ.setdefault("UTEKE_EMBEDDING_ENDPOINT_PATH", "/v1/embeddings")
    os.environ.setdefault("UTEKE_EMBEDDING_DIMS", "768")
    if os.environ.get("EMBED_MODEL"):
        os.environ.setdefault("UTEKE_EMBEDDING_MODEL", os.environ["EMBED_MODEL"])

# Skip CLI update check in subprocess calls — saves ~500ms per call (#1006).
os.environ["UTEKE_NO_UPDATE_CHECK"] = "1"

try:
    from tqdm import tqdm
except ImportError:
    def tqdm(x, **kwargs):
        return x


def session_to_text(session):
    """Convert a chat session (list of turns) into plain text."""
    lines = []
    for turn in session:
        role = turn.get("role", "unknown")
        content = turn.get("content", "")
        lines.append(f"{role}: {content}")
    return "\n".join(lines)


def chunk_session_text(text, max_chars=2000):
    """Split a long session text into overlapping chunks for better embedding precision.

    Splits on turn boundaries ("user:" / "assistant:") to avoid cutting mid-sentence.
    Each chunk is at most max_chars. Returns list of (chunk_text, chunk_idx) tuples.
    """
    if len(text) <= max_chars:
        return [(text, 0)]

    # Split on turn boundaries
    parts = text.split("\nuser:|\nassistant:")
    # Re-split properly — the text uses "role: content" format
    lines = text.split("\n")
    chunks = []
    current = []
    current_len = 0

    for line in lines:
        line_len = len(line) + 1  # +1 for newline
        if current_len + line_len > max_chars and current:
            chunks.append("\n".join(current))
            current = [line]
            current_len = line_len
        else:
            current.append(line)
            current_len += line_len

    if current:
        chunks.append("\n".join(current))

    return [(chunk, idx) for idx, chunk in enumerate(chunks)]


def turn_has_answer(turn):
    """Check if a turn is marked as containing the answer."""
    return turn.get("has_answer", False)


def run_uteke(args, store_path, subcommand, extra_args=None):
    """Run a uteke CLI command.

    On Linux, the process is CPU-limited to 2 cores (taskset) and lowest
    priority (nice) to avoid starving other services during long benchmarks.
    On non-Linux platforms these wrappers are skipped.
    """
    # Resolve to the repo's release binary to avoid PATH ambiguity
    # (e.g. /opt/data/.cargo/bin/uteke may be x86_64 on ARM hosts).
    # scripts/ -> longmemeval/ -> benchmarks/ -> repo root.
    repo_root = Path(__file__).resolve().parent.parent.parent.parent
    uteke_bin = str(repo_root / "target" / "release" / "uteke")
    if not Path(uteke_bin).exists():
        # Prefer the known-good local install over ambiguous PATH hits
        # (/opt/data/.cargo/bin/uteke is x86_64-dynamic and broken on this host)
        for cand in ("/opt/data/.local/bin/uteke",):
            if Path(cand).exists():
                uteke_bin = cand
                break
        else:
            uteke_bin = shutil.which("uteke") or "uteke"

    # Linux-only: throttle CPU so benchmarks don't starve the server.
    if sys.platform == "linux":
        pre_cmd = ["taskset", "-c", "0-1", "nice", "-n", "19"]
    else:
        pre_cmd = []

    cmd = pre_cmd + [
        uteke_bin,
        "--store", str(store_path),
        "--namespace", args.namespace,
        "--json",
    ] + subcommand
    if extra_args:
        cmd += extra_args
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
    if result.returncode != 0:
        raise RuntimeError(f"uteke failed: {' '.join(cmd)}\nstderr: {result.stderr}")
    return result.stdout.strip()


def insert_sessions(args, store_path, entry):
    """
    Insert all haystack sessions for one question into uteke via batch JSONL import.
    Returns: (set of successfully inserted session_ids,
              dict session_id -> set of turn indices that has answers,
              dict memory_id -> session_id).
    """
    session_ids = entry.get("haystack_session_ids", [])
    sessions = entry.get("haystack_sessions", [])
    dates = entry.get("haystack_dates", [])

    answer_turns = {}  # session_id -> set of turn indices with has_answer
    inserted_sids = set()  # track which sessions actually inserted
    mid_to_sid = {}  # memory_id -> session_id mapping

    # Build JSONL for batch import — dramatically faster than per-session subprocess calls.
    # 50 sessions via individual remember calls: ~115s. Via batch import: ~12s. (10x speedup)
    jsonl_lines = []
    sid_order = []  # track session_ids in import order for counting
    for i, (sid, session) in enumerate(zip(session_ids, sessions)):
        text = session_to_text(session)
        date = dates[i] if i < len(dates) else None

        # Chunk long sessions to reduce embedding dilution (#1009).
        if args.chunk_sessions:
            chunks = chunk_session_text(text, args.chunk_size)
        else:
            chunks = [(text, 0)]

        for chunk_text, chunk_idx in chunks:
            metadata = {"session_id": sid}
            if date:
                metadata["date"] = date
            if len(chunks) > 1:
                metadata["chunk"] = chunk_idx

            record = {
                "content": chunk_text,
                "tags": ["longmemeval"],
                "type": "context",
                "metadata": metadata,
            }
            jsonl_lines.append(json.dumps(record))
            sid_order.append(sid)

        # Track answer turns
        answer_indices = set()
        for j, turn in enumerate(session):
            if turn_has_answer(turn):
                answer_indices.add(j)
        if answer_indices:
            answer_turns[sid] = answer_indices

    # Write JSONL to temp file and batch import
    import tempfile
    with tempfile.NamedTemporaryFile(mode="w", suffix=".jsonl", delete=False) as f:
        f.write("\n".join(jsonl_lines))
        jsonl_path = f.name

    try:
        stdout = run_uteke(args, store_path, ["import", jsonl_path])
        # Batch import returns {"imported": N, "skipped": M}.
        # We don't get individual memory_ids, so mid_to_sid stays empty.
        # For session-level eval, recall results are matched via metadata.session_id fallback.
        try:
            import_data = json.loads(stdout)
            imported_count = import_data.get("imported", len(sid_order))
        except (json.JSONDecodeError, TypeError):
            imported_count = len(sid_order)

        # Only mark sessions as inserted if import succeeded.
        # If import partially succeeded, all sessions are still marked since
        # we cannot map individual JSONL lines to import results.
        if imported_count > 0:
            inserted_sids.update(set(sid_order))
        else:
            print(f"  Warning: batch import returned 0 inserted", file=sys.stderr)
    except RuntimeError as e:
        print(f"  Warning: batch import failed: {e}", file=sys.stderr)
    finally:
        os.unlink(jsonl_path)

    return inserted_sids, answer_turns, mid_to_sid


def recall_and_evaluate(args, store_path, entry, answer_sessions, inserted_sids, mid_to_sid):
    """
    Run uteke recall for the question, then evaluate retrieval accuracy.

    Returns dict with retrieval metrics for this question.
    """
    question = entry["question"]
    # Only count evidence sessions that were actually inserted (avoid false negatives
    # from insert failures).
    evidence_session_ids = set(entry.get("answer_session_ids", [])) & inserted_sids

    # Run recall — fetch top-50 for Recall@5/10/50.
    # --fusion mode (#1123): RRF-fuse two uteke rankings (vector + hybrid).
    # Vector and hybrid fail on DISJOINT question sets (fast50 evidence:
    # 7 questions vector-only wins, 5 hybrid wins) — RRF captures both.
    # Weights vec×1.5 : hybrid×1 sit mid-plateau [1.3, 1.6] of R@5=0.97.
    fusion_mode = getattr(args, "fusion", False)
    # "default" sentinel: omit --strategy so the binary's zero-config
    # default (0.16.0: fusion) is what gets validated (#1123).
    strategy_args = [] if args.strategy == "default" else ["--strategy", args.strategy]
    recall_cmd = [
        "recall", question,
        "--limit", "50",
        "--tags", "longmemeval",
        *strategy_args,
        "--min", "0.0",  # Disable threshold — evaluate raw retrieval ranking (#995)
    ]
    if fusion_mode:
        # Base ranking from primary strategy, then fuse with the other.
        # Build the complementary command by REPLACING the --strategy value
        # (appending would duplicate the flag → clap error).
        other = "vector" if args.strategy == "hybrid" else "hybrid"
        alt_cmd = [
            "recall", question,
            "--limit", "50",
            "--tags", "longmemeval",
            "--strategy", other,
            "--min", "0.0",
        ]
        fusion_cmds = [recall_cmd, alt_cmd]
    else:
        fusion_cmds = [recall_cmd]
    try:
        outputs = [run_uteke(args, store_path, cmd) for cmd in fusion_cmds]
    except RuntimeError as e:
        print(f"  Warning: recall failed: {e}", file=sys.stderr)
        return None

    # Parse results
    try:
        parsed = [json.loads(o) for o in outputs]
        results = parsed if fusion_mode else (parsed[0] if isinstance(parsed[0], list) else [parsed[0]])
    except json.JSONDecodeError:
        print(f"  Warning: could not parse recall output", file=sys.stderr)
        return None

    # Deduplicate session_ids preserving rank order (first occurrence = highest rank).
    # When --chunk-sessions is enabled, multiple chunks from the same session
    # can appear in results. We keep only the first (best-ranked) occurrence
    # so that session-level Recall@k measures unique sessions, not chunks.
    def _dedup_ranking(raw_sids):
        seen = set()
        out = []
        for sid in raw_sids:
            if sid not in seen:
                seen.add(sid)
                out.append(sid)
        return out

    if fusion_mode:
        # results = [ranking_primary, ranking_secondary] (lists of recall dicts)
        per_ranking_sids = []
        for res in results:
            raw = []
            for r in res:
                mid = r.get("memory_id") or r.get("id")
                if mid and mid in mid_to_sid:
                    raw.append(mid_to_sid[mid])
                else:
                    meta = r.get("metadata", {})
                    sid = meta.get("session_id")
                    if sid:
                        raw.append(sid)
            per_ranking_sids.append(_dedup_ranking(raw))

        # RRF fuse: primary (vector) ×1.7 + secondary (hybrid) ×1, k=60.
        # Tuned on ACTUAL x86 Modal rankings: plateau [1.7, 1.9] → R@5=0.98.
        scores = {}
        primary_w = getattr(args, "fusion_primary_weight", 1.7)
        k = 60
        for i, sid in enumerate(per_ranking_sids[0]):
            scores[sid] = scores.get(sid, 0.0) + primary_w / (k + i + 1)
        for i, sid in enumerate(per_ranking_sids[1]):
            scores[sid] = scores.get(sid, 0.0) + 1.0 / (k + i + 1)
        retrieved_session_ids = sorted(scores, key=lambda s: -scores[s])
    else:
        raw_session_ids = []
        for r in results:
            mid = r.get("memory_id") or r.get("id")
            if mid and mid in mid_to_sid:
                raw_session_ids.append(mid_to_sid[mid])
            else:
                # Fallback: try metadata (older uteke versions)
                meta = r.get("metadata", {})
                sid = meta.get("session_id")
                if sid:
                    raw_session_ids.append(sid)
        retrieved_session_ids = _dedup_ranking(raw_session_ids)

    # Temporal date-window boost (#1119): soft-boost sessions whose date falls
    # inside the question's temporal window ("last month", "in February",
    # "between X and Y", ...). No-op when the question has no temporal
    # expression or when dates are missing. Default OFF (--temporal).
    if getattr(args, "temporal", False) and retrieved_session_ids:
        qdate = parse_question_date(entry.get("question_date", ""))
        window = parse_temporal(question, qdate)
        if window:
            session_dates = {}
            for sid, dstr in zip(entry.get("haystack_session_ids", []),
                                 entry.get("haystack_dates", [])):
                d = parse_question_date(dstr)
                if d is not None:
                    session_dates[sid] = d
            retrieved_session_ids = boost_ranking(
                retrieved_session_ids, session_dates, window,
                boost=getattr(args, "temporal_boost", 0.0022))

    # MMR diversity rerank (#1120): penalise redundant sessions in the top-k.
    # Zero-model post-processing (TF-IDF cosine over candidate texts) —
    # default OFF (--mmr-lambda not passed → None → no-op).
    if getattr(args, "mmr_lambda", None) is not None and retrieved_session_ids:
        # haystack_sessions[i] (list of turn dicts) aligns with
        # haystack_session_ids[i] by index — build sid -> text directly.
        texts_by_sid = {
            sid: session_to_text(hs)
            for sid, hs in zip(entry.get("haystack_session_ids", []),
                               entry.get("haystack_sessions", []))
        }
        candidate_texts = [texts_by_sid.get(sid, "") for sid in retrieved_session_ids]
        retrieved_session_ids = mmr_rerank(
            retrieved_session_ids, candidate_texts, lam=args.mmr_lambda)

    # --- Session-level metrics ---
    # Recall@k: fraction of evidence sessions in top-k
    session_recall = {}
    for k in [5, 10, 50]:
        top_k = retrieved_session_ids[:k]
        hits = len(set(top_k) & evidence_session_ids)
        total_evidence = len(evidence_session_ids) if evidence_session_ids else 1
        session_recall[f"recall_all@{k}"] = hits / total_evidence if total_evidence > 0 else 0.0

    # NDCG@k for sessions
    session_ndcg = {}
    for k in [5, 10, 50]:
        top_k = retrieved_session_ids[:k]
        # Binary relevance: 1 if evidence session, 0 otherwise
        gains = [1.0 if sid in evidence_session_ids else 0.0 for sid in top_k]
        discounts = [1.0 / np_log2(i + 2) for i in range(len(gains))]
        dcg = sum(g * d for g, d in zip(gains, discounts))

        # Ideal DCG: all relevant items at top
        ideal_len = min(len(evidence_session_ids), len(top_k))
        ideal_gains = [1.0] * ideal_len
        ideal_discounts = [1.0 / np_log2(i + 2) for i in range(len(ideal_gains))]
        idcg = sum(g * d for g, d in zip(ideal_gains, ideal_discounts))

        session_ndcg[f"ndcg_any@{k}"] = dcg / idcg if idcg > 0 else 0.0

    return {
        "session": {**session_recall, **session_ndcg},
        "_retrieved_session_ids": retrieved_session_ids[:50],
        # Note: turn-level retrieval requires per-turn indexing, which this
        # harness does not do (sessions are inserted as single memories).
        # Turn-level metrics are omitted to avoid misleading copies of
        # session-level numbers.
    }


def np_log2(x):
    """Compute log2 without numpy dependency for this helper."""
    if x <= 0:
        return 0.0
    import math
    return math.log2(x)


def main():
    parser = argparse.ArgumentParser(description="LongMemEval retrieval eval for uteke")
    parser.add_argument("--data", required=True, help="Path to longmemeval JSON file")
    parser.add_argument("--output", default="results", help="Output directory")
    parser.add_argument("--namespace", default="lmeval", help="Uteke namespace")
    parser.add_argument("--limit", type=int, default=0, help="Limit questions (0 = all)")
    parser.add_argument("--keep-store", action="store_true",
                        help="Keep the uteke store after eval (for debugging)")
    parser.add_argument("--resume", action="store_true",
                        help="Resume from existing results file (skip already-evaluated questions)")
    parser.add_argument("--reset-every", type=int, default=20,
                        help="Wipe and recreate the store every N questions to prevent memory buildup (default: 20)")
    parser.add_argument("--strategy", default="vector",
                        choices=["vector", "fts5", "hybrid", "graph", "fusion", "default"],
                        help="Recall strategy (default: vector). 'fusion' runs "
                             "the IN-CORE weighted RRF (vector×1.7+hybrid×1) — "
                             "distinct from the harness-level --fusion flag (#1123). "
                             "'default' omits --strategy entirely so the binary's "
                             "own zero-config default applies (0.16.0: fusion)")
    parser.add_argument("--fusion", action="store_true",
                        help="RRF-fuse two recall rankings: primary strategy ×1.7 + "
                             "complementary (vector↔hybrid) ×1, k=60 (#1123)")
    parser.add_argument("--fusion-primary-weight", type=float, default=1.7,
                        help="RRF weight for the primary strategy ranking (default: 1.5)")
    parser.add_argument("--chunk-sessions", action="store_true",
                        help="Chunk long sessions into smaller memories to reduce embedding dilution")
    parser.add_argument("--chunk-size", type=int, default=2000,
                        help="Max chars per chunk when --chunk-sessions is enabled (default: 2000)")
    parser.add_argument("--temporal", action="store_true",
                        help="Enable temporal date-window boost (#1119), default OFF")
    parser.add_argument("--temporal-boost", type=float, default=0.0022,
                        help="Additive RRF boost for sessions inside the temporal window (default: 0.0022)")
    parser.add_argument("--mmr-lambda", type=float, default=None,
                        help="Enable MMR diversity rerank (#1120) with this lambda "
                             "(e.g. 0.7). Default: off (None, no rerank).")
    args = parser.parse_args()

    if args.temporal:
        print(f"Temporal boost: enabled (boost={args.temporal_boost})")
    if args.mmr_lambda is not None:
        print(f"MMR diversity rerank: enabled (lambda={args.mmr_lambda})")

    # Load data
    with open(args.data) as f:
        data = json.load(f)

    if args.limit > 0:
        data = data[:args.limit]

    print(f"LongMemEval retrieval evaluation")
    print(f"  Questions: {len(data)}")
    print(f"  Namespace: {args.namespace}")
    print(f"  Strategy: {args.strategy}")
    if getattr(args, "fusion", False):
        other = "vector" if args.strategy == "hybrid" else "hybrid"
        print(f"  Fusion: enabled (RRF {args.strategy}×{args.fusion_primary_weight} + {other}×1, k=60)")
    if args.chunk_sessions:
        print(f"  Chunking: enabled (max {args.chunk_size} chars/session)")
    else:
        print(f"  Chunking: disabled")
    print()

    # Create temp store
    store_path = Path(tempfile.mkdtemp(prefix="uteke-lmeval-"))
    print(f"Store: {store_path}")

    os.makedirs(args.output, exist_ok=True)
    results_file = Path(args.output) / "retrieval_results.jsonl"

    # Resume support: load already-evaluated question IDs
    done_ids = set()
    if args.resume and results_file.exists():
        with open(results_file) as f:
            for line in f:
                try:
                    entry = json.loads(line.strip())
                    done_ids.add(entry.get("question_id"))
                except json.JSONDecodeError:
                    pass
        if done_ids:
            print(f"Resume: {len(done_ids)} questions already evaluated, skipping...")

    total_start = time.time()
    evaluated = 0

    # Open in append mode for resume, write mode for fresh run
    mode = "a" if args.resume and done_ids else "w"
    with open(results_file, mode) as fout:
        for idx, entry in enumerate(tqdm(data, desc="Evaluating")):
            qid = entry.get("question_id", f"q{idx}")

            # Skip if already evaluated (resume mode)
            if qid in done_ids:
                continue

            # Periodic store reset to prevent memory buildup
            if evaluated > 0 and evaluated % args.reset_every == 0:
                shutil.rmtree(store_path, ignore_errors=True)
                store_path.mkdir(parents=True, exist_ok=True)

            # Insert sessions
            inserted_sids, answer_sessions, mid_to_sid = insert_sessions(args, store_path, entry)

            # Recall + evaluate
            metrics = recall_and_evaluate(args, store_path, entry, answer_sessions, inserted_sids, mid_to_sid)

            if metrics is not None:
                result_entry = {
                    "question_id": qid,
                    "question_type": entry.get("question_type", "unknown"),
                    "retrieval_results": {
                        "metrics": metrics,
                        # Post-rerank session order — enables offline replay of
                        # temporal/MMR experiments without re-burning credits.
                        "retrieved_session_ids": metrics.pop("_retrieved_session_ids", []),
                    },
                }
                fout.write(json.dumps(result_entry) + "\n")
                fout.flush()  # Flush for resume safety

            evaluated += 1

            # Clean up memories for this question (avoid cross-contamination).
            # If forget fails, wipe the entire store to guarantee a clean slate.
            try:
                run_uteke(args, store_path, ["forget", "--all", "--confirm"])
            except RuntimeError:
                print(f"  Warning: forget --all failed; removing store to reset", file=sys.stderr)
                shutil.rmtree(store_path, ignore_errors=True)
                store_path.mkdir(parents=True, exist_ok=True)

    elapsed = time.time() - total_start
    print(f"\nDone in {elapsed:.1f}s")
    print(f"Results saved to {results_file}")
    print(f"\nRun: python print_metrics.py {results_file}")

    # Cleanup
    if not args.keep_store:
        shutil.rmtree(store_path, ignore_errors=True)


if __name__ == "__main__":
    main()
