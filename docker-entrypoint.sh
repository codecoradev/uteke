#!/bin/sh
set -e

DATA_DIR="${UTEKE_HOME:-/data}"
MODEL_DIR="${DATA_DIR}/models/embeddinggemma-q4"
MODEL_FILE="${MODEL_DIR}/onnx/model_q4.onnx"

# Expected SHA256 checksums for model files
MODEL_ONNX_SHA256="ad1dfee81a70f7944b9b9d1cc6e48075b832881cf33fab2f2b248be78f3f0043"
MODEL_DATA_SHA256="599962c3143b040de2dd05e5975be3e9091dd067cacc6a8f7186e3203bab9e02"
TOKENIZER_SHA256="4dda02faaf32bc91031dc8c88457ac272b00c1016cc679757d1c441b248b9c47"

verify_sha256() {
  file_path="$1"
  expected="$2"
  if [ -z "$expected" ]; then
    echo "ERROR: No expected checksum provided for $file_path" >&2
    return 1
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    echo "$expected  $file_path" | sha256sum -c - || {
      echo "ERROR: Checksum verification failed for $file_path" >&2
      return 1
    }
  elif command -v shasum >/dev/null 2>&1; then
    echo "$expected  $file_path" | shasum -a 256 -c - || {
      echo "ERROR: Checksum verification failed for $file_path" >&2
      return 1
    }
  else
    echo "WARNING: sha256sum not available, skipping checksum verification" >&2
    return 1
  fi
}

# Embedding backend (#1398): UTEKE_EMBEDDING_BACKEND wins, then `[embedding]
# backend` in $UTEKE_HOME/uteke.toml, then the default (onnx). Only the onnx
# backend needs the local model; openai/ollama talk to an external embedder.
embedding_backend() {
  if [ -n "${UTEKE_EMBEDDING_BACKEND:-}" ]; then
    echo "$UTEKE_EMBEDDING_BACKEND"
    return
  fi
  toml="${DATA_DIR}/uteke.toml"
  if [ -f "$toml" ]; then
    awk '
      /^[[:space:]]*\[/ { in_embedding = ($0 ~ /^[[:space:]]*\[embedding\][[:space:]]*(#.*)?$/) ; next }
      in_embedding && /^[[:space:]]*backend[[:space:]]*=/ {
        v = $0
        sub(/^[^=]*=[[:space:]]*/, "", v)
        sub(/[[:space:]]*#.*$/, "", v)
        gsub(/["\047[:space:]]/, "", v)
        print v
        exit
      }
    ' "$toml"
  fi
}

BACKEND="$(embedding_backend)"
BACKEND="${BACKEND:-onnx}"

# Lazy download: only for the onnx backend, and only if the model is not
# present in the volume.
if [ "$BACKEND" != "onnx" ]; then
  echo "Embedding backend is '${BACKEND}': skipping the local ONNX model download."
elif [ ! -f "$MODEL_FILE" ]; then
  echo "Model not found, downloading embedding model (~208MB)..."
  if ! mkdir -p "${MODEL_DIR}/onnx" 2>/dev/null; then
    echo "ERROR: cannot create ${MODEL_DIR}: ${DATA_DIR} is not writable by $(id -un) (uid $(id -u))." >&2
    echo "Fix the volume permissions (chown the host directory to that uid), pre-populate the model there," >&2
    echo "or use an external embedder (UTEKE_EMBEDDING_BACKEND=openai|ollama) so no download is needed." >&2
    exit 1
  fi

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
