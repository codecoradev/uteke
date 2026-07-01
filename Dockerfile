# ── Stage 1: Builder ────────────────────────────────────────────────────
FROM debian:bookworm-slim AS builder

ARG TARGETARCH
ARG VERSION=v0.6.0

LABEL version=${VERSION}
LABEL org.opencontainers.image.version=${VERSION}

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy CI-downloaded binaries (preferred).
COPY binaries/ ./
RUN if [ "$TARGETARCH" = "arm64" ]; then \
      mv uteke-arm64 uteke && mv uteke-serve-arm64 uteke-serve && mv uteke-mcp-arm64 uteke-mcp && \
      rm -f uteke-amd64 uteke-serve-amd64 uteke-mcp-amd64; \
    else \
      mv uteke-amd64 uteke && mv uteke-serve-amd64 uteke-serve && mv uteke-mcp-amd64 uteke-mcp && \
      rm -f uteke-arm64 uteke-serve-arm64 uteke-mcp-arm64; \
    fi && \
    chmod +x uteke uteke-serve uteke-mcp

# ── Stage 2: Runtime ────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 libgomp1 curl && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd --system --gid 1000 uteke && \
    useradd --system --uid 1000 --gid uteke --home /data uteke

# Copy binaries
COPY --from=builder /build/uteke /usr/local/bin/uteke
COPY --from=builder /build/uteke-serve /usr/local/bin/uteke-serve
COPY --from=builder /build/uteke-mcp /usr/local/bin/uteke-mcp

# Copy embedding model (pre-baked by CI)
COPY models/ /data/models/embeddinggemma-q4

# Copy entrypoint script (handles lazy model download)
COPY docker-entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Data directory (mount volume here for persistence)
ENV UTEKE_HOME=/data

# Create data directory with correct ownership
RUN mkdir -p /data && chown uteke:uteke /data

USER uteke

EXPOSE 8767

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
