#!/bin/sh
set -e

MODEL_DIR="/data/models/embeddinggemma-q4"
MODEL_FILE="${MODEL_DIR}/onnx/model_q4.onnx"

# Lazy download: only if model not pre-baked by CI
if [ ! -f "$MODEL_FILE" ]; then
  echo "Model not found, downloading embedding model (~208MB)..."
  mkdir -p "${MODEL_DIR}/onnx"
  curl -fSL --retry 3 -o "${MODEL_DIR}/onnx/model_q4.onnx" \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/onnx/model_q4.onnx"
  curl -fSL --retry 3 -o "${MODEL_DIR}/onnx/model_q4.onnx_data" \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/onnx/model_q4.onnx_data"
  curl -fSL --retry 3 -o "${MODEL_DIR}/tokenizer.json" \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/tokenizer.json"
  echo "Model download complete."
fi

exec uteke-serve "$@"
