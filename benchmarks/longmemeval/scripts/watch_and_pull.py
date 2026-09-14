#!/usr/bin/env python3
"""Watch & pull LongMemEval shards from Modal Volume every 10 minutes.

- Merges completed shards into results_modal_<strategy>/retrieval_results.jsonl
  (partial analysis possible anytime)
- Auto-relaunches `modal run modal_fanout.py` if the Modal run died
  (shards on the Volume are resumed, not recomputed)
- Writes progress to lmeval_watch_state.json; log to lmeval_watch.log

Run: nohup-style background via Hermes terminal(background=true).
Stop: kill the process (state persists on Volume + local jsonl).
"""
import json
import pathlib
import subprocess
import time

BASE = pathlib.Path("/opt/data/github/codecoradev/uteke/benchmarks/longmemeval")
STRATEGY = "hybrid"
NUM_SHARDS = 20
INTERVAL = 600  # 10 menit
STATE = BASE / "lmeval_watch_state.json"
LOG = BASE / "lmeval_watch.log"
RESULTS = BASE / f"results_modal_{STRATEGY}" / "retrieval_results.jsonl"


def log(msg):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    print(line, flush=True)
    with open(LOG, "a") as f:
        f.write(line + "\n")


def pull_shards():
    """Run pull_shards.py, parse JSON from last line."""
    r = subprocess.run(
        ["modal", "run", "pull_shards.py", "--strategy", STRATEGY],
        capture_output=True, text=True, cwd=BASE, timeout=300,
    )
    if r.returncode != 0:
        log(f"pull failed: {(r.stderr or r.stdout)[-200:]}")
        return {}
    for line in reversed((r.stdout + "\n" + r.stderr).splitlines()):
        line = line.strip()
        if line.startswith("{") and "shard_" in line:
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                continue
    return {}


def merge(shards: dict):
    seen, rows = set(), []
    for fname in sorted(shards):
        for line in shards[fname].splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                qid = json.loads(line).get("question_id")
            except json.JSONDecodeError:
                continue
            if qid and qid not in seen:
                seen.add(qid)
                rows.append(line)
    RESULTS.parent.mkdir(parents=True, exist_ok=True)
    RESULTS.write_text("\n".join(rows) + ("\n" if rows else ""))
    return len(rows)


def app_running():
    """True kalau ada app uteke-longmemeval state=ephemeral DAN punya tasks > 0.
    Pakai --json biar tidak kena format tabel terpotong."""
    import json as _json
    r = subprocess.run(["modal", "app", "list", "--json"], capture_output=True, text=True, timeout=60)
    try:
        apps = _json.loads(r.stdout)
    except _json.JSONDecodeError:
        return True  # gagal parse → asumsi jalan (jangan salah relaunch)
    for a in apps:
        desc = str(a.get("description", ""))
        if "uteke-longmemeval" in desc and a.get("state") == "ephemeral":
            return True
    return False


def relaunch():
    log("app tidak aktif → relaunch modal_fanout (resume dari Volume)")
    subprocess.Popen(
        ["modal", "run", "modal_fanout.py", "--strategy", STRATEGY,
         "--num-shards", str(NUM_SHARDS)],
        cwd=BASE,
        stdout=open(BASE / "lmeval_relaunch.log", "a"),
        stderr=subprocess.STDOUT,
        start_new_session=True,  # lepas dari process group watcher — kill watcher ≠ kill run
    )


def report(n_shards: int, n_q: int, metrics_stdout: str):
    """Progress report ke Discord thread benchmark (via discord_notify pattern)."""
    import sys
    sys.path.insert(0, "/opt/data/scripts")
    try:
        from discord_notify import send_embed
    except ImportError:
        log("report: discord_notify tidak importable — skip")
        return

    recall5 = ndcg5 = recall10 = "?"
    for line in metrics_stdout.splitlines():
        if "recall_all@5" in line:
            recall5 = line.split("=")[1].strip()
        elif "ndcg_any@5" in line:
            ndcg5 = line.split("=")[1].strip()
        elif "recall_all@10" in line:
            recall10 = line.split("=")[1].strip()

    pct = n_q * 100 // 500
    desc = (
        f"Shards selesai: **{n_shards}/20** · Q merged: **{n_q}/500** ({pct}%)\n"
        f"recall_all@5 = **{recall5}** · ndcg_any@5 = **{ndcg5}** · recall_all@10 = {recall10}"
    )
    r = send_embed("1541981760668176484", {  # thread benchmark ini
        "title": f"📊 Progress {time.strftime('%H:%M')} — LongMemEval 500Q hybrid",
        "description": desc,
        "color": 0x3498DB,
    }, bot="cmo")
    log(f"report: {'OK' if r.get('success') else r.get('error')}")


def main():
    log(f"watcher start: strategy={STRATEGY} shards={NUM_SHARDS} interval={INTERVAL}s")
    while True:
        try:
            shards = pull_shards()
            n_shards = len(shards)
            n_q = merge(shards)
            state = {"last_pull": time.strftime("%H:%M:%S"),
                     "shards_done": n_shards, "questions": n_q}
            json.dump(state, open(STATE, "w"), indent=1)
            log(f"pull: {n_shards}/{NUM_SHARDS} shard, {n_q} Q merged → {RESULTS.name}")

            # hitung partial metrics + report ke Discord
            pm = subprocess.run(
                ["python3", "print_metrics.py", str(RESULTS)],
                capture_output=True, text=True, cwd=BASE, timeout=120,
            )
            report(n_shards, n_q, pm.stdout)

            if n_shards >= NUM_SHARDS:
                if n_q >= 480:
                    # 480/500 = toleransi beberapa Q gagal evaluasi (metrics None)
                    log("SEMUA SHARD SELESAI — watcher berhenti")
                    # final: simpan metrik lengkap (pm sudah dihitung di atas)
                    open(BASE / "lmeval_final_metrics.txt", "w").write(pm.stdout)
                    log("metrik final tersimpan: lmeval_final_metrics.txt")
                    return
                else:
                    log(f"20 file shard ada tapi baru {n_q}/500 Q — ada shard partial; "
                        f"tunggu relaunch resume (kode 14400s) menuntaskan sisa Q")

            if not app_running():
                relaunch()
            else:
                log("app masih aktif")
        except Exception as e:
            log(f"error: {type(e).__name__}: {e}")
        time.sleep(INTERVAL)


if __name__ == "__main__":
    main()
