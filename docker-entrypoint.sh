#!/bin/sh
set -e

MODEL_DIR="/data/models/embeddinggemma-q4"
MODEL_FILE="${MODEL_DIR}/onnx/model_q4.onnx"

# Expected SHA256 checksums for model files
MODEL_ONNX_SHA256="ad1dfee81a70f7944b9b9d1cc6e48075b832881cf33fab2f2b248be78f3f0043"
MODEL_DATA_SHA256="599962c3143b040de2dd05e5975be3e9091dd067cacc6a8f7186e3203bab9e02"
TOKENIZER_SHA256="4dda02faaf32bc91031dc8c88457ac272b00c1016cc679757d1c441b248b9c47"

verify_sha256() {
  file_path="$1"
  expected="$2"
  if command -v sha256sum >/dev/null 2>&1; then
    echo "$expected  $file_path" | sha256sum -c -
  elif command -v shasum >/dev/null 2>&1; then
    echo "$expected  $file_path" | shasum -a 256 -c -
  else
    echo "WARNING: sha256sum not available, skipping checksum verification"
    return 0
  fi
}

# Lazy download: only if model not pre-baked by CI
if [ ! -f "$MODEL_FILE" ]; then
  echo "Model not found, downloading embedding model (~208MB)..."
  mkdir -p "${MODEL_DIR}/onnx"

  curl -fSL --retry 3 -o "${MODEL_DIR}/onnx/model_q4.onnx" \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/onnx/model_q4.onnx"
  verify_sha256 "${MODEL_DIR}/onnx/model_q4.onnx" "$MODEL_ONNX_SHA256"

  curl -fSL --retry 3 -o "${MODEL_DIR}/onnx/model_q4.onnx_data" \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/onnx/model_q4.onnx_data"
  verify_sha256 "${MODEL_DIR}/onnx/model_q4.onnx_data" "$MODEL_DATA_SHA256"

  curl -fSL --retry 3 -o "${MODEL_DIR}/tokenizer.json" \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/tokenizer.json"
  verify_sha256 "${MODEL_DIR}/tokenizer.json" "$TOKENIZER_SHA256"

  echo "Model download and verification complete."
fi

exec uteke-serve "$@"
