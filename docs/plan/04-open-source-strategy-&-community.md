# 04 — Open Source Strategy & Community

**Uteke v2 · Library-First Approach**


---

## License: MIT

**Why MIT, not Apache 2.0 or GPL?**

| Consideration | MIT | Apache 2.0 | GPL |
|---------------|-----|------------|-----|
| Simplicity    | ✅ Short, clear | Medium     | Complex |
| Corporate adoption | ✅ No patent clause concerns | Patent grant (good/bad) | ❌ Viral, blocks use in products |
| Community friendliness | ✅ Lowest friction | Good       | Can alienate commercial users |
| Competition risk | ⚠️ Someone can fork commercially | Same       | Protects against closed forks |

MIT wins because:


1. **Maximum adoption** — anyone can use it without talking to lawyers
2. **Community signal** — stars and forks are the v2 metric, not revenue
3. **Future optionality** — if we productize later, dual-license is still possible
4. **Rust ecosystem norm** — most popular Rust crates are MIT/Apache dual


---

## Branding

| Element | Choice | Notes |
|---------|--------|-------|
| **Name** | Uteke  | Swahili origin, means "to remember" — fitting for a memory tool |
| **Tagline** | "Local-first memory for AI agents" | Clear, specific, differentiates from cloud solutions |
| **Domain** | uteke.dev | Purchased or planned; redirects to GitHub |
| **Logo** | Simple icon (brain + file) | Designed after v2 ships, not before |
| **Color** | Rust-inspired amber/orange | Ties to language choice |
| **Repository** | github.com/ajianaz/uteke | Clean org or personal repo |


---

## GitHub Repo Structure

```
uteke/
├── .github/
│   ├── workflows/
│   │   ├── ci.yml           # test + lint + build
│   │   └── release.yml      # binary release
│   └── ISSUE_TEMPLATE/
│       ├── bug_report.md
│       └── feature_request.md
├── src/                      # Core library
├── cli/                      # CLI binary
├── tests/
├── benches/
├── docs/
│   ├── getting-started.md
│   ├── api-reference.md
│   ├── hermes-integration.md
│   └── architecture.md
├── examples/
│   └── python_hermes.py
├── Cargo.toml
├── README.md                 # HERO DOC
├── LICENSE
└── CHANGELOG.md
```


---

## README Strategy (Hero Document)

The README is the landing page. It must sell in 5 seconds:

```
┌─────────────────────────────────────────┐
│  🧠 Uteke                               │
│  Local-first memory for AI agents        │
│                                          │
│  [Demo GIF: remember → recall in <50ms] │
│                                          │
│  A fast, local, offline semantic memory   │
│  engine written in Rust. Store, search,  │
│  and retrieve knowledge for your AI       │
│  agents without any cloud dependency.     │
│                                          │
│  $ uteke remember "Rust uses ownership"  │
│  $ uteke recall "memory management"      │
│                                          │
│  ⚡ <50ms recall · 🔒 100% local ·       │
│  📦 <2MB binary                          │
└─────────────────────────────────────────┘
```

Sections:


1. **Hero** — name, tagline, demo GIF, 3 badges
2. **Quick Start** — install + 3 commands
3. **Why Uteke?** — 3 bullet comparison with alternatives
4. **Use Cases** — Hermes, research, personal knowledge
5. **Benchmarks** — retrieval speed table
6. **Architecture** — simplified diagram
7. **Roadmap** — link to future phases doc
8. **Contributing** — how to help
9. **License** — MIT


---

## Launch Plan

### Phase 1: Soft Launch (Week 5-6)

* Publish to GitHub with complete README and docs
* Post to relevant subreddits: r/rust, r/LocalLLaMA, r/ChatGPT
* Submit to Hacker News (Show HN) — **single launch post**
* Tweet thread with demo GIF
* Goal: 50-100 stars in first week

### Phase 2: Content Marketing (Month 2-3)

* Blog post: "Why I built a local memory engine in Rust"
* Video walkthrough (5-10 min screen recording)
* Cross-post to Dev.to, Medium, Lobste.rs
* Respond to every GitHub issue and discussion
* Goal: 200-500 stars by Month 3

### Phase 3: Community Building (Month 3-6)

* Curate feature requests, accept community PRs
* Monthly "Uteke updates" GitHub Discussions
* Consider Matrix/Discord only if 100+ stars
* Speak at local meetups (Jakarta, if applicable)
* Goal: 500+ stars, active contributors


---

## Community Metrics (Nice to Have, Not Kill Criteria)

| Metric | 1 Month | 3 Months | 6 Months | If Hit... |
|--------|---------|----------|----------|-----------|
| GitHub stars | 50      | 200      | 500      | Validate demand |
| Weekly downloads | 20      | 100      | 500      | People are using it |
| Contributors | 1 (me)  | 3        | 10       | Community is forming |
| GitHub issues | 5       | 20       | 50       | Engagement signal |
| Blog mentions | 1       | 5        | 15       | Word of mouth |

**None of these kill v2.** v2 succeeds if I use it daily. Community metrics inform v3+ decisions.


---

## When to Productize

| Signal | Threshold | Action |
|--------|-----------|--------|
| SDK demand | 5+ requests or PRs | Build Python bindings (PyO3) |
| Editor demand | 10+ requests | Start VS Code extension |
| Sync demand | 20+ requests | Design sync protocol |
| Team demand | 500+ stars OR funding | Hire, build product |
| Revenue viability | 1000+ monthly active users | Freemium model |

Each threshold is a **decision gate**, not an automatic trigger. Assess effort vs. reward at each point.