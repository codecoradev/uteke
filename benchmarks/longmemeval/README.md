# LongMemEval Benchmark Harness for Uteke

This harness evaluates uteke's retrieval quality against the [LongMemEval](https://github.com/xiaowu0162/LongMemEval) benchmark (ICLR 2025).

## Quick Start

```bash
# 1. Download LongMemEval dataset (oracle subset, ~5MB)
./download_data.sh

# 2. Install Python deps
pip install -r requirements.txt

# 3. Build uteke (ensure `uteke` binary is on PATH)
cargo build --release

# 4. Run retrieval evaluation
python run_eval.py --data data/longmemeval_oracle.json --output results/

# 5. Print metrics
python print_metrics.py results/retrieval_results.jsonl
```

## What It Measures

**Retrieval accuracy** — how well does uteke recall the correct evidence sessions?

Metrics:
- **Session-level Recall@k** — fraction of evidence sessions in top-k results
- **Session-level NDCG@k** — ranking quality

> Turn-level metrics require per-turn indexing and are not measured by this harness.

Question types evaluated:
| Type | Description |
|------|-------------|
| `single-session-user` | Single-session user info extraction |
| `single-session-assistant` | Single-session assistant info extraction |
| `single-session-preference` | User preference extraction |
| `multi-session` | Cross-session reasoning |
| `knowledge-update` | Updated information recall |
| `temporal-reasoning` | Time-based reasoning |

## How It Works

1. For each of the 500 questions:
   - Extract `haystack_sessions` (chat history with timestamps)
   - Insert each session as a uteke memory (content = session text, timestamp = session date)
2. Run `uteke recall` with the question as query
3. Check if evidence sessions appear in top-k results
4. Calculate Recall@5, Recall@10, NDCG@5, NDCG@10

## Dataset

Download from [HuggingFace](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned):
- `longmemeval_oracle.json` — oracle retrieval (evidence sessions only, ~5MB)
- `longmemeval_s_cleaned.json` — short version (~115k tokens history, ~50MB)
- `longmemeval_m_cleaned.json` — medium version (~500 sessions, ~200MB)

We recommend starting with `longmemeval_oracle.json` for quick iteration.

## Evaluation Models

Retrieval uses uteke's built-in EmbeddingGemma Q4 (768d). No external API needed.

Answer correctness evaluation (optional) uses GPT-4o via the official LongMemEval script:
```bash
export OPENAI_API_KEY=your_key
cd ../LongMemEval/src/evaluation
python evaluate_qa.py gpt-4o ../../uteke-bench/results/hypotheses.jsonl ../../data/longmemeval_oracle.json
```

## Results

See `results/` for the latest benchmark output. Add results to the main README's benchmark table.

## Dependencies

- Python 3.9+
- uteke CLI (built from this repo)
- `tqdm`, `rank_bm25`, `numpy` (see `requirements.txt`)
