# Measuring memory retrieval quality

`scripts/memory-retrieval-eval.py` compares memory retrieval strategies on a real corpus, using an LLM judge and confidence intervals bootstrapped over paired per-query differences.
It is an operator tool.
Run it when you are deciding between retrieval strategies or embedding models on your own data, and never as part of a build.

## Why it exists in this repository

LibreFang compiles in one retrieval strategy and one embedding model.
Until #7756 nobody had measured either against an alternative on a real corpus, and the reason was structural rather than negligent: measuring it required reading a copy of the database and reimplementing the ranking path in a program that lived nowhere.

@nevgenov did that work out of tree, ran it five times over three real corpora and roughly 5800 LLM-judged pairs, and the result was decisive twice.
The first run found the compiled-in cosine arm last on all three corpora, significantly so on two.
The control run — same corpora, same queries, same seed, same judge, one variable changed — withdrew that finding: it was an artifact of a 4-bit-quantized embedder, and the embedder swap moved the vector arm by +0.096 and +0.032 nDCG@10, further than any difference between retrieval methods that any of the five runs could resolve.

Both the finding and its retraction are the argument for keeping this in the tree.
A question that changes the answer twice is a question the project needs a supported way to ask, and a harness that lives in a linked repository is exactly as far away as no harness at all.

## Where the line is drawn

The judging half needs a live judge model and a live embedding endpoint.
It is not hermetic and not deterministic — re-running the same commit gives slightly different nDCG, because the judged pool is the union of every arm's output and an LLM grades it.
That is correct for a decision harness and wrong for a test, so it never runs in CI, on any trigger.
This repository deliberately does not expose a provider key to PR runners, and this tool does not ask it to start.

The arithmetic half is the opposite.
nDCG@10, the paired bootstrap, pool construction and the judge-response parse rule are pure functions of numbers, and a subtle error in any of them produces output that looks exactly like correct output.
So `--self-test` runs a synthetic-input suite with no network and no secret, and CI runs that on every PR as the `Memory Eval Harness Self-Test` job.

The suite also drives `run()` end to end over a throwaway SQLite corpus with the judge and the embedder stubbed out, because *which arms get scored at all* turned out to be the layer where a wrong number could be published with a confidence interval attached (#7950), and that is not checkable one function at a time.

Python and stdlib-only, which is the existing convention for `scripts/` — `check-skills-supply-chain.py` states it as policy and `read_email.py` is the nearest precedent for a live-endpoint, env-credentialed operator tool that CI never invokes.
There is no `requirements.txt` in this repository and this file does not introduce one.

## Running it

The corpus must be a **copy**.
The database is opened `mode=ro&immutable=1`, and `immutable=1` is a promise you are making to SQLite that the file cannot change underneath it — a promise that is only true of a copy.
The tool never contacts the daemon and never writes to what it reads.

```bash
cp ~/.librefang/memory.db /tmp/corpus.db

export LIBREFANG_EVAL_JUDGE_URL=https://your-endpoint/v1/chat/completions
export LIBREFANG_EVAL_JUDGE_MODEL=your-judge-model
export LIBREFANG_EVAL_JUDGE_KEY=...
export LIBREFANG_EVAL_EMBED_URL=https://your-endpoint/v1/embeddings
export LIBREFANG_EVAL_EMBED_MODEL=your-embedding-model

python3 scripts/memory-retrieval-eval.py run \
    --corpus /tmp/corpus.db --queries queries.txt --arms cosine,fts5,rrf --out run-baseline.json
```

Corpus text is sent to whatever judge endpoint you configure.
That is a third party unless you point it at something you run, so choose it deliberately — this is personal data by construction.

### Building the queries file, and the trap in the obvious way to do it

Queries must be real user messages taken from the agent's own history, one per line.
Generated queries measure how well the retriever answers questions nobody asked.

**A query taken from that history is also a document in the corpus it is about to be run against.**
The per-turn writer stores every exchange verbatim in a `[Past exchange]` row, so the message you copy out is still sitting in `memories`, containing the question word for word.
Every arm finds it, the judge grades it 3 — correctly, it *does* answer the question — and the arm is rewarded for retrieving the query back.

This is not a rounding error.
On a 2619-row corpus with 30 queries, deleting the 30 source rows moved cosine nDCG@10 from **0.8709 to 0.7195** (#7950).
That is a 0.1514 shift from 1.1 % of the corpus, nearly four times the 0.040 noise floor the harness declares for itself, and it moves in the direction of looking good.
For scale, the embedder swap that motivated this whole investigation moved nDCG by +0.096.

There are two ways to build a file that does not do this, and the first is the default.

**Name the source row and let the harness exclude it.**
A queries line may be `memory_id<TAB>text`, and that row is dropped from every arm for that query only — inside the ranking, so the arm is not also penalised for a short list.
The id is the one the `SELECT` that produced the line already had in hand, so this costs nothing to adopt.
Several ids may be given comma-separated with no spaces, for a turn stored across more than one row.
The first field is read as an id list only when it contains no whitespace at all, so a tab *inside* a message (`fix the<TAB>indentation`) stays query text.
That rule is a heuristic rather than a proof, and it has one hole: a message whose *first word* is followed by a tab (`TODO<TAB>fix the retry budget`) reads as an id plus a query that has lost its first word.
So the harness also checks every id against the corpus and refuses to run when one names no row, which catches both that truncation and a file built against a different database.
If a run stops with `this corpus has no such row`, look at the line it names before assuming the id is stale.

```bash
python3 - <<'PY' > queries.txt
import sqlite3

conn = sqlite3.connect("file:/tmp/corpus.db?mode=ro&immutable=1", uri=True)
rows = conn.execute(
    "SELECT id, content FROM memories "
    "WHERE deleted = 0 AND content LIKE '[Past exchange]%' "
    "ORDER BY created_at DESC LIMIT 40"
).fetchall()
for memory_id, content in rows:
    them = [line[len("Them: "):].strip() for line in content.splitlines() if line.startswith("Them: ")]
    if them and them[0]:
        print(f"{memory_id}\t{them[0]}")
PY
```

One query per line means a multi-line user turn keeps only its first line, so read the file before spending a judging pass on it.

**Or split the corpus and the queries in time.**
Take the queries from messages *newer* than a cutoff and the corpus from rows *older* than it, which is what a retrieval system actually faces: a new question against everything already stored.
This needs no file format change, but it does need doing deliberately, because the natural move is to `SELECT` the user turns out of the same database the corpus comes from.
Extract the queries from the copy first, then trim the copy, then point the harness at it — the v50 triggers keep `memories_fts` in step with the delete, so the lexical arm stays consistent with what the vector arm sees, and `immutable=1` is a promise that nothing changes the file while the harness holds it open.

```bash
# 1. queries from the recent tail (same snippet as above, without the id column)
# 2. corpus from everything before the cutoff
sqlite3 /tmp/corpus.db "DELETE FROM memories WHERE created_at >= '2026-06-01'"
```

Either way, say which one you used when you report numbers.
An undocumented study-design choice here moves the headline figure further than any of the arms do.

### The arms

| arm | what it is |
|---|---|
| `cosine` | The embedding path the runtime actually takes: cosine against the stored vector, one sort key, no tiebreak (`crates/librefang-memory/src/semantic.rs`). |
| `fts5` | The lexical path the runtime actually takes: `memories_fts MATCH … ORDER BY rank`, which is FTS5's bm25, over the index migration v50 built (#7808). |
| `rrf` | Weighted reciprocal-rank fusion of the two. Offered because the RFC proposed it, not because it is recommended — across five runs it never reached significance over cosine alone in either direction. |

The lexical arm is the daemon's own index rather than a reimplementation of BM25, which is what keeps it from drifting away from what the daemon would really do.

**`fts5` and `rrf` need a corpus at migration v50 or later.**
`memories_fts` arrives in v50 (#7808), and `--corpus` explicitly wants a copy of a production database, which is exactly where an older schema comes from.
The harness probes the index before spending any judging — and before the `--reembed` pass, which is a real spend against a real endpoint — and refuses those arms rather than scoring them, naming the migration version it needs; if they were the only arms requested it aborts.
That refusal is recorded the way a withholding is, under the table and in the run JSON's `withheld_arms`, so a pasted result carries it and a later `--noise-floor` cannot pick a refused arm as its instrument.
That refusal exists because an arm that cannot issue a query scores 0.0 on every query, and nothing downstream can tell that apart from an arm that ran and lost — the reported run certified `fts5 | 0.0000 | +0.8709 [+0.7864, +0.9351] significant` for an arm that never executed, while `rrf` silently became the cosine arm under a second name, because fusing a ranking with an empty list returns that ranking (#7950).
The same refusal applies to any arm that comes back empty for every counted query, whatever the reason.

### What a run costs

Judging is one call per (query, document) pair, and that is the whole runtime.
Thirty queries over three arms took about **25 minutes**, against about 2 minutes for the same queries graded in pooled calls — roughly 30x.

The trade is deliberate.
A per-pair grade has no position effects, and a malformed response costs one pair instead of a whole query.
But two consequences follow and neither is optional to know about:

* **`--judge-cache` is load-bearing, not a convenience.** It defaults to `.memory-eval-judgments.json` and is flushed after every graded pair, so an interrupted run resumes and a second arm costs only the pairs it newly contributes. Delete it or point it somewhere new and you pay the full 25 minutes again.
* **Grades are uncalibrated across documents.** The judge never sees two candidates side by side, so a 2 on one document and a 2 on another were assigned independently. nDCG only needs the ordering within one query's pool, which is why this is acceptable, but it means an absolute grade distribution from this tool is not a measurement of anything.

### Comparing a different embedding model

A query embedded by one model and scored against vectors written by another compares two unrelated coordinate systems and produces confident nonsense.
`--reembed` rebuilds every corpus vector with the configured endpoint, and refuses to mix re-embedded and stored rows.

```bash
python3 scripts/memory-retrieval-eval.py run \
    --corpus /tmp/corpus.db --queries queries.txt --arms cosine,fts5 --reembed \
    --embed-model intfloat/multilingual-e5-large \
    --query-prefix 'query: ' --passage-prefix 'passage: ' \
    --out run-e5.json
```

The prefixes matter and are not cosmetic.
A large part of the current embedding landscape, the e5 family among it, is trained on an asymmetric `query:` / `passage:` pair; omitting them narrowed the relevant-versus-irrelevant margin from 0.12 to 0.07 in the measurement on #7912, which was enough to produce a false confirmation of the hypothesis under test.
The daemon cannot send prefixes today (#7912, gap 3), so this is the only place the question is answerable.

Embedding and judging are both cached on disk, keyed by model and content.
Adding an arm therefore costs the pairs it newly contributes rather than a full re-spend, which is the practical reason the original investigation stopped at five runs.

## Reading the output

```
78 queries counted (2 dropped), 2054 judged pairs, seed 0, noise floor 0.040.

| arm | nDCG@10 | vs leader |
|---|---|---|
| fts5 | 0.6642 | — |
| rrf 0.5/0.5 | 0.6167 | +0.0475 [-0.0123, +0.1109] noise |
| cosine | 0.6025 | +0.0616 [+0.0045, +0.1217] significant |
```

**The intervals are paired, and that is load-bearing.**
Between-query variance dwarfs between-arm variance on corpora of this size: an unpaired comparison cannot detect the effect at all and will report "no difference" for a difference that is really there.
The bootstrap resamples *queries*, and both arms are scored on the same query against the same judged pool, which is why the pool is a set union rather than per-arm.
The tool has no unpaired mode and `bootstrap_paired_ci` will not accept unaligned input.

**A significant difference is not automatically a result.**
The method carries a noise floor of roughly 0.04 nDCG.
Because the judged pool is the union of all arms, changing one arm changes what the judge sees for every arm — in the original runs the embedder-independent lexical arms moved by -0.040 and -0.016 between two runs whose only changed variable was the vector arm.
Differences smaller than that are reported as `significant but below the noise floor`, which is the verdict people skip.

The bound that has to clear the floor is **the one nearest zero**, not the one furthest from it.
`[+0.01, +0.20]` against a 0.040 floor is consistent with a true effect of 0.01, which this method cannot resolve, so it lands in the middle category — reading it as a result means believing the optimistic end of an interval whose other end says "unmeasurable".

**An arm can be withheld from scoring entirely.**
When it is, the report says so under the table and gives no score, no interval and no verdict for it:

```
**Withheld from scoring**, with no score, no interval and no verdict. […]

- `fts5` — returned nothing for all 30 counted queries.
- `rrf 0.5/0.5` — degenerates into a duplicate of its other input once fts5 is withheld.
```

That block is part of the result, so keep it when you paste the table.
A withheld arm is also removed from `stable_arms`, and `--noise-floor` refuses to use one: an arm that issued no query shows zero movement between two runs, and a floor measured that way is 0.0000, which licenses every difference.
When the run itself declares no stable arm left, `--noise-floor` says so and names the withheld arm rather than reporting an arm-set mismatch.

The run JSON also carries `empty_rankings` — how many counted queries each arm returned nothing for — which is the evidence behind every withholding decision and the only way to see the borderline the harness deliberately still scores: an arm that answered some queries and not others.

Measure the floor for your own setup rather than inheriting 0.04:

```bash
python3 scripts/memory-retrieval-eval.py --noise-floor run-baseline.json run-e5.json --stable-arms fts5
```

An arm declared stable has no path by which its own score can move between those two runs, so whatever movement it shows is the pool shifting underneath it.
That movement is the resolution limit of the whole method.

**Below about 30 counted queries the report says so.**
The bootstrap has too few distinct per-query differences to resample and the interval is decoration.
The runs on #7756 counted 59 to 80.

## What it does not settle

Whether the self-match inflation of #7950 changes the *ordering* of the arms, as opposed to the absolute number.
The reporter could not test that, because the only arm that could have shown it does not run on a pre-v50 corpus, and neither corpus available at the time was both v50+ and large enough.
The 0.1514 shift is demonstrated inflation of one arm's score; it is not evidence that the comparison between arms is distorted, and it is not evidence that it is not.

One judge and one judge model, which is the limitation every run on #7756 carries and which none of them controlled for.
The cache is keyed by judge model, so a second judge starts cold rather than blending two judges into one number, but running one is still on you.

nDCG is computed against a pool that is the union of the arms under test, not against exhaustive relevance labels — those do not exist for a corpus like this and cannot be produced at this size.
A perfect score means the arm put the pool's best documents on top, not that it found everything relevant in the corpus.

It does not measure corpus composition, which is a separate question and the one that turned out to matter for episodic retention (#7911).
Comparing per-record retrieval counts without controlling for age inverts the conclusion; a same-month cohort gave 29.5 against 110 where the uncontrolled numbers said the opposite.

## Credit

The method, the noise-floor diagnostic, the discard-don't-clamp rule for judge responses, and every number quoted above are @nevgenov's, from the investigation on #7756 and #7912.
The corpora are real production data and the retraction is in the thread alongside the finding.
