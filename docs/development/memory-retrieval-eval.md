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

Queries should be real user messages taken from the agent's own history, one per line.
Generated queries measure how well the retriever answers questions nobody asked.

Corpus text is sent to whatever judge endpoint you configure.
That is a third party unless you point it at something you run, so choose it deliberately — this is personal data by construction.

### The arms

| arm | what it is |
|---|---|
| `cosine` | The embedding path the runtime actually takes: cosine against the stored vector, one sort key, no tiebreak (`crates/librefang-memory/src/semantic.rs`). |
| `fts5` | The lexical path the runtime actually takes: `memories_fts MATCH … ORDER BY rank`, which is FTS5's bm25, over the index migration v50 built (#7808). |
| `rrf` | Weighted reciprocal-rank fusion of the two. Offered because the RFC proposed it, not because it is recommended — across five runs it never reached significance over cosine alone in either direction. |

The lexical arm is the daemon's own index rather than a reimplementation of BM25, which is what keeps it from drifting away from what the daemon would really do.

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

One judge and one judge model, which is the limitation every run on #7756 carries and which none of them controlled for.
The cache is keyed by judge model, so a second judge starts cold rather than blending two judges into one number, but running one is still on you.

nDCG is computed against a pool that is the union of the arms under test, not against exhaustive relevance labels — those do not exist for a corpus like this and cannot be produced at this size.
A perfect score means the arm put the pool's best documents on top, not that it found everything relevant in the corpus.

It does not measure corpus composition, which is a separate question and the one that turned out to matter for episodic retention (#7911).
Comparing per-record retrieval counts without controlling for age inverts the conclusion; a same-month cohort gave 29.5 against 110 where the uncontrolled numbers said the opposite.

## Credit

The method, the noise-floor diagnostic, the discard-don't-clamp rule for judge responses, and every number quoted above are @nevgenov's, from the investigation on #7756 and #7912.
The corpora are real production data and the retraction is in the thread alongside the finding.
