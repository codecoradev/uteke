# Uteke — Launch Tweet Thread

**Tweet 1/7**
AI agents have amnesia.

Every new session starts from zero. Decisions lost. Context rebuilt. Again.

I built Uteke to fix this. A local-first memory engine for AI agents — written in Rust.

🧠 https://github.com/ajianaz/uteke

#Rust #AI #OpenSource

---

**Tweet 2/7**
What does Uteke do?

uteke remember "BOND uses Go + SvelteKit monorepo" --tags architecture
uteke recall "what architecture does BOND use?"

→ Stored. Indexed. Searchable. Forever.

Semantic search powered by ONNX embeddings. All local.

---

**Tweet 3/7**
Why not just use ChromaDB or MemGPT?

- No Python dependency hell
- No server to run
- No Docker container
- No API keys
- No cloud

One binary. Zero config. `cargo install uteke` and done.

---

**Tweet 4/7**
Under the hood:

🔹 SQLite — metadata & structured storage
🔹 HNSW — fast vector search (approximate nearest neighbor)
🔹 ONNX — all-MiniLM-L6-v2 embeddings (384d)
🔹 Rust — no unsafe code, memory safe

Everything lives in ~/.uteke/

---

**Tweet 5/7**
JSON output on every command:

uteke recall "deploy steps" --json
→ [{"memory": {...}, "score": 0.95}]

Makes it trivial to integrate with any AI agent, script, or tool. Python wrapper included (stdlib only).

---

**Tweet 6/7**
Built for my own AI agent fleet (Hermes). Dogfooding daily.

Uteke is MIT licensed, fully open source. Contributions welcome.

→ Architecture docs in repo
→ Contributing guide included
→ CI passing on all PRs

---

**Tweet 7/7**
If you build AI agents and hate that they forget everything — give Uteke a try.

⭐ Star the repo: https://github.com/ajianaz/uteke
🐛 Report bugs: Open an issue
🤝 Contribute: PRs to develop branch

Local-first. Zero config. Your memory, your machine.

🧠⚡
