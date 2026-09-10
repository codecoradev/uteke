#!/usr/bin/env python3
"""Convert LoCoMo (snap-research) conversations into the LongMemEval harness schema.

Output files feed DIRECTLY into ../longmemeval/scripts/run_eval.py — same fields,
same metric family, so numbers are comparable across benchmarks and no second
evaluator exists to maintain.

Mapping (LoCoMo -> harness):
- conversation.session_N (list of {speaker, text, dia_id}) -> haystack_sessions[N]
- conversation.session_N_date_time                        -> haystack_dates[N]
- qa[].question / .category                               -> question / question_type
- qa[].evidence "D1:3" (dia_id prefix)                    -> answer_session_ids

Methodology decisions (recorded in README.md):
- Category 5 (adversarial / unanswerable) is EXCLUDED from the retrieval dataset —
  it has no gold evidence, so R@k is undefined for it. Reported separately.
- QA whose evidence parses to zero sessions (4 QA in the full set, category 3) are
  skipped and counted in the summary.
- question_date is left empty (LoCoMo does not ship one); the harness temporal
  boost is OFF by default, so this affects nothing.
"""
#!/usr/bin/env python3
import argparse
import json
import re
import sys
from pathlib import Path

CATEGORY_NAMES = {1: "single-hop", 2: "temporal", 3: "multi-hop", 4: "open-domain", 5: "adversarial"}
EV_RE = re.compile(r"D(\d+):")


def parse_evidence(evidence):
    """'D1:3' / 'D1:3; D1:5' / 'D1:3 D2:1' -> {1, 3...} session numbers."""
    out = set()
    for e in evidence or []:
        out.update(int(m) for m in EV_RE.findall(e))
    return out


def convert(data_path, out_dir):
    data = json.loads(Path(data_path).read_text())
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    summary = {"conversations": [], "skipped_adversarial": 0, "skipped_no_evidence": 0}
    for conv in data:
        sample_id = conv["sample_id"]
        talks = conv["conversation"]
        sess_nums = sorted(
            int(m.group(1))
            for k in talks
            if (m := re.fullmatch(r"session_(\d+)", k))
        )
        sids = [f"locomo_{sample_id}_s{n}" for n in sess_nums]
        # Normalize turns to the harness contract ({role, content}):
        # session_to_text() reads turn["role"] / turn["content"]; LoCoMo ships
        # speaker/text (+dia_id, kept for provenance/debugging).
        sessions = [
            [{"role": t.get("speaker", "unknown"), "content": t.get("text", ""), **{
                k: v for k, v in t.items() if k not in ("speaker", "text")}}
             for t in talks[f"session_{n}"]]
            for n in sess_nums
        ]
        dates = [talks.get(f"session_{n}_date_time", "") for n in sess_nums]

        entries, n_adv, n_nogold = [], 0, 0
        for idx, qa in enumerate(conv["qa"]):
            cat = qa["category"]
            if cat == 5:
                n_adv += 1
                continue
            gold = parse_evidence(qa.get("evidence"))
            if not gold:
                n_nogold += 1
                continue
            entries.append({
                "question_id": f"{sample_id}_q{idx}",
                "question_type": CATEGORY_NAMES[cat],
                "question": qa["question"],
                "question_date": "",
                "answer": qa.get("answer", ""),
                "answer_session_ids": sorted(f"locomo_{sample_id}_s{n}" for n in gold),
                "haystack_session_ids": sids,
                "haystack_sessions": sessions,
                "haystack_dates": dates,
            })

        out_file = out_dir / f"locomo_{sample_id}.json"
        out_file.write_text(json.dumps(entries, ensure_ascii=False, indent=1))
        summary["conversations"].append({
            "sample_id": sample_id, "sessions": len(sess_nums),
            "qa_total": len(conv["qa"]), "qa_evaluable": len(entries),
            "skipped_adversarial": n_adv, "skipped_no_evidence": n_nogold,
            "file": out_file.name,
        })
        print(f"conv {sample_id}: {len(sess_nums)} sessions -> {len(entries)} evaluable QA "
              f"({n_adv} adversarial, {n_nogold} no-evidence skipped)")

    (out_dir / "conversion_summary.json").write_text(json.dumps(summary, indent=1))
    total = sum(c["qa_evaluable"] for c in summary["conversations"])
    print(f"\nTotal evaluable QA: {total} across {len(summary['conversations'])} conversations")
    return summary


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--data", default="locomo10.json", help="path to locomo10.json")
    ap.add_argument("--out", default="harness", help="output directory for harness-schema files")
    args = ap.parse_args()
    convert(args.data, args.out)
