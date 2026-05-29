# Revenue Model & Monetization

## Strategy: Open Core

**Open Core** = core engine open source (MIT), value-add features paid.

### Why Open Core?

* Developers trust open source more than proprietary tools
* VS Code, Docker, Git — all open core success stories
* Lowers adoption barrier (try before buy)
* Community drives innovation and feedback
* Competitors are closed-source (differentiation)

## Pricing Tiers

### Tier 1: Open Core (FREE, MIT License)

| Feature | Details |
|---------|---------|
| Core Engine | Full Rust binary |
| CLI Tool | uteke remember, recall, context |
| Memory Types | Vector + Structured + Graph + Temporal |
| Git Sync | Built-in, zero config |
| Embedding | Local model only |
| Storage | Unlimited (local) |
| Devices | Unlimited (local) |
| Support | Community (GitHub) |

**Goal:** 100% of users start here. Convert 3-5% to paid.

### Tier 2: Self-Hosted Sync Server ($19/mo or $190/yr)

| Feature | Details |
|---------|---------|
| Everything in Free |         |
| Sync Server | Rust binary, Docker deploy |
| Real-time Sync | WebSocket |
| Team Support | Multi-user, shared memory |
| Auth    | API key + basic RBAC |
| Embedding | API support (OpenAI, etc.) |
| Priority Support | Email within 24h |
| Updates | Stable releases |
| Max Users | 10      |

**Target:** Small teams (L2). Annual billing incentivized (2 months free).

### Tier 3: Managed Cloud

| Feature | Free | Pro ($9/mo) | Team ($29/mo) |
|---------|------|-------------|---------------|
| Managed Sync | YES  | YES         | YES           |
| Cloud Storage | 10K memories | 100K        | 1M            |
| Embedding API | Local only | Local + Cloud | Local + Cloud + Custom |
| Devices | 3    | Unlimited   | Unlimited     |
| Users   | 1    | 1           | Unlimited     |
| Team Sharing | NO   | NO          | YES           |
| Priority Support | NO   | YES         | YES           |
| SSO     | NO   | NO          | YES           |
| SLA     | NO   | 99.9%       | 99.99%        |

**Target:** Non-technical users (Free), power users (Pro), teams (Team).

### Tier 4: Enterprise (Custom Pricing)

| Feature | Details |
|---------|---------|
| Everything in Team |         |
| On-Premise Deploy | Dedicated instance |
| SSO/SAML/OIDC | Corporate identity |
| Compliance | SOC2, GDPR, HIPAA ready |
| Audit Log | Full activity logging |
| Custom Embedding | Fine-tuned models |
| SLA     | 99.99% with dedicated support |
| Training | Onboarding + workshops |
| Contract | Annual, negotiated |

**Target:** Fortune 500, regulated industries. Sales-driven.

## Revenue Projections (Conservative)

### Year 1 (Post-Launch)

| Metric | Q1  | Q2  | Q3  | Q4  |
|--------|-----|-----|-----|-----|
| Free Users | 500 | 2K  | 5K  | 10K |
| Self-Hosted Paid | 5   | 20  | 50  | 100 |
| Cloud Free | 200 | 800 | 2K  | 5K  |
| Cloud Pro | 5   | 15  | 40  | 80  |
| Cloud Team | 0   | 2   | 10  | 30  |
| MRR    | $95 | $380 | $1,235 | $3,110 |

### Year 2 (Growth)

| Metric | Q1  | Q2  | Q3  | Q4  |
|--------|-----|-----|-----|-----|
| Total Users | 25K | 50K | 100K | 200K |
| Paid Users | 500 | 1.2K | 2.5K | 5K  |
| MRR    | $5K | $12K | $25K | $50K |
| ARR    | $60K | $144K | $300K | $600K |

### Break-Even Analysis

* Server costs: \~$200/mo (cloud sync service)
* Payment processing: 2.9% + $0.30
* Break-even: \~150 paid users at $19/mo = $2,850/mo
* Target: Break-even in Q2 Year 1 (conservative)

## Monetization Principles


1. **Never gate core memory features** — Free tier always has full memory engine
2. **Charge for sync, team, and scale** — These are inherently multi-user needs
3. **Annual discount > 15%** — Incentivize commitment
4. **Free tier generous enough for solo devs** — This is the growth engine
5. **Enterprise = custom** — Don't self-serve, sell to enterprises