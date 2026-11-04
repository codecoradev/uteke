#!/usr/bin/env python3
"""Similarity distribution of relevant vs irrelevant recall hits (#1650).

Builds a corpus from the repository's own docs (paragraph or sentence sized
memories), imports it into a throw-away store, recalls every query with
`--strategy vector --explain` and reads `vector_similarity`, the raw cosine of
the installed embedding model. A hit is *relevant* when it comes from the
query's target doc and contains one of its anchor keywords, *irrelevant* when
it comes from another doc and contains none; the rest is ambiguous and left
out. Six off-topic queries (no doc is about them) give the ceiling a floor
must clear.

Usage (from the repo root, with the model installed under ~/.uteke/models):

    python3 benchmarks/internal/scripts/strict_floor.py paragraph /tmp/uteke-floor-p
    python3 benchmarks/internal/scripts/strict_floor.py sentence  /tmp/uteke-floor-s

The store directory is created and linked to the installed model; the real
store is never touched.
"""
import json, os, re, statistics as st, subprocess, sys

mode, home = sys.argv[1], os.path.abspath(sys.argv[2])
DOCS = ["time-travel", "rooms", "smart-decay", "relationship-graph", "tls", "docker", "viewer", "mcp",
        "multi-agent", "memory-lifecycle", "organizing-memories", "configuration", "getting-started",
        "isolation-suite", "architecture", "extensions"]

corpus = []
for d in DOCS:
    t = re.sub(r"```.*?```", "", open(f"docs/{d}.md").read(), flags=re.S)
    paras = [re.sub(r"\s+", " ", p).strip(" #|-*>") for p in re.split(r"\n\s*\n", t)]
    paras = [p for p in paras if 80 <= len(p) <= 700 and not p.startswith("|") and "---" not in p[:6]]
    for p in paras[:25]:
        if mode == "paragraph":
            corpus.append({"doc": d, "content": p})
        else:
            for s in re.split(r"(?<=[.!?])\s+", p):
                if 35 <= len(s) <= 180:
                    corpus.append({"doc": d, "content": s})
seen = set()
corpus = [r for r in corpus if not (r["content"] in seen or seen.add(r["content"]))]

os.makedirs(home, exist_ok=True)
models = os.path.expanduser("~/.uteke/models")
if not os.path.exists(home + "/models"):
    os.symlink(models, home + "/models")
env = dict(os.environ, UTEKE_HOME=home)
jsonl = home + "/corpus.jsonl"
with open(jsonl, "w") as f:
    for r in corpus:
        f.write(json.dumps({"content": r["content"], "tags": ["doc:" + r["doc"]]}) + "\n")
subprocess.run(["uteke", "import", jsonl, "--format", "jsonl"], env=env, check=True, capture_output=True)

# (query, target doc, anchor keywords: a chunk of the target doc counts as relevant only if it has one)
Q=[
("how do I see what the store knew at an earlier date","time-travel",["at","point in time","past","earlier"]),
("what happens to memories that are never recalled","smart-decay",["decay","forget","age","aging"]),
("share memories between several agents","multi-agent",["agent","share"]),
("what is a room and who can write in it","rooms",["room"]),
("enable https for the server","tls",["tls","https","certificate"]),
("run uteke in a container","docker",["docker","container","image"]),
("browse memories in the browser","viewer",["viewer","browser"]),
("connect an MCP client to uteke","mcp",["mcp"]),
("link memories to each other with edges","relationship-graph",["edge","graph","relationship"]),
("promote and consolidate memories over time","memory-lifecycle",["lifecycle","promot","consolidat"]),
("how should I tag and group memories","organizing-memories",["tag","entity","categor"]),
("where is the config file and which settings exist","configuration",["config","toml"]),
("install uteke and store my first memory","getting-started",["install","remember"]),
("isolate namespaces so they do not leak","isolation-suite",["isolation","namespace","scope"]),
("how is the engine structured internally","architecture",["engine","sqlite","index","crate"]),
("write an extension","extensions",["extension","plugin"]),
]
OFF=["best recipe for carbonara pasta","who won the football world cup in 2018","how to train a golden retriever puppy","symptoms of the common cold","tallest mountain in south america","cheap flights to lisbon in march"]

def recall(q):
    p=subprocess.run(["uteke","recall",q,"--explain","--strategy","vector","--limit","100","--json","--type","memory"],capture_output=True,text=True,env=env)
    out=p.stdout
    i=out.index("[{") if "[{" in out else None
    if i is None: return []
    return json.loads(out[i:out.rindex("]")+1])
bytext = {r["content"]: r for r in corpus}
rel=[];irr=[];top_off=[];top_irr_on=[]
per=[]
for q,doc,kw in Q:
    res=recall(q)
    top_irr=0
    for r in res:
        sim=r["explanation"]["vector_similarity"]
        c=bytext.get(r["memory"]["content"])
        if not c: continue
        has=any(k in c["content"].lower() for k in kw)
        if c["doc"]==doc and has: rel.append(sim)
        elif c["doc"]!=doc and not has:
            irr.append(sim); top_irr=max(top_irr,sim)
    top_irr_on.append(top_irr)
for q in OFF:
    res=recall(q)
    top_off.append(max(r["explanation"]["vector_similarity"] for r in res))
def q(v,p): v=sorted(v); return v[min(len(v)-1,int(p*len(v)))]
def row(n,v): print(f"{n:28} n={len(v):4} min={min(v):.3f} p10={q(v,.1):.3f} p25={q(v,.25):.3f} median={st.median(v):.3f} p75={q(v,.75):.3f} p90={q(v,.9):.3f} p99={q(v,.99):.3f} max={max(v):.3f}")
row("relevant (labelled)",rel)
row("irrelevant (labelled)",irr)
row("best irrelevant per query",top_irr_on)
row("best hit of off-topic query",top_off)
for f in (0.3,0.4,0.5,0.55,0.6,0.65,0.7):
    tp=sum(s>=f for s in rel)/len(rel); fp=sum(s>=f for s in irr)/len(irr)
    offq=sum(s>=f for s in top_off)/len(top_off)
    print(f"floor {f:.2f}: relevant kept {tp:6.1%}  irrelevant kept {fp:6.1%}  off-topic queries returning anything {offq:5.1%}")
