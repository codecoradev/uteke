# Show HN: Uteke — Local-first memory for AI agents, written in Rust

**URL:** https://github.com/ajianaz/uteke

Hey HN,

I built Uteke because I was tired of AI agents forgetting everything between sessions. Every new chat with Claude, GPT, or Gemini starts from zero. Decisions get lost. Context gets rebuilt every time.

Uteke is a local-first memory engine for AI agents. It's a single Rust binary with zero configuration:

```
uteke remember "Deploy v2.1 to staging on Friday" --tags deploy
uteke recall "what deployment is coming up?"
```

**What makes it different:**

- **Single binary** — `cargo install` and you're done. No Python, no server, no Docker.
- **Zero config** — First run downloads the embedding model (~90MB ONNX). That's it.
- **Fully offline** — No API keys. No cloud. Data lives in `~/.uteke/`. SQLite + HNSW.
- **Semantic search** — Uses all-MiniLM-L6-v2 (384d) for vector similarity. ONNX runtime runs locally.
- **JSON everywhere** — Every command supports `--json` output. Perfect for scripting and agent integration.

**How it works:**
1. Text goes in → embedded into 384d vector via ONNX → stored in SQLite + indexed in HNSW
2. Query goes in → embedded → HNSW finds nearest neighbors → returns ranked results
3. Everything is local. SQLite for metadata. HNSW for fast vector search. ONNX for embeddings.

**Why Rust?** I wanted something I could `cargo install` on any machine and it just works. No dependency hell. No virtualenv. Small binary. Fast startup.

**Who is this for?**
- AI agent developers who need persistent memory
- People who want a local, searchable second brain
- Developers tired of context window limits
- Anyone who values their data staying on their machine

**Python integration** included — a zero-dependency wrapper that shells out to the binary. Works with any Python 3.8+.

The project is MIT licensed. I'm using it daily with my own AI agents (Hermes fleet). PRs welcome.

Happy to answer questions about the architecture, embedding choices, or Rust/ML integration.
