#!/usr/bin/env python3
"""Performance baseline for one commit, on Modal (CPU, 2 vCPU like the LongMemEval runs).

Builds `uteke` and `uteke-serve` from an exact git SHA, then measures

  1. `uteke bench` (insert throughput, recall latency, storage footprint), and
  2. `uteke-serve` under concurrent load (`serve_load.py`): throughput and
     p50/p95/p99 latency of /recall and /remember at several client counts,
     each level on a fresh store.

The real ONNX embedding model is baked into the image, so the numbers include
the embedding cost the server pays inside its request lock (#1518). The result
file carries a manifest (commit, flags, machine) so a number can be reproduced
and compared with another commit (#1468).

    UTEKE_GIT_REF=<exact sha> UTEKE_RELEASE=v0.21.1 \
    LMEVAL_MODEL_DIR=$HOME/.codecora/uteke/models/embeddinggemma-q4 \
    modal run modal_perf.py --clients 1,4,16 --duration 20 --counts 100,1000

`UTEKE_RELEASE` only names the release whose bundled libonnxruntime is reused
(the `ort` crate loads it dynamically); the measured binary is the SHA.
Results land in benchmarks/internal/results/perf-<sha7>-<timestamp>.json.
"""
import json
import os
import pathlib
import platform
import subprocess
import sys
import time

import modal

HERE = pathlib.Path(__file__).parent  # benchmarks/internal/scripts
OUT_DIR = HERE.parent / "results"
UTEKE_GIT_REF = os.environ.get("UTEKE_GIT_REF", "")
UTEKE_RELEASE = os.environ.get("UTEKE_RELEASE", "v0.21.1")
MODEL_SOURCE = pathlib.Path(
    os.environ.get("LMEVAL_MODEL_DIR", str(pathlib.Path.home() / ".codecora/uteke/models/embeddinggemma-q4"))
)

app = modal.App("uteke-perf")

image = modal.Image.from_registry("ubuntu:24.04", add_python="3.11").apt_install(
    "curl", "ca-certificates", "util-linux", "build-essential", "pkg-config"
)
if UTEKE_GIT_REF:
    image = image.run_commands(
        "curl -sL https://github.com/codecoradev/uteke/releases/download/"
        f"{UTEKE_RELEASE}/uteke-x86_64-unknown-linux-gnu-legacy-{UTEKE_RELEASE}.tar.gz"
        " | tar xz -C /tmp && cp /tmp/libonnxruntime* /usr/local/bin/ && "
        "curl -sL https://github.com/codecoradev/uteke/archive/" + UTEKE_GIT_REF + ".tar.gz | tar xz -C /tmp && "
        "cd /tmp/uteke-*/ && "
        "curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable && "
        "$HOME/.cargo/bin/cargo build --release -p uteke-cli -p uteke-server && "
        "cp target/release/uteke target/release/uteke-serve /usr/local/bin/ && "
        "chmod +x /usr/local/bin/uteke /usr/local/bin/uteke-serve && uteke --version"
    )
image = (
    image.add_local_dir(str(MODEL_SOURCE), "/root/.codecora/uteke/models/embeddinggemma-q4")
    .add_local_file(str(HERE / "serve_load.py"), "/root/perf/serve_load.py")
)

PORT = 8799


def _wait_health(timeout=180):
    import urllib.request

    end = time.time() + timeout
    while time.time() < end:
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2).read()
            return True
        except Exception:
            time.sleep(1)
    return False


@app.function(image=image, timeout=3600, cpu=2)
def run_perf(spec: dict) -> dict:
    import shutil

    env = dict(os.environ)
    result = {"nproc": os.cpu_count(), "uteke_version": subprocess.run(
        ["uteke", "--version"], capture_output=True, text=True).stdout.strip()}

    # 1. uteke bench
    bench_home = "/tmp/perf-bench-home"
    shutil.rmtree(bench_home, ignore_errors=True)
    t0 = time.time()
    b = subprocess.run(
        ["uteke", "bench", "--store", bench_home + "/store", "--counts", spec["counts"], "--json"],
        capture_output=True, text=True, env=dict(env, UTEKE_HOME=bench_home),
    )
    result["bench"] = {"exit": b.returncode, "wall_s": round(time.time() - t0, 1)}
    try:
        result["bench"]["results"] = json.loads(b.stdout)
    except Exception:
        result["bench"]["stdout_tail"] = b.stdout[-800:]
        result["bench"]["stderr_tail"] = b.stderr[-800:]

    # 2. uteke-serve under load, a fresh store per client level.
    result["serve"] = []
    for n in [int(x) for x in spec["clients"].split(",") if x]:
        home = f"/tmp/perf-serve-home-{n}"
        shutil.rmtree(home, ignore_errors=True)
        os.makedirs(home)
        srv = subprocess.Popen(
            ["uteke-serve", "--port", str(PORT)],
            env=dict(env, UTEKE_HOME=home), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        try:
            if not _wait_health():
                result["serve"].append({"clients": n, "error": "server did not become healthy"})
                continue
            p = subprocess.run(
                [sys.executable, "/root/perf/serve_load.py", "--base-url", f"http://127.0.0.1:{PORT}",
                 "--clients", str(n), "--duration", str(spec["duration"]),
                 "--recall-share", str(spec["recall_share"]), "--seed", str(spec["seed"])],
                capture_output=True, text=True,
            )
            try:
                result["serve"].append(json.loads(p.stdout))
            except Exception:
                result["serve"].append({"clients": n, "error": (p.stderr or p.stdout)[-500:]})
        finally:
            srv.terminate()
            try:
                srv.wait(timeout=15)
            except Exception:
                srv.kill()
    return result


@app.local_entrypoint()
def main(clients: str = "1,4,16", duration: float = 20.0, counts: str = "100,1000",
         recall_share: float = 0.7, seed: int = 200, tag: str = ""):
    if not UTEKE_GIT_REF:
        sys.exit("Set UTEKE_GIT_REF to the exact commit SHA to measure.")
    if not MODEL_SOURCE.exists():
        sys.exit(f"Model source not found: {MODEL_SOURCE} (set LMEVAL_MODEL_DIR)")
    spec = {"clients": clients, "duration": duration, "counts": counts,
            "recall_share": recall_share, "seed": seed}
    t0 = time.time()
    res = run_perf.remote(spec)
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    doc = {
        "manifest": {
            "kind": "uteke-perf",
            "git_ref": UTEKE_GIT_REF,
            "onnx_runtime_from_release": UTEKE_RELEASE,
            "modal": {"cpu": 2, "image_base": "ubuntu:24.04"},
            "spec": spec,
            "tag": tag,
            "driver": {"python": platform.python_version(), "wall_s": round(time.time() - t0, 1)},
            "created_utc": stamp,
        },
        "results": res,
    }
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    out = OUT_DIR / f"perf-{UTEKE_GIT_REF[:7]}{('-' + tag) if tag else ''}-{stamp}.json"
    out.write_text(json.dumps(doc, indent=2) + "\n")
    print(f"\nResults: {out}\n")
    print(f"{'clients':>7} {'op':>9} {'ops/s':>8} {'p50':>8} {'p95':>8} {'p99':>8} {'err':>4}")
    for row in res.get("serve", []):
        if "error" in row:
            print(f"{row['clients']:>7} ERROR {row['error']}")
            continue
        for op in ("recall", "remember"):
            r = row[op]
            print(f"{row['clients']:>7} {op:>9} {r['ops_per_sec']:>8} {r['p50_ms']:>8} {r['p95_ms']:>8} {r['p99_ms']:>8} {r['errors']:>4}")
