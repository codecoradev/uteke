// Renders docs/assets/longmemeval-comparison-{dark,light}.png (the chart under
// "Benchmarks" in the README) from the numbers below.
//
//   bun add -d playwright && bunx playwright install chromium   # once
//   bun run benchmarks/longmemeval/scripts/make_comparison_chart.mjs
//
// Every count has a source in benchmarks/longmemeval/COMPARISON.md; edit the
// data here, not the images.
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const { chromium } = createRequire(import.meta.url)("playwright");
const OUT = join(dirname(fileURLToPath(import.meta.url)), "../../../docs/assets");
const N = 470; // answerable questions: the 30 abstention ones are not scored for retrieval

// any = at least one gold session in the top 5; strict = every gold session.
const GROUPS = [
  {
    title: "Local, no LLM and no paid API in retrieval",
    systems: [
      { name: "Uteke", note: "v0.24.0 · CPU only", any: 462, strict: 413, hero: true },
      { name: "MemPalace", note: "raw mode", any: 454, strict: 403 },
      { name: "agentmemory", note: "BM25 component only", any: 452, strict: 398 },
    ],
  },
  {
    title: "Needs an LLM or a paid API in retrieval",
    hatched: true,
    systems: [
      { name: "MemPalace", note: "hybrid + LLM rerank", any: 467, strict: 423 },
      { name: "gbrain", note: "OpenAI embeddings + Voyage rerank", any: 470, strict: 451, caveat: "†" },
    ],
  },
];

const pct = (c) => ((100 * c) / N).toFixed(1);

function rows(g) {
  return g.systems
    .map((s) => {
      const bar = (label, c, cls) => `
        <div class="row ${cls}">
          <span class="lab">${label}</span>
          <span class="track"><span class="fill" style="width:${pct(c)}%"></span></span>
          <span class="val">${pct(c)}%</span>
          <span class="cnt">${c}/${N}</span>
        </div>`;
      return `
      <div class="sys ${s.hero ? "hero" : ""}">
        <div class="name">${s.name}${s.caveat ? `<sup>${s.caveat}</sup>` : ""} <span class="note">${s.note}</span></div>
        ${bar("any@5", s.any, "any")}
        ${bar("strict@5", s.strict, "strict")}
      </div>`;
    })
    .join("");
}

const html = (theme) => `<!doctype html><html><head><meta charset="utf-8">
<link href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;500;700&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
<style>
:root{
  --bg:${theme === "dark" ? "#0f1519" : "#ffffff"};
  --fg:${theme === "dark" ? "#eef2f1" : "#13201f"};
  --mut:${theme === "dark" ? "#93a3a1" : "#5a6b69"};
  --line:${theme === "dark" ? "#27322f" : "#dde4e2"};
  --track:${theme === "dark" ? "#1a2326" : "#eef2f1"};
  --other:${theme === "dark" ? "#5d6e6b" : "#8fa09d"};
  --hero:${theme === "dark" ? "#3fd6c9" : "#0b8f86"};
}
*{box-sizing:border-box;margin:0}
body{width:1600px;background:var(--bg);color:var(--fg);font-family:'Space Grotesk','Inter',sans-serif;padding:56px 72px 44px}
h1{font-size:54px;font-weight:700;letter-spacing:-.01em}
.sub{margin-top:10px;font-size:22px;color:var(--mut)}
.grp{margin-top:38px}
.gt{font:500 17px 'JetBrains Mono',monospace;color:var(--mut);text-transform:uppercase;letter-spacing:.06em;padding-bottom:10px;border-bottom:1px solid var(--line)}
.sys{margin-top:22px}
.name{font-size:28px;font-weight:600}
.name sup{font-size:18px;margin-left:3px;color:var(--mut);font-weight:500}
.name .note{font-size:19px;font-weight:400;color:var(--mut);margin-left:10px}
.hero .name{color:var(--hero)}
.row{display:flex;align-items:center;gap:16px;margin-top:8px}
.lab{width:96px;font:400 16px 'JetBrains Mono',monospace;color:var(--mut)}
.track{flex:1;height:22px;border-radius:11px;background:var(--track);overflow:hidden}
.fill{display:block;height:100%;border-radius:11px;background:var(--other)}
.hero .fill{background:var(--hero)}
.strict .fill{opacity:.7}
.hatched .fill{background:repeating-linear-gradient(135deg,var(--other) 0 8px,transparent 8px 14px)}
.val{width:118px;text-align:right;font-size:30px;font-weight:700}
.strict .val{font-weight:500;color:var(--mut)}
.hero .any .val{color:var(--hero)}
.cnt{width:96px;font:400 15px 'JetBrains Mono',monospace;color:var(--mut)}
.foot{margin-top:34px;padding-top:16px;border-top:1px solid var(--line);font-size:16.5px;line-height:1.5;color:var(--mut)}
.foot p+p{margin-top:6px}
.path{margin-top:14px;text-align:right;font:500 17px 'JetBrains Mono',monospace;color:var(--fg)}
</style></head><body>
<h1>Same benchmark, same questions, two metrics</h1>
<div class="sub">LongMemEval-S · ${N} answerable questions · top 5 sessions · higher is better · axis starts at 0</div>
${GROUPS.map(
  (g) => `<div class="grp ${g.hatched ? "hatched" : ""}"><div class="gt">${g.title}</div>${rows(g)}</div>`,
).join("")}
<div class="foot">
<p><b>any@5</b> = at least one gold session in the top 5; <b>strict@5</b> = every gold session in the top 5. Retrieval only, no answers generated. The 30 abstention questions are left out, as in the official retrieval scorer.</p>
<p>Uteke: its own v0.24.0 receipt. MemPalace: recounted by us from its committed per-question rankings. agentmemory: its published receipt, BM25 component rather than its full hybrid. gbrain: its own report. Self-reported, not re-run by us.</p>
<p>† gbrain returns 5 chunks, not 5 sessions, and was tuned on these 470 questions. Uteke's fusion weights were tuned on a 50-question subset of this benchmark.</p>
</div>
<div class="path">benchmarks/longmemeval/COMPARISON.md</div>
</body></html>`;

const browser = await chromium.launch();
for (const theme of ["dark", "light"]) {
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 }, deviceScaleFactor: 2 });
  await page.setContent(html(theme), { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(OUT, `longmemeval-comparison-${theme}.png`), fullPage: true });
  await page.close();
}
await browser.close();
