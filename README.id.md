<h1 align="center">Uteke</h1>
<p align="center"><strong>Berikan AI Anda memori yang tidak pernah meninggalkan perangkat Anda.</strong></p>
<p align="center">
  <em>Mesin memori semantik offline-first — satu binary, tanpa konfigurasi, recall 30ms.</em>
</p>
<p align="center">
  <a href="https://github.com/codecoradev/uteke/actions/workflows/ci.yml?branch=develop"><img src="https://github.com/codecoradev/uteke/actions/workflows/ci.yml/badge.svg?branch=develop" alt="CI" /></a>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache 2.0" /></a>
  <img src="https://img.shields.io/badge/Rust-1.75+-orange.svg" alt="Rust 1.75+" />
  <img src="https://img.shields.io/badge/status-v0.0.13-green.svg" alt="v0.0.13" />
</p>

<p align="center">
  <a href="README.md">🇬🇧 English</a> · <strong>🇮🇩 Bahasa Indonesia</strong>
</p>

---

## Mulai Cepat

```bash
# Install (macOS, Linux, Windows)
curl -sSL https://raw.githubusercontent.com/codecoradev/uteke/main/install.sh | sh

# Simpan memori dengan metadata
uteke remember "Deploy v2.1 ke staging" --tags deploy,staging \
  --entity staging-server --category infrastructure

# Pencarian hybrid (vektor + FTS5, diperingkat oleh RRF)
uteke recall "kapan kita deploy?"

# Statistik
uteke stats
```

**Itu saja.** Tanpa API key. Tanpa Docker. Tanpa Python. Saat pertama kali dijalankan, model embedding akan diunduh otomatis (~188MB) dan Anda langsung siap.

> 📖 [Opsi instalasi](INSTALL.md) · [Binary pre-built](https://github.com/codecoradev/uteke/releases) · [Docker](https://github.com/codecoradev/uteke/pkgs/container/uteke) · [Dokumentasi lengkap](https://github.com/codecoradev/uteke/tree/develop/docs)

---

## Kenapa Uteke?

AI agent melupakan semua hal antar sesi. Uteke memberikan mereka memori persisten dan dapat dicari — sepenuhnya offline, dalam satu binary.

| | **Uteke** | **Mem0** | **Letta** | **Zep** |
|---|---|---|---|---|
| **Setup** | Satu binary | pip + Docker + Qdrant | pip + Docker + Postgres | pip + Docker + Neo4j |
| **API key dibutuhkan** | ❌ Tidak ada | ✅ OpenAI/LLM key | ✅ LLM key | ✅ LLM key |
| **Offline** | ✅ Penuh | ❌ Embedding cloud | ❌ Perlu LLM server | ❌ Perlu LLM + vector DB |
| **Pencarian semantik** | ✅ ONNX lokal + hybrid FTS5 | ✅ Embedding cloud | ⚠️ Kata kunci + arsip | ✅ GraphRAG |
| **Pencarian teks penuh** | ✅ FTS5 bawaan | ❌ | ⚠️ Hanya kata kunci | ❌ |
| **Kecepatan recall** | ~30ms (library) | Round-trip jaringan | Round-trip jaringan | Round-trip jaringan |
| **Privasi** | ✅ Data tidak pernah keluar dari perangkat | ⚠️ Data dikirim ke LLM | ⚠️ Data dikirim ke LLM | ⚠️ Data dikirim ke LLM |
| **Lisensi** | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |

---

## Fitur Utama

- 🧠 **Pencarian Hybrid** — Kemiripan vektor + pencarian teks penuh FTS5, digabungkan oleh Reciprocal Rank Fusion (RRF)
- 🏷️ **Pengayaan Metadata** — Tag, entitas, kategori, dan metadata key:value pada setiap memori
- 👥 **Namespace Multi-Agent** — Memori terisolasi penuh per agent, tanpa overhead
- 🖥️ **Mode Server** — Daemon persisten dengan recall hangat ~42ms (75x lebih cepat dari CLI)
- 🔥 **Memori Bertingkat** — Pelacakan Hot/Warm/Cold dengan pembersihan otomatis memori basi
- 🔒 **Sepenuhnya Offline** — Embedding ONNX lokal (768d), tanpa telemetri, tanpa cloud, tanpa panggilan API
- 📦 **Satu Binary** — Zero dependensi. Tanpa Docker, tanpa server database, tanpa Python, tanpa API key
- 📥 **Import/Export** — Backup dan restore berbasis JSONL

📖 [Dokumentasi lengkap](docs/getting-started.md) · [Referensi CLI](docs/cli-reference.md) · [Konfigurasi](docs/configuration.md)

---

## Pengembangan

```bash
cargo build --workspace        # Build
cargo test --workspace         # Test (107 unit tests)
cargo clippy -- -D warnings    # Lint
cargo fmt                      # Format
```

Lihat [CONTRIBUTING.md](CONTRIBUTING.md) untuk panduan kontribusi lengkap.

---

## Lisensi

[Apache License 2.0](LICENSE) — gunakan, fork, kembangkan.

---

## Star History

<a href="https://www.star-history.com/?repos=codecoradev%2Fcora-cli%2Ccodecoradev%2Futeke&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=codecoradev/cora-cli%2Ccodecoradev/uteke&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=codecoradev/cora-cli%2Ccodecoradev/uteke&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=codecoradev/cora-cli%2Ccodecoradev/uteke&type=date&legend=top-left" />
 </picture>
</a>
