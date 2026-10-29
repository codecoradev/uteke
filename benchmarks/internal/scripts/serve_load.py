#!/usr/bin/env python3
"""Concurrent load generator for `uteke-serve` (stdlib only).

Seeds a store, then runs N client threads for a fixed duration against a mix of
`POST /recall` and `POST /remember`, and prints one JSON document with
throughput and latency percentiles per operation. It measures the server as a
black box, so the same script can be pointed at a binary built from any commit
(baseline before an optimisation, the same numbers after it).

    python3 serve_load.py --base-url http://127.0.0.1:8767 --clients 8 \
        --duration 20 --recall-share 0.7 --seed 200 [--token T]

Content is generated deterministically from a seed so two runs are comparable.
"""
import argparse
import http.client
import json
import random
import statistics
import sys
import threading
import time
import urllib.parse

SUBJECTS = ["system", "server", "database", "client", "agent", "module", "service", "pipeline", "router", "worker"]
VERBS = ["processes", "stores", "manages", "handles", "routes", "validates", "transforms", "caches", "indexes", "schedules"]
OBJECTS = ["requests", "data", "events", "messages", "records", "transactions", "sessions", "connections", "payloads", "queries"]
ADJ = ["fast", "secure", "reliable", "scalable", "durable", "atomic", "lazy", "eager", "robust", "minimal"]


def sentence(rng):
    return f"The {rng.choice(ADJ)} {rng.choice(SUBJECTS)} {rng.choice(VERBS)} {rng.choice(ADJ)} {rng.choice(OBJECTS)}"


class Client:
    def __init__(self, base_url, token):
        u = urllib.parse.urlparse(base_url)
        self.host, self.port = u.hostname, u.port or 80
        self.headers = {"content-type": "application/json"}
        if token:
            self.headers["authorization"] = f"Bearer {token}"
        self.conn = http.client.HTTPConnection(self.host, self.port, timeout=120)

    def post(self, path, body):
        t0 = time.perf_counter()
        try:
            self.conn.request("POST", path, json.dumps(body), self.headers)
            r = self.conn.getresponse()
            r.read()
            ok = 200 <= r.status < 300
        except Exception:
            ok = False
            self.conn.close()
            self.conn = http.client.HTTPConnection(self.host, self.port, timeout=120)
        return (time.perf_counter() - t0) * 1000.0, ok


def pct(sorted_vals, p):
    if not sorted_vals:
        return None
    k = min(len(sorted_vals) - 1, max(0, int(round(p / 100.0 * (len(sorted_vals) - 1)))))
    return round(sorted_vals[k], 2)


def summarise(samples, errors, wall):
    vals = sorted(samples)
    n = len(vals)
    return {
        "count": n,
        "errors": errors,
        "ops_per_sec": round(n / wall, 2) if wall > 0 else 0.0,
        "mean_ms": round(statistics.fmean(vals), 2) if vals else None,
        "p50_ms": pct(vals, 50),
        "p95_ms": pct(vals, 95),
        "p99_ms": pct(vals, 99),
        "max_ms": round(vals[-1], 2) if vals else None,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base-url", default="http://127.0.0.1:8767")
    ap.add_argument("--clients", type=int, default=4)
    ap.add_argument("--duration", type=float, default=15.0, help="seconds of load")
    ap.add_argument("--recall-share", type=float, default=0.7, help="fraction of operations that are recalls")
    ap.add_argument("--seed", type=int, default=100, help="memories stored before the load starts")
    ap.add_argument("--rng", type=int, default=1, help="random seed for the generated content")
    ap.add_argument("--token", default="")
    ap.add_argument("--limit", type=int, default=5, help="recall limit")
    args = ap.parse_args()

    seeder = Client(args.base_url, args.token)
    rng = random.Random(args.rng)
    for _ in range(args.seed):
        _, ok = seeder.post("/remember", {"content": sentence(rng)})
        if not ok:
            print("seeding failed (is the server up, and the token right?)", file=sys.stderr)
            sys.exit(2)

    lock = threading.Lock()
    lat = {"recall": [], "remember": []}
    err = {"recall": 0, "remember": 0}
    stop_at = time.perf_counter() + args.duration

    def worker(idx):
        c = Client(args.base_url, args.token)
        r = random.Random(args.rng * 1000 + idx)
        while time.perf_counter() < stop_at:
            if r.random() < args.recall_share:
                op, path, body = "recall", "/recall", {"query": sentence(r), "limit": args.limit}
            else:
                op, path, body = "remember", "/remember", {"content": sentence(r) + f" #{r.randrange(10**9)}"}
            ms, ok = c.post(path, body)
            with lock:
                if ok:
                    lat[op].append(ms)
                else:
                    err[op] += 1

    t0 = time.perf_counter()
    threads = [threading.Thread(target=worker, args=(i,)) for i in range(args.clients)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wall = time.perf_counter() - t0

    out = {
        "clients": args.clients,
        "duration_s": round(wall, 2),
        "recall_share": args.recall_share,
        "seeded": args.seed,
        "recall": summarise(lat["recall"], err["recall"], wall),
        "remember": summarise(lat["remember"], err["remember"], wall),
        "total_ops_per_sec": round((len(lat["recall"]) + len(lat["remember"])) / wall, 2),
    }
    print(json.dumps(out))


if __name__ == "__main__":
    main()
