---
title: Comparison
---

# Comparison

## uteke vs Alternatives

| Feature | uteke | Mem0 | Letta | Cognee |
|---------|-------|------|-------|--------|
| Install | 1 binary | pip + Docker + Qdrant | pip + Docker + Postgres | pip + Docker + Neo4j |
| API Keys | ✅ None needed | ❌ OpenAI/LLM key | ❌ LLM key | ❌ LLM + vector DB |
| Offline | ✅ Fully | ❌ Cloud embedding | ❌ Needs server | ❌ Needs LLM + DB |
| Semantic Search | ✅ Local ONNX | ✅ Cloud embedding | ⚠️ Keyword + archival | ✅ GraphRAG |
| Rooms | ✅ Built-in | ❌ | ❌ | ❌ |
| Time-travel | ✅ `--at` flag | ❌ | ❌ | ❌ |
| MCP Server | ✅ Built-in | ❌ | ❌ | ❌ |
| Relationship Graph | ✅ `--related` | ❌ | ❌ | ✅ GraphRAG |
| Smart Decay | ✅ Pin + importance | ✅ | ✅ Core memory | ✅ TTL-based |
| Recall Cache | ✅ LRU + TTL | ❌ | ❌ | ❌ |
| Benchmarks | ✅ Built-in | ❌ | ❌ | ❌ |
| Privacy | ✅ Data stays local | ⚠️ Sent to LLM | ⚠️ Sent to LLM | ⚠️ Sent to LLM |
| Recall Speed | ~30ms | Network RTT | Network RTT | Network RTT |
| Tag Management | ✅ list/rename/delete | ⚠️ Basic | ❌ | ⚠️ Basic |
| Memory Aging | ✅ Auto-cleanup | ✅ | ✅ Core memory | ✅ TTL-based |
| Shell Hooks | ✅ bash/zsh/fish | ❌ | ❌ | ❌ |
| License | Apache-2.0 | Apache-2.0 | Apache-2.0 | Apache-2.0 |

## Why Local?

uteke runs entirely on your machine. No network calls, no API keys, no data leaving your machine. Your memories — your infrastructure.

**Performance:** ~30ms recall is possible because embedding inference, HNSW vector search, and FTS5 all run in-process. No network round-trip.

**Privacy:** Data never leaves your machine. No telemetry. No cloud dependency.

**Simplicity:** Single binary. No Docker Compose, no database server, no Python environment. Just `curl | sh` and go.
