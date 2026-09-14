#!/usr/bin/env bash
# Download LongMemEval dataset from HuggingFace.
set -euo pipefail

mkdir -p data
cd data

BASE_URL="https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main"

echo "Downloading longmemeval_oracle.json (oracle subset, ~5MB)..."
if [ ! -f longmemeval_oracle.json ]; then
    curl -L -o longmemeval_oracle.json "$BASE_URL/longmemeval_oracle.json"
fi

echo ""
echo "Done. Dataset saved to benchmarks/longmemeval/data/"
echo "For the full dataset (short/medium), download manually:"
echo "  $BASE_URL/longmemeval_s_cleaned.json  (~50MB)"
echo "  $BASE_URL/longmemeval_m_cleaned.json  (~200MB)"
