# ── Stage 1: Builder ────────────────────────────────────────────────────
FROM debian:bookworm-slim AS builder

ARG VERSION=v0.0.5
ARG TARGET=aarch64-unknown-linux-gnu

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gpg && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Download release binary (contains uteke + uteke-serve)
RUN ARCHIVE="uteke-${TARGET}-${VERSION}.tar.gz" && \
    URL="https://github.com/ajianaz/uteke/releases/download/${VERSION}/${ARCHIVE}" && \
    echo "Downloading ${ARCHIVE}..." && \
    curl -fsSL "${URL}" -o "/tmp/${ARCHIVE}" && \
    tar xzf "/tmp/${ARCHIVE}" --strip-components=1 && \
    rm "/tmp/${ARCHIVE}" && \
    chmod +x uteke uteke-serve && \
    echo "Binary download complete."

# Download embedding model (~208MB)
RUN mkdir -p /models/onnx && \
    echo "Downloading embedding model (this may take a minute)..." && \
    curl -fsSL -o /models/onnx/model_q4.onnx \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/onnx/model_q4.onnx" && \
    curl -fsSL -o /models/onnx/model_q4.onnx_data \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/onnx/model_q4.onnx_data" && \
    curl -fsSL -o /models/tokenizer.json \
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/main/tokenizer.json" && \
    echo "Model download complete." && \
    ls -lh /models/onnx/ /models/tokenizer.json

# ── Stage 2: Runtime ────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

# Copy binaries
COPY --from=builder /build/uteke /usr/local/bin/uteke
COPY --from=builder /build/uteke-serve /usr/local/bin/uteke-serve

# Copy embedding model
COPY --from=builder /models /data/models/embeddinggemma-q4

# Data directory (mount volume here)
ENV UTEKE_HOME=/data

EXPOSE 8767

ENTRYPOINT ["uteke-serve"]
