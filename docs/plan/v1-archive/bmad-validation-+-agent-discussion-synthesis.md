# BMAD Synthesis: Uteke — AI Agent Memory Framework

**Date:** 29 May 2026 **Methodology:** 6-Persona BMAD → CFO Cross-Validation → Bad Sector Adversarial Review → CTO Synthesis **Rounds:** 1 (BMAD initial + agent cross-validation) **Participants:** CTO (leader), CFO (financial review), Bad Sector (adversarial review)


---

## Scoring Evolution

| Dimension | BMAD Initial | CFO Review | Bad Sector | Synthesis |
|-----------|:------------:|:----------:|:----------:|:---------:|
| Market Opportunity | 6.2          | 4.5        | 4.0        | **4.5**   |
| Technical Feasibility | 7.3          | 7.0        | 7.0        | **7.0**   |
| Financial Viability | 5.2          | 3.5        | 3.0        | **3.5**   |
| Execution Risk | 6.0          | 4.0        | 3.5        | **4.0**   |
| Legal/Compliance | 7.3          | 7.0        | 6.5        | **7.0**   |
| **WEIGHTED TOTAL** | **6.5/10**   | **4.8/10** | **4.5/10** | **5.0/10** |

**Score went DOWN from 6.5 → 5.0 — this is healthy. Cross-validation caught over-optimistic assumptions.**


---

## Consensus Decision Matrix

| Issue | CTO | CFO | Bad Sector | Consensus |
|-------|-----|-----|------------|-----------|
| **Revenue projections** | Optimistic | Fantasy (Y2: $200-500, not $50K) | Not modeled | **CFO model adopted** |
| **Timeline** | 5-7 months | 8-12 months realistic | 12+ months | **8-12 months** |
| **Conversion rate** | 3-5% | 1-2% for v0.1 CLI | Not addressed | **1-2% Year 1** |
| **Churn modeling** | Missing | Critical gap — 8-12% monthly | Not addressed | **Must model before build** |
| **Incumbent threat** | Managed | CRITICAL — Anthropic/Cursor could ship native memory | CRITICAL — window closing | **CRITICAL** |
| **Build approach** | Full product | Library-first (2-3 weeks) | Consider contributing to mem0 | **Library-first, validate, then product** |
| **Self-hosted tier** | $19/mo | Kill for Y1 — support nightmare | Not addressed | **Cloud-only $9/mo for Y1** |
| **Local-first** | Key differentiator | Philosophy, not value prop | Limitation disguised as feature | **Engineering choice, not marketing message** |
| **Scope** | 9 docs, 4 phases | Too much for solo dev | 9 docs = scope creep risk | **Reduce to Phase 1 only initially** |
| **Opportunity cost** | Not quantified | Critical — other projects suffer | BOND/Gofin/MikroSaaS compete | **Must quantify hours/week** |


---

## Key Insights from Cross-Validation

### 🔴 CRITICAL: Incumbent Threat is Real (Both Agents Agree)

* Claude Code, Cursor, and Copilot are ALL investing in persistent memory
* Uteke's 5-7 month build window = exactly when incumbents likely ship
* **Mitigation:** Position as library/plumbing, not standalone product. If incumbents ship, Uteke becomes the open-source alternative they build on.

### 🔴 CRITICAL: Library-First, Not Product-First (CFO + CTO Agreement)

* Original CTO recommendation: "Approach A — Rust memory helper, 2-3 weeks"
* Uteke = that 2-3 week effort expanded to 5-7 month product
* **Consensus:** Build library first → open source → validate demand → THEN productize

### 🔴 HIGH: Revenue is $200-500 MRR, Not $50K (CFO Verdict)

* Year 2 $50K MRR requires 16x growth with no marketing/sales/funding
* Realistic: $200-500 MRR in Year 1, $1,500-5,000 in Year 2
* This is a side-income product, not a standalone business (unless traction is exceptional)

