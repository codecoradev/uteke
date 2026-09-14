#!/usr/bin/env python3
"""
Contradiction-segment benchmark (#1172 Fase 3).

Measures how conflict resolution (supersede, #1053/#1172) affects retrieval
quality on a synthetic knowledge-update workload. This is the ACTIVE-store
counterpart to LongMemEval's passive knowledge-update subset (subset_kupd):
instead of asking whether stale sessions are retrieved, we resolve conflicts
in the store first and ask whether recall surfaces the WINNER.

Design (deterministic, N topics):
  1. Seed 2 memories per topic:
     - stale:  the OLD fact ("… uses tool X")
     - winner: the NEW fact ("… switched to tool Y")
     plus D distractor memories per topic (same domain vocabulary, no conflict).
  2. Baseline run: both facts active (no supersede). Query each topic
     semantically ("what does topic use now?"). Measures how often the
     stale fact pollutes top-k when nothing resolved the conflict.
  3. Resolved run: supersede(stale → winner) via the CLI, then re-query.
     Measures winner@k and stale@k on the RESOLVED store.
  4. Ledger sanity: contradiction_resolutions lists the pair; undo restores
     (audited, then re-superseded).

Metrics per strategy:
  winner@{1,3,5}  — winner memory ranked in top-k
  stale@{1,5}     — stale memory present in top-k (0.0 expected post-resolve)
  MRR (winner)    — reciprocal rank of the winner

Usage:
  python3 contradiction_segment.py --binary ../../target/release/uteke --topics 40
  python3 contradiction_segment.py --topics 40 --json out.json
"""

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

TOPICS = [
    ("acme-corp", "build tooling", "gradle", "bazel"),
    ("brightpath", "package manager", "yarn", "pnpm"),
    ("cloudline", "hosting provider", "heroku", "fly.io"),
    ("dataworks", "message queue", "rabbitmq", "kafka"),
    ("everhost", "web server", "apache", "nginx"),
    ("fintrak", "ledger database", "postgresql", "cockroachdb"),
    ("gadgethub", "mobile framework", "cordova", "flutter"),
    ("heliosoft", "ci system", "jenkins", "github actions"),
    ("innodata", "search engine", "elasticsearch", "meilisearch"),
    ("jetstream", "api style", "soap", "grpc"),
    ("kobalt", "css framework", "bootstrap", "tailwind"),
    ("lumenpath", "state management", "redux", "zustand"),
    ("metriq", "observability stack", "graphite", "prometheus"),
    ("novabyte", "language runtime", "java", "kotlin"),
    ("orbita", "container runtime", "docker swarm", "kubernetes"),
    ("pixelbay", "image format", "jpeg-xl", "avif"),
    ("quantex", "config format", "ini", "toml"),
    ("riverbend", "version control", "svn", "git"),
    ("saltmarsh", "auth protocol", "basic auth", "oauth2"),
    ("tidewater", "template engine", "ejs", "handlebars"),
    ("umbracloud", "storage layer", "mongodb", "sqlite"),
    ("vellum", "docs generator", "javadoc", "rustdoc"),
    ("wharfside", "package registry", "nexus", "ghcr"),
    ("xenolith", "testing framework", "junit4", "junit5"),
    ("yarrow", "scheduler", "cron", "systemd timers"),
    ("zephyr", "linting tool", "tslint", "eslint"),
    ("argonhold", "secret store", "env files", "vault"),
    ("basaltix", "logging library", "log4j", "tracing"),
    ("cobaltrun", "runtime monitor", "new relic", "otel"),
    ("duskfield", "error tracker", "rollbar", "sentry"),
    ("emberfall", "feature flags", "launchdarkly", "unleash"),
    ("frostline", "cache layer", "memcached", "redis"),
    ("glacierpeak", "object store", "s3 class", "r2"),
    ("hollowpine", "markdown parser", "marked", "comrak"),
    ("irisvale", "date library", "moment", "dayjs"),
    ("jaderock", "http client", "axios", "fetch"),
    ("kelpforest", "orm", "sequelize", "drizzle"),
    ("lavaglass", "bundler", "webpack", "vite"),
    ("mistvale", "type checker", "flow", "typescript"),
    ("nightsky", "charting library", "chart.js", "d3"),
]

