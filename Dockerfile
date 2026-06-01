# ── Stage 1: Builder ────────────────────────────────────────────────────
FROM debian:bookworm-slim AS builder

ARG TARGETARCH
ARG VERSION=v0.0.5

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Select correct binary for target architecture
# TARGETARCH is set by Docker buildx: amd64 or arm64
COPY binaries/ ./
RUN if [ "$TARGETARCH" = "arm64" ]; then \
      mv uteke-arm64 uteke && mv uteke-serve-arm64 uteke-serve && \
      rm -f uteke-amd64 uteke-serve-amd64; \
    else \
      mv uteke-amd64 uteke && mv uteke-serve-amd64 uteke-serve && \
      rm -f uteke-arm64 uteke-serve-arm64; \
    fi && \
    chmod +x uteke uteke-serve

# ── Stage 2: Runtime ────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

# Copy binaries
COPY --from=builder /build/uteke /usr/local/bin/uteke
COPY --from=builder /build/uteke-serve /usr/local/bin/uteke-serve

# Copy embedding model (downloaded in CI)
COPY models/ /data/models/embeddinggemma-q4

# Data directory (mount volume here)
ENV UTEKE_HOME=/data

EXPOSE 8767

ENTRYPOINT ["uteke-serve"]
