# Uteke README Marketing Optimization Plan v2

## Tujuan
Tingkatkan adopsi & GitHub stars dengan README yang lebih menjual, lebih scannable, dan lebih shareable.

---

## 🔬 Full Competitive Landscape (Jul 2026)

### Tier 1: Direct Competitors (General AI Memory)

| Competitor | Stars | Language | Architecture | API Keys | Offline | Key Differentiator |
|---|---|---|---|---|---|---|
| **Mem0** | ~48K | Python | Vector + Graph + KV | ✅ Required (OpenAI/LLM) | ❌ | Broadest adoption, $24M Series A, auto-extraction |
| **AgentMemory** | ~25K | TypeScript | iii-engine + 53 MCP tools | ✅ Required (LLM) | ❌ | #2 PHotD May 2026, 12 auto-hooks, 1423 tests, multi-language README (12 bahasa) |
| **Zep / Graphiti** | ~24K | Python | Temporal Knowledge Graph | ✅ Required | ❌ | Temporal reasoning, entity tracking over time, 63.8% LongMemEval |
| **Letta (MemGPT)** | ~21K | Python | OS-inspired tiered (Core/Recall/Archival) | ✅ Required | ❌ | Agent self-manages memory like RAM/disk |
| **Cognee** | ~12K | Python | KG + Vector pipelines | ✅ Required | ❌ | Institutional knowledge from raw docs |
| **Hindsight** | ~4K | Python | Multi-strategy + Reflect | ✅ Required | ❌ | 91.4% LongMemEval (highest score), research-focused |
| **SuperMemory** | — | Proprietary | Memory + RAG | ✅ Required | ❌ | Managed cloud, enterprise-only self-host |

### Tier 2: Emerging Competitors (OSSInsight "Agent Memory Race 2026")

| Competitor | Stars | Language | Architecture | Key Angle |
|---|---|---|---|---|
| **MemPalace** | 43.5K | Python | Palace-room verbatim (ChromaDB) | "Store everything, don't summarize" — 96.6% LongMemEval (raw mode) |
| **OpenViking** (ByteDance/volcengine) | 21.9K | Python + Go + C++ | Filesystem context database | L0/L1/L2 tiered loading, enterprise-grade, multilingual docs |
| **code-review-graph** | 7.3K | Python | Knowledge graph (tree-sitter + GraphRAG) | Codebase-specific memory, 6.8× fewer tokens on code review |
| **SimpleMem** | 3.2K | Python | Multimodal lifelong memory | Multimodal (not just text) |
| **Engram** | 2.4K | Go | SQLite + FTS5, single binary | **DIRECT competitor to Uteke** — same thesis (one binary, zero deps, MCP) |

### Tier 3: Infrastructure (Not Full Memory Platforms)

| Tool | Category | Used By Memory Platforms |
|---|---|---|
| Pinecone | Managed vector store | Infrastructure layer |
| Weaviate | Hybrid search vector store | Infrastructure layer |
| Neo4j | Graph database | Entity relationship reasoning |
| Redis | Cache + vector | Short-term memory / semantic caching |
| LangMem | SDK / library | LangGraph-native memory |

### Uteke's Position

```
         PRIVACY-FIRST (Offline, No API Keys)
                    ↑
         Uteke ★    |    
         Engram ★   |    Mem0 ★★★★★
                    |    Letta ★★★★
   ZERO-DEP ←-------+-------→ FULL-FEATURED
   (1 binary)       |    (cloud + docker)
         Uteke ★    |    AgentMemory ★★★★
         Engram ★   |    Zep ★★★
                    |    Cognee ★★
                    ↓
         CLOUD-DEPENDENT (API Keys Required)
```

**Uteke's unique wedge:** Only Rust binary with **hybrid search (HNSW + FTS5 + RRF)** + **rooms** + **time-travel** + **graph relationships** + **fully offline ONNX embeddings**. No competitor combines ALL of these.

**Direct competitor: Engram** (Go, SQLite+FTS5 only). Uteke advantage: vector search (Engram is FTS5-only, no semantic), rooms, graph edges, time-travel, tiered decay, batch import, document engine.

---

## 📊 Internal Research (from Zeko's Competitive Analysis, Jul 2026)