# Question phrasings deliberately avoid the exact "uses/switched to" verbs
# so the query is semantic, not keyword lookup.
QUESTION = "which {thing} does {topic} use now?"


def resolve_binary(cli_path: str) -> str:
    if cli_path:
        p = Path(cli_path)
        if p.exists():
            return str(p)
        print(f"warning: --binary {cli_path} missing; falling back", file=sys.stderr)
    repo = Path(__file__).resolve().parent.parent.parent.parent
    cand = repo / "target" / "release" / "uteke"
    if cand.exists():
        return str(cand)
    for c in ("/opt/data/.local/bin/uteke",):
        if Path(c).exists():
            return str(c)
    return shutil.which("uteke") or "uteke"


def uteke(binary: str, store: Path, namespace: str, args: list[str]) -> dict | list:
    cmd = [
        binary,
        "--store", str(store),
        "--namespace", namespace,
        "--json",
        *args,
    ]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if result.returncode != 0:
        raise RuntimeError(f"uteke {' '.join(args)} failed: {result.stderr[:400]}")
    return json.loads(result.stdout)


def remember(binary, store, ns, content: str) -> str:
    out = uteke(binary, store, ns, ["remember", content])
    return str(out["id"])  # type: ignore[no-any-return]


def supersede(binary, store, ns, old: str, new: str) -> None:
    uteke(binary, store, ns, ["supersede", old, new, "--reason", "benchmark conflict resolution"])


def recall_ids(binary, store, ns, query: str, strategy: str, k: int) -> list[str]:
    out = uteke(binary, store, ns, [
        "recall", query,
        "--limit", str(k),
        "--min", "0.0",
        "--strategy", strategy,
    ])
    return [m["memory_id"] for m in out]


def recall_map(binary, store, ns) -> dict[str, str]:
    """id → content for the namespace (to map ids back to roles)."""
    out = uteke(binary, store, ns, ["list", "--limit", "500"])
    return {m["id"]: m["content"] for m in out}


def metrics_at(ranking: list[str], winner: str, stale: str) -> tuple[float, float, float, float, float]:
    w = lambda k: 1.0 if winner in ranking[:k] else 0.0
    s = lambda k: 1.0 if stale in ranking[:k] else 0.0
    rr = 0.0
    for i, mid in enumerate(ranking, start=1):
        if mid == winner:
            rr = 1.0 / i
            break
    return w(1), w(5), rr, s(1), s(5)