### 🔴 HIGH: Timeline is 2x Optimistic (Hofstadter's Law)

* Blueprint: 5-7 months → Realistic: 8-12 months
* Solo dev, part-time (20-30 hrs/wk), context-switching = 2.5x multiplier


---

## Revised Strategy (Post-Validation)

### Option A: Library-First Approach (RECOMMENDED)


1. Build Rust memory library (2-3 weeks) — wrap HNSW + SQLite
2. Use internally in Hermes fleet
3. Open source on GitHub (MIT)
4. If organic traction (100+ stars, community requests): build product around it
5. If no traction: library still useful internally, zero wasted effort

**Effort:** 2-3 weeks | **Risk:** Low | **Upside:** Medium (product path if demand exists)

### Option B: Full Product (Current Blueprint — NOT RECOMMENDED)


1. Build full Uteke as designed (5-7 months → realistic 8-12 months)
2. Launch and hope for traction
3. Pivot if incumbents ship native memory

**Effort:** 8-12 months | **Risk:** High | **Upside:** High (if market materializes before incumbents)

### Option C: Contribute to mem0 (CHEAPEST VALIDATION)


1. Add Rust core to mem0 (contribution, not fork)
2. Gets instant community, existing users, funded team support
3. Validates the "Rust for AI memory" thesis with zero market risk

**Effort:** 3-4 weeks | **Risk:** Very Low | **Upside:** Community + credibility


---

## Pre-Build Requirements (BLOCKING)


1. **Phase 0 interviews (10-20 users)** — MUST confirm pain point before ANY code
2. **Answer "why not mem0?"** credibly — what does Uteke offer that mem0 contribution can't?
3. **Local embedding model benchmark** — which model, what size, what quality?
4. **Quantify opportunity cost** — how many hours/week? What projects pause?
5. **Domain registration** — uteke.dev before any public announcement
6. **Kill criteria defined** — hard numbers, non-negotiable, reviewed by CFO

## Kill Criteria (Revised)

| Metric | Threshold | Timeline | Action |
|--------|-----------|----------|--------|
| GitHub stars | < 50      | Month 2 post-launch | Kill or pivot |
| Weekly active users | < 100     | Month 4  | Product-market fit absent |
| MRR    | < $100    | Month 6  | Maintenance mode |
| Hours invested vs MRR | > 300hrs & MRR < $200 | Month 9  | Redirect to higher-ROI project |
| Incumbent ships memory | Any major provider | Anytime  | Pivot to library/fork |


---

## Open Questions (Need Sibung Decision)


1. **Library-first or full product?** CFO and Bad Sector strongly recommend library-first. CTO neutral.
2. **Hours per week allocation?** Without this, all timeline estimates are meaningless.
3. **What projects pause for Uteke?** BOND? Gofin? MikroSaaS? All competing for time.
4. **What if Anthropic ships Claude Code memory next month?** What's the pivot plan?
5. **Contribute to mem0 vs build from scratch?** Saves months, gets instant community.


---

## Final Verdict

### Score: 5.0/10 — CONDITIONAL GO (Downgraded from 6.5)

**Go condition:** Library-first approach (2-3 weeks), NOT full product build.

* Build Rust memory library
* Open source, validate demand
* Productize ONLY if organic traction proves market exists

**No-Go if:**

* Sibung wants full product build immediately (5-7 months) — too risky given incumbent threat
* Less than 8/10 interviewees confirm "context loss" as top pain point
* No clear hours/week allocation

**Key quote from CFO:** "Don't build a product looking for users. Build for users you already know exist."

**Key quote from Bad Sector:** "The strongest products start with 1-2 pages of vision, not 9 blueprint documents."


---

*Synthesis by CTO — Hermes Agent* *Methodology: BMAD 6-Persona v6.8.0 + CFO + Bad Sector cross-validation* *All reviews saved: /tmp/bmad_uteke_analysis.md, /opt/data/cfo-uteke-review.md, /opt/data/badsector-uteke-review.md*