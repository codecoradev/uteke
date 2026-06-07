# Show HN: Uteke — Offline-first semantic memory for AI agents, single Rust binary

**URL:** https://github.com/ajianaz/uteke

Hey HN,

I built Uteke because I was tired of AI agents forgetting everything between sessions. Every new chat with Claude, GPT, or Gemini starts from zero. Decisions get lost. Context gets rebuilt every time.

Uteke is a local-first memory engine for AI agents. It's a single Rust binary with zero configuration:

```
curl -sSL https://raw.githubusercontent.com/ajianaz/uteke/main/install.sh | sh

uteke remember "Deploy v2.1 to staging on Friday" --tags deploy
uteke recall "what deployment is coming up?"
```

**What makes it different:**

- **Single binary** — `curl | sh` and you're done. No Python, no server, no Docker.
- **Zero config** — First run downloads the embedding model (~188MB ONNX). That's it.
- **Fully offline** — No API keys. No cloud. Data lives in `~/.uteke/`. SQLite + HNSW.
- **Semantic search** — Uses EmbeddingGemma Q4 (768d) for vector similarity. ONNX runtime runs locally.
- **~30ms recall** via library, ~42ms via HTTP server. CLI cold start ~3s (model load).
- **JSON everywhere** — Every command supports `--json` output. Perfect for scripting and agent integration.

**How it works:**
1. Text goes in → embedded into 768d vector via ONNX → stored in SQLite + indexed in HNSW (usearch)
2. Query goes in → embedded → usearch finds nearest neighbors → returns ranked results
3. Everything is local. SQLite for metadata. usearch for persistent vector search. ONNX for embeddings.

**Features:**
- Multi-agent namespaces — isolate memories per agent
- Memory aging — hot/warm/cold tiers with auto-cleanup
- Temporal facts — time-bounded memories with auto-expiry
- Contradiction detection — finds conflicting memories on insert
- Consolidation — merge near-duplicate memories
- Import/export — JSONL backup and restore
- Shell hooks — auto-load project context in bash/zsh/fish
- HTTP server mode — persistent daemon for ~75x faster recall

**Why Rust?** I wanted something that works everywhere with a single binary. No dependency hell. No virtualenv. Small binary. Fast startup. Memory safe (no unsafe code).

**Who is this for?**
- AI agent developers who need persistent memory across sessions
- CLI power users who want a searchable, offline second brain
- Privacy-conscious developers who want their data on their machine

**Python integration** included — a zero-dependency wrapper that shells out to the binary. Works with any Python 3.8+.

The project is Apache 2.0 licensed. I'm using it daily with my own AI agent fleet. PRs welcome.

Happy to answer questions about the architecture, embedding choices, or Rust/ML integration.