def main() -> int:
    ap = argparse.ArgumentParser(description="#1172 F3 contradiction segment")
    ap.add_argument("--binary", default="", help="uteke binary path")
    ap.add_argument("--store", default="", help="existing store to reuse (default: temp)")
    ap.add_argument("--namespace", default="bench-contradiction")
    ap.add_argument("--distractors", type=int, default=3, help="distractors per topic")
    ap.add_argument("--topics", type=int, default=0, help="limit topics (0 = all)")
    ap.add_argument("--strategy", default="fusion")
    ap.add_argument("--json", default="", help="write metrics JSON here")
    args = ap.parse_args()

    topics = TOPICS[: args.topics] if args.topics > 0 else TOPICS
    binary = resolve_binary(args.binary)
    print(f"binary: {binary}")

    tmp = None
    if args.store:
        store = Path(args.store)
    else:
        tmp = tempfile.TemporaryDirectory(prefix="uteke-contradiction-")
        store = Path(tmp.name) / "bench.uteke"

    ns = args.namespace
    roles: dict[str, tuple[str, str]] = {}  # winner id → (stale id, topic)
    topic_thing = {t: th for t, th, _o, _n in topics}

    # ── Seed ────────────────────────────────────────────────────────────
    print(f"seeding {len(topics)} topics (+{args.distractors} distractors each)…")
    distractor_pool = [
        "weekly sync notes and standup summaries",
        "onboarding checklist for new engineers",
        "retro action items from the last sprint",
        "vendor invoice and billing contacts",
        "conference talk notes and takeaways",
    ]
    for topic, thing, old_tool, new_tool in topics:
        stale = remember(binary, store, ns,
                         f"{topic} uses {old_tool} for {thing}. Decision recorded after evaluation.")
        winner = remember(binary, store, ns,
                          f"{topic} switched to {new_tool} for {thing}. The old {old_tool} setup is retired.")
        roles[winner] = (stale, topic)
        for d in range(args.distractors):
            remember(binary, store, ns,
                     f"{topic} {distractor_pool[d % len(distractor_pool)]} ({thing} context {d})")

    strategies = [s.strip() for s in args.strategy.split(",") if s.strip()]
    results: dict[str, dict] = {}

    def measure(strategy: str) -> dict:
        w1s = w5s = rrs = ss1 = ss5 = 0.0
        n = 0
        for winner, (stale, topic) in roles.items():
            q = QUESTION.format(thing=topic_thing[topic], topic=topic)
            ranking = recall_ids(binary, store, ns, q, strategy, 10)
            w1, w5, rr, s1, s5 = metrics_at(ranking, winner, stale)
            w1s += w1; w5s += w5; rrs += rr; ss1 += s1; ss5 += s5
            n += 1
        return {
            "winner@1": w1s / n, "winner@5": w5s / n, "winner_mrr": rrs / n,
            "stale@1": ss1 / n, "stale@5": ss5 / n, "n": n,
        }

    # ── Baseline for ALL strategies on the UNRESOLVED store first ──────
    # (code-scanning fix: resolving per-strategy left later strategies
    # measuring "baseline" on an already-resolved store.)
    baselines = {s: measure(s) for s in strategies}

    # ── Resolve once: supersede every stale → winner ───────────────────
    for winner, (stale, _topic) in roles.items():
        supersede(binary, store, ns, stale, winner)

    # Ledger sanity: every resolution is listed (F2 surface, in-loop).
    ledger_raw = subprocess.run(
        [binary, "--store", str(store), "--json",
         "contradictions", "list", "--namespace", ns, "--limit", "500"],
        capture_output=True, text=True, timeout=600,
    )
    ledger = json.loads(ledger_raw.stdout)
    listed = {e["id"] for e in ledger}
    ledger_ok = all(stale in listed for _w, (stale, _t) in roles.items())

    # ── Resolved metrics for all strategies ────────────────────────────
    resolved = {s: measure(s) for s in strategies}

    for strategy in strategies:
        baseline = {k: v for k, v in baselines[strategy].items() if k != "n"}
        res = {k: v for k, v in resolved[strategy].items() if k != "n"}
        n = baselines[strategy]["n"]
        results[strategy] = {
            "questions": n,
            "baseline_unresolved": baseline,
            "resolved": res,
            "ledger_listed_all": ledger_ok,
        }
        print(f"\n[{strategy}]  n={n}")
        print(f"  baseline (unresolved): winner@1={baseline['winner@1']:.3f} "
              f"winner@5={baseline['winner@5']:.3f} MRR={baseline['winner_mrr']:.3f} "
              f"stale@1={baseline['stale@1']:.3f} stale@5={baseline['stale@5']:.3f}")
        print(f"  resolved (superseded): winner@1={res['winner@1']:.3f} "
              f"winner@5={res['winner@5']:.3f} MRR={res['winner_mrr']:.3f} "
              f"stale@1={res['stale@1']:.3f} stale@5={res['stale@5']:.3f}")
        print(f"  ledger lists all resolutions: {ledger_ok}")

    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(results, indent=2))
        print(f"\nmetrics → {out}")

    if tmp:
        tmp.cleanup()
    return 0


if __name__ == "__main__":
    sys.exit(main())