**Uteke vs AgentMemory** (from `uteke-agentmemory-competitive-analysis-july-2026` knowledge entry):
- AgentMemory: 24.4k stars (now 25k), TypeScript/Node.js, #2 PHotD May 2026
- 12 auto-hooks, 53 MCP tools, LLM-dependent
- Uteke: Rust, memory engine with hybrid RRF + graph RAG, rooms, time-travel
- **Uteke positioning vs AgentMemory: privacy-first, no API keys, faster (local Rust), but smaller ecosystem**

---

## 🎯 Updated Strategy Insights

### 1. Engram is the Direct Threat — Acknowledge & Differentiate
Engram (2.4K stars, Go) has the EXACT same thesis: "one binary, zero dependencies, MCP server, SQLite." The README must clearly show why Uteke > Engram:
- ✅ Hybrid search (vector + FTS5 + RRF) vs Engram's FTS5-only
- ✅ Semantic recall (find by meaning) vs keyword-only
- ✅ Rooms with author attribution
- ✅ Time-travel recall
- ✅ Graph relationships (typed edges, auto-linking)
- ✅ Tiered memory with smart decay
- ✅ Document engine (wiki/knowledge base)
- ✅ Batch import with auto-strategy

### 2. AgentMemory Shows What Works for Growth
- 12-language README → massive international reach
- Product Hunt #2 → launch strategy matters
- 53 MCP tools → developer tooling ecosystem
- Banner image + Trendshift badge + Star History = social proof stack
- **Uteke should adopt: multi-language README (at minimum EN + ID + zh-CN), Trendshift submission, badge stack**

### 3. LongMemEval Benchmark is Table Stakes
- Hindsight: 91.4%, MemPalace: 96.6%, Zep: 63.8%, Mem0: 49.0%
- Uteke has `uteke bench` + LongMemEval harness — **publish the score**
- Even if not highest, transparency builds trust

### 4. "Offline + No API Keys" is Still Unique
- NOT ONE competitor in the top 10 works fully offline without API keys
- This is Uteke's strongest wedge — **make it the #1 visual element**

### 5. MCP Compatibility is Expected
- Every major competitor has MCP support
- It's not a differentiator anymore — it's table stakes
- But "MCP + offline + Rust speed" combination IS unique

---

## 📋 Plan Eksekusi (Updated)

### Phase 1: README Rewrite (English) — `README-new.md` ✅ DONE
- [x] New hook: "Your AI forgets everything between sessions"
- [x] 30-second quick start (simple + rich versions)
- [x] Expanded comparison table (Mem0, Letta, Zep — + add AgentMemory + Engram)
- [x] Use cases section (4 personas)
- [x] Features grouped into 4 tables (not bullets)
- [x] Mermaid architecture diagram
- [x] FAQ section (6 questions)
- [x] Star History + CTA

### Phase 2: README Indonesia Rewrite — `README.id-new.md` ✅ DONE
- [x] Rewrite (not translate), casual builder tone
- [x] "Anda" → "kamu"
- [x] Same structure as English

### Phase 3: Competitor Table Updates — TODO
- [ ] Add AgentMemory to comparison table (5 competitors, not 4)
- [ ] Add Engram to comparison table (direct competitor)
- [ ] Add "LongMemEval Score" row to comparison table
- [ ] Add "Languages" row (show multi-language README)
- [ ] Add "Multi-Agent Rooms" row (Uteke unique feature)

### Phase 4: Supporting Assets
- [ ] Demo GIF: `uteke remember` → `uteke recall` → results (above the fold)
- [ ] GitHub repo topics: `ai-memory`, `rag`, `vector-database`, `offline-ai`, `semantic-search`, `llm`, `ai-agent`, `rust`, `onnx`, `mcp`, `embeddings`, `privacy`
- [ ] Run LongMemEval benchmark → publish score in README
- [ ] Submit to Trendshift (agentmemory has this, Uteke should too)
- [ ] Banner image (professional, dark theme, terminal aesthetic)

### Phase 5: Distribution (Post-Merge)
- [ ] Write blog post "Why We Built Uteke — Offline AI Memory in Rust"
- [ ] Share comparison infographic to social media
- [ ] Submit to: Awesome-AI-Agents, Awesome-Rust, HackerNews (Show HN)
- [ ] Dev.to / Medium cross-post
- [ ] Indonesian dev communities (Discord, Telegram)
- [ ] Product Hunt launch (coordinate)
- [ ] Multi-language README expansion: zh-CN, ja-JP (AgentMemory has 12 languages)
- [ ] Benchmark blog post: "Uteke vs Engram vs Mem0 — Benchmark Comparison"
