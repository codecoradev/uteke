# The `strict` similarity floor (0.5) — benchmark note (#1650)

`recall --strict` / `min_score_strict` keeps a hit only when its `similarity`, the
cosine of the vector leg, reaches the floor (default **0.5**). This note says why 0.5
fits EmbeddingGemma Q4 cosine similarity, and what it costs.

## Method

Reproduce with [`scripts/strict_floor.py`](scripts/strict_floor.py). Real data, real
model: the corpus is the repository's own docs (16 pages), imported into a throw-away
store with the installed EmbeddingGemma Q4 (v0.22.0, `~/.uteke/models`). Each of 16
queries is recalled with `--strategy vector --explain`, which reports the raw cosine as
`vector_similarity`. A hit is

- **relevant** when it comes from the query's target doc and contains one of the
  query's anchor keywords;
- **irrelevant** when it comes from another doc and contains none;
- ambiguous otherwise, and left out.

Six off-topic queries (pasta, football, puppies, ...) that no doc answers give the
ceiling a floor has to clear. Two memory sizes: whole paragraphs (194 memories) and
single sentences (301 memories, closer to how people write memories).

Limits: the labels are keyword-based and the queries are mine, so treat the numbers as
a distribution, not a quality score. 16 queries and one corpus are small.

## Result

Cosine similarity, sentence-sized memories (paragraph-sized in brackets):

| Group | n | median | p90 | p99 | max |
|---|---|---|---|---|---|
| relevant | 73 (89) | 0.459 (0.399) | 0.590 (0.574) | 0.711 (0.711) | 0.711 |
| irrelevant | 1235 (1171) | 0.373 (0.328) | 0.475 (0.426) | 0.557 (0.531) | 0.614 |
| best hit of an off-topic query | 6 | 0.281 (0.262) | 0.370 (0.359) | | 0.370 (0.359) |

What each floor keeps (sentence-sized; paragraph-sized in brackets):

| Floor | relevant kept | irrelevant kept | off-topic queries that return anything |
|---|---|---|---|
| 0.30 | 97% (90%) | 94% (69%) | 33% (17%) |
| 0.40 | 73% (48%) | 34% (17%) | 0% (0%) |
| **0.50** | **38% (27%)** | **5% (2%)** | **0% (0%)** |
| 0.55 | 25% (16%) | 1.4% (0.5%) | 0% |
| 0.60 | 8% (6%) | 0.1% (0%) | 0% |

## Why 0.5

- **It clears the noise.** The best hit of every off-topic query is at most 0.37, and
  99% of irrelevant hits are below about 0.55. At 0.5, a query nobody has an answer for
  returns nothing, which is what "confident match only" has to mean.
- **It is a precision floor, not a recall floor.** At 0.5 it keeps about 3 in 8 of the
  labelled-relevant sentences (1 in 4 paragraphs) and drops about 95% of the irrelevant
  ones. Relevant hits are not all strong matches: a short question against a long
  passage rarely scores above 0.5.
- **Below 0.4 it stops filtering.** At 0.3 a third of the off-topic queries still return
  something and most irrelevant hits pass.

Recommendation: keep **0.5** as the default for `strict` ("confident only"). A caller
that wants recall with some noise control can lower `[recall] min_score_strict` to
about 0.4 (73% of the relevant sentences, about a third of the irrelevant ones, still no
off-topic query returning anything).

## Caveats for anyone copying the number

- The floor belongs to this model. EmbeddingGemma Q4 scores a short question against a
  sentence-sized memory mostly between 0.3 and 0.7. Another embedding model, another
  quantisation or another vector store distance can shift that range: re-run the script
  against it before reusing 0.5. Cosine similarity from the vector index and the same
  model should match.
- `similarity` is `null` for a hit that only full-text search found, and such hits are
  dropped under `strict`: a keyword match carries no cosine.
- `score` is untouched. Under the default fusion strategy it stays a rank-based value
  (about 0.0 to 0.25), which is why the old `strict` (a 0.5 floor on `score`) could
  never match.
