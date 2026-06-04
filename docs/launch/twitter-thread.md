# Uteke — Launch Tweet Thread

**Tweet 1/7**
AI agents have amnesia.

Every new session starts from zero. Decisions lost. Context rebuilt. Again.

I built Uteke to fix this. A local-first semantic memory engine for AI — single Rust binary, fully offline, 30ms recall.

🧠 https://github.com/ajianaz/uteke

#Rust #AI #OpenSource

---

**Tweet 2/7**
What does Uteke do?

curl -sSL https://raw.githubusercontent.com/ajianaz/uteke/main/install.sh | sh

uteke remember "BOND uses Go + SvelteKit monorepo" --tags architecture
uteke recall "what architecture does BOND use?"

→ Stored. Indexed. Searchable. Forever.

Semantic search powered by ONNX embeddings. All local.

---

**Tweet 3/7**
Why not just use Mem0, Letta, or Zep?

- No API keys needed
- No Docker container
- No cloud dependencies
- No Python requirement
- No server to run (optional)

One binary. Zero config. `curl | sh` and done.

---

**Tweet 4/7**
Under the hood:

🔹 SQLite — metadata & structured storage
🔹 usearch — persistent HNSW vector search
🔹 ONNX — EmbeddingGemma Q4 embeddings (256d)
🔹 Rust — memory safe, no unsafe code

Everything lives in ~/.uteke/

---

**Tweet 5/7**
Performance:

Library recall: ~30ms
Server recall: ~42ms
CLI cold start: ~3s (model load)

For real-time agent use, run uteke-serve — model stays in memory, 75x faster than CLI.

JSON output on every command. Python wrapper included.

---

**Tweet 6/7**
Features:

✅ Multi-agent namespaces
✅ Memory aging (hot/warm/cold)
✅ Temporal facts with auto-expiry
✅ Contradiction detection
✅ Consolidation (merge duplicates)
✅ Import/export (JSONL)
✅ Shell hooks (bash/zsh/fish)
✅ HTTP server mode

Apache 2.0 — use it, fork it, ship it.

---

**Tweet 7/7**
If you build AI agents and hate that they forget everything — give Uteke a try.

⭐ Star the repo: https://github.com/ajianaz/uteke
🐛 Report bugs: Open an issue
🤝 Contribute: PRs to develop branch

Offline. Zero config. Your memory, your machine.

🧠⚡
