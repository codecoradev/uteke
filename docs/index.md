---
layout: home

hero:
  name: uteke
  text: Give Your AI a Memory
  tagline: Local-first semantic memory engine. Single Rust binary, zero infrastructure, 30ms recall, fully offline.
  actions:
    - theme: brand
      text: Get Started
      link: /getting-started
    - theme: alt
      text: GitHub
      link: https://github.com/codecoradev/uteke

features:
  - icon: 🧠
    title: Semantic Memory
    details: AI remembers by meaning, not keywords. Local ONNX embeddings (768d) with usearch persistent HNSW index. Hybrid search via FTS5 + RRF.
  - icon: 📦
    title: Single Binary
    details: Zero dependencies. No Docker, no database server, no Python, no API keys.
  - icon: 🏷️
    title: Tags + Namespaces + Metadata
    details: Multi-agent isolation built-in. Entity, category, and key:value metadata enrichment. Tag management with list/rename/delete.
  - icon: 🕰️
    title: Memory Aging
    details: Hot/Warm/Cold tier tracking. Auto-cleanup stale memories.
  - icon: 🐚
    title: Shell Hooks
    details: Auto-load project context. bash/zsh/fish support.
  - icon: 🔒
    title: Fully Offline
    details: Local ONNX embeddings. No telemetry, no cloud, no API calls.
  - icon: 🏠
    title: Rooms
    details: Group memories by context (meetings, projects). Author attribution, semantic room recall, structured document generation.
  - icon: ⏳
    title: Time-Travel Queries
    details: Recall memories as they existed at any point in time. Temporal validity with valid_from/valid_until.
  - icon: 🔗
    title: Relationship Graph
    details: Link memories with typed edges. Supersedes, contradicts, references. BFS traversal with --related --depth.
  - icon: 📉
    title: Smart Decay
    details: Composite importance scoring. Pin critical memories. Auto-recalculate on access patterns.
  - icon: 🔌
    title: MCP Server
    details: Model Context Protocol server for AI tool integration. Expose memories to Claude, GPT, and other MCP clients.
  - icon: 📊
    title: Benchmarks
    details: Built-in uteke bench for performance testing. LongMemEval retrieval harness for accuracy evaluation.
---

## Compare

| Feature | uteke | Mem0 | Letta | Cognee |
|---------|-------|------|-------|--------|
| Install | 1 binary | pip + Docker + Qdrant | pip + Docker + Postgres | pip + Docker + Neo4j |
| API Keys | ✅ None needed | ❌ OpenAI/LLM key | ❌ LLM key | ❌ LLM + vector DB |
| Offline | ✅ Fully | ❌ Cloud embedding | ❌ Needs server | ❌ Needs LLM + DB |
| Semantic Search | ✅ Local ONNX | ✅ Cloud embedding | ⚠️ Keyword + archival | ✅ GraphRAG |
| Rooms | ✅ Built-in | ❌ | ❌ | ❌ |
| Time-travel | ✅ --at flag | ❌ | ❌ | ❌ |
| MCP Server | ✅ Built-in | ❌ | ❌ | ❌ |
| Relationship Graph | ✅ --related | ❌ | ❌ | ✅ GraphRAG |
| Smart Decay | ✅ Pin + importance | ✅ | ✅ Core memory | ✅ TTL-based |
| Recall Cache | ✅ LRU + TTL | ❌ | ❌ | ❌ |
| Benchmarks | ✅ Built-in | ❌ | ❌ | ❌ |
| Privacy | ✅ Data stays local | ⚠️ Sent to LLM | ⚠️ Sent to LLM | ⚠️ Sent to LLM |
| Recall Speed | ~30ms | Network RTT | Network RTT | Network RTT |
| Tag Management | ✅ list/rename/delete | ⚠️ Basic | ❌ | ⚠️ Basic |
| Memory Aging | ✅ Auto-cleanup | ✅ | ✅ Core memory | ✅ TTL-based |
| Shell Hooks | ✅ bash/zsh/fish | ❌ | ❌ | ❌ |
| License | Apache-2.0 | Apache-2.0 | Apache-2.0 | Apache-2.0 |
