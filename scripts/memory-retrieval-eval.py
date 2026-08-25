#!/usr/bin/env python3
"""Compare memory retrieval strategies on a real corpus, with an LLM judge and paired confidence intervals.

Why this exists
---------------

LibreFang compiles in one retrieval strategy and one embedding model, and until now there was no supported way for an operator to ask whether either choice was costing them anything on their own corpus.
That question was answered once, by @nevgenov on #7756, using an out-of-tree Python harness run against a read-only copy of a production database — five runs over three real corpora, ~5800 LLM-judged pairs.
The headline finding of the first run ("the retriever the runtime hardcodes is the worst of five arms") did not survive the reporter's own control run: it was an artifact of a 4-bit-quantized embedder, and swapping the embedding model moved the vector arm further (+0.096 / +0.032 nDCG@10) than any difference between retrieval methods that any of the five runs could resolve.

Both the finding and its retraction are the argument for this file.
The measurement was decisive twice, and both times it required work that lived nowhere in the tree.

What it does
------------

For each query, every arm produces a ranked top-k.
The union of those lists is pooled and shuffled, so the judge grades documents without seeing which arm retrieved them or in what position.
An LLM grades each (query, document) pair 0-3.
nDCG@10 is computed per arm per query, and every arm is compared against the leader with a confidence interval bootstrapped over *paired* per-query differences.

The pairing is not a detail.
Between-query variance dwarfs between-arm variance on corpora this size: an unpaired comparison cannot detect the effect at all, and will report "no difference" for a difference that is really there.
This harness therefore has no unpaired mode, and `bootstrap_paired_ci` refuses inputs that are not aligned per query.

What it deliberately does not do
--------------------------------

It does not run in CI, on any trigger, ever.
It needs a live judge model and a live embedding endpoint, so it is neither hermetic nor deterministic — the same commit re-run gives slightly different nDCG.
That is fine for a decision harness and wrong for a test.

The one part that *is* hermetic is the arithmetic — nDCG, the paired bootstrap, pool construction, and the judge-response parse rule — and getting that subtly wrong is the failure mode nobody would notice.
So it is unit-tested with synthetic inputs under `--self-test`, which is what CI runs, in about a second, with no network and no secret.

Reading the numbers
-------------------

The method has a measurable noise floor of roughly 0.04 nDCG, and it is measurable rather than assumed.
Because the judged pool is the union of all arms, changing one arm changes what the judge sees for every arm: in the reporter's runs the embedder-independent lexical arms moved by -0.040 and -0.016 between two runs whose only changed variable was the vector arm.
Any difference below that floor is not a result.
`--noise-floor RUN_A.json RUN_B.json` recomputes it for your own setup from two runs, using the arms you declare embedder-independent.

Three things that each cost the reporter a re-run, and where this harness stands on each:

* e5-family embedders are trained on an asymmetric `query:` / `passage:` pair, and omitting the prefixes narrowed the relevant-vs-irrelevant margin from 0.12 to 0.07 — enough to produce a false confirmation of the hypothesis under test.
  Guarded: `--query-prefix` and `--passage-prefix` make this a measurable variable here, even though the daemon cannot yet send prefixes (#7912, gap 3).
* Comparing a candidate embedder against the stored vectors is meaningless, because a query in one embedding space scored against vectors from another is not a comparison of anything.
  Guarded: `--reembed` rebuilds the corpus vectors with the configured endpoint, and refuses to mix the two.
* Comparing per-record retrieval counts without controlling for age inverts the conclusion — a same-month cohort gave 29.5 against 110 where the uncontrolled numbers said the opposite.
  Not guarded, because it is a corpus-composition question rather than a retrieval one; if you extend this tool that way, control for age.

Safety
------

The corpus is opened read-only and immutable, and it must be a *copy*.
This tool never contacts the daemon, never binds a port, and never writes to the database it reads.
Corpus text is sent to whatever judge endpoint you configure — that is a third party unless you point it at something you run, so choose it deliberately.

Usage
-----

    python3 scripts/memory-retrieval-eval.py --self-test

    cp ~/.librefang/memory.db /tmp/corpus.db
    export LIBREFANG_EVAL_JUDGE_URL=https://.../v1/chat/completions
    export LIBREFANG_EVAL_JUDGE_MODEL=...
    export LIBREFANG_EVAL_JUDGE_KEY=...
    export LIBREFANG_EVAL_EMBED_URL=https://.../v1/embeddings
    export LIBREFANG_EVAL_EMBED_MODEL=...
    python3 scripts/memory-retrieval-eval.py run \\
        --corpus /tmp/corpus.db --queries queries.txt --arms cosine,fts5,rrf \\
        --out run-bge-m3.json

    python3 scripts/memory-retrieval-eval.py --noise-floor run-a.json run-b.json --stable-arms fts5

To compare a different embedding model, re-embed the corpus rather than scoring new queries against old vectors:

    python3 scripts/memory-retrieval-eval.py run \\
        --corpus /tmp/corpus.db --queries queries.txt --arms cosine,fts5 --reembed \\
        --embed-model intfloat/multilingual-e5-large --query-prefix 'query: ' --passage-prefix 'passage: ' \\
        --out run-e5.json

Stdlib only, on purpose: no virtualenv, no requirements file, `python3 scripts/memory-retrieval-eval.py` and it runs, exactly like every other tool in `scripts/`.

Credit for the method, the corpus statistics and the noise-floor diagnostic: @nevgenov, on #7756 and #7912.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import random
import sqlite3
import struct
import sys
import unittest
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from typing import Callable, Iterable, Sequence

# ── Constants ────────────────────────────────────────────────────────────────────────────────────────────

#: Rank cutoff for the reported metric.
#: Ten because that is the pool depth the judge grades.
NDCG_K = 10

#: Judge grades outside this range are discarded whole rather than clamped — see `parse_judge_grade`.
GRADE_MIN = 0
GRADE_MAX = 3

#: Resamples for the paired bootstrap.
#: 10_000 is enough that the interval is stable to the third decimal, which is a finer resolution than the method's ~0.04 noise floor can use anyway.
BOOTSTRAP_ITERATIONS = 10_000

#: Two-sided interval width.
BOOTSTRAP_ALPHA = 0.05

#: Reported alongside every comparison so nobody reads a 0.02 difference as a finding.
#: Recompute it for your own setup with `--noise-floor`; this default is the figure @nevgenov measured across five runs on #7756 and should not be treated as universal.
DEFAULT_NOISE_FLOOR = 0.04

#: Below this many counted queries the paired bootstrap resamples too few distinct values for its interval to mean much, so the report says so rather than letting a confident-looking interval stand unqualified.
#: The runs on #7756 counted 59-80.
MIN_USEFUL_QUERIES = 30

#: Reciprocal-rank-fusion damping.
#: 60 is the constant from the original RRF paper and the one every reference implementation uses; it is exposed as a flag rather than tuned here, because tuning it on one corpus is the same unvalidated bet this harness exists to expose.
RRF_K = 60

#: Documents are truncated before being shown to the judge.
#: A 200_000-character PDF row (the largest observed on the reporter's corpus) would otherwise dominate the judge's context and cost.
JUDGE_DOC_MAX_CHARS = 4_000

JUDGE_SYSTEM_PROMPT = (
    "You grade how useful a stored memory fragment is for answering a user's message. "
    "Reply with a single integer and nothing else.\n"
    "3 = directly answers or is essential context.\n"
    "2 = clearly relevant, partially useful.\n"
    "1 = marginally related.\n"
    "0 = irrelevant."
)


# ── Corpus ───────────────────────────────────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Document:
    """One row of the memory corpus, as the retrieval arms see it."""

    id: str
    content: str
    scope: str
    source: str
    #: Decoded from the stored BLOB.
    #: `None` for a text-only row, which the cosine arm cannot rank.
    embedding: tuple[float, ...] | None


def decode_embedding(blob: bytes | None) -> tuple[float, ...] | None:
    """Decode the little-endian f32 array LibreFang stores in `memories.embedding`.

    Mirrors `embedding_from_bytes` in `crates/librefang-memory/src/semantic.rs`, including its behaviour on a trailing partial float: the Rust side uses `as_chunks::<4>()` and drops the remainder, so a truncated blob yields a short vector rather than an error.
    Keeping that quirk means a corrupt row ranks the same here as it does in the daemon.
    """
    if not blob:
        return None
    count = len(blob) // 4
    if count == 0:
        return None
    return struct.unpack(f"<{count}f", blob[: count * 4])


def load_corpus(db_path: str, agent_id: str | None = None) -> list[Document]:
    """Read every live memory row from a read-only, immutable copy of the substrate.

    `immutable=1` is not decoration: it tells SQLite the file cannot change underneath it, which is only true because you copied it.
    Point this at a live daemon database and you are reading a file being written through WAL, which is both wrong and the kind of wrong that produces plausible numbers.
    """
    if not os.path.exists(db_path):
        raise SystemExit(f"corpus not found: {db_path}")
    uri = f"file:{urllib.request.pathname2url(os.path.abspath(db_path))}?mode=ro&immutable=1"
    conn = sqlite3.connect(uri, uri=True)
    try:
        sql = "SELECT id, content, scope, source, embedding FROM memories WHERE deleted = 0"
        params: tuple[str, ...] = ()
        if agent_id:
            sql += " AND agent_id = ?"
            params = (agent_id,)
        rows = conn.execute(sql, params).fetchall()
    finally:
        conn.close()
    return [
        Document(id=r[0], content=r[1] or "", scope=r[2] or "", source=r[3] or "", embedding=decode_embedding(r[4]))
        for r in rows
    ]


def fts_rank_ids(db_path: str, query: str, limit: int, agent_id: str | None = None) -> list[str]:
    """Rank document ids with the daemon's own FTS5 index.

    This is deliberately not a reimplementation of BM25.
    `memories_fts` is built and kept in step by migration v50 (#7808) and `ORDER BY rank` on an FTS5 table is FTS5's bm25 — so the lexical arm measured here is the lexical path the daemon would actually take, not a lookalike that can drift away from it.

    Returns an empty list when the index is absent (a corpus copied from a pre-v50 database) or when the query contains no indexable token, which is the same "no usable pre-selection" signal `fts_candidate_ids` uses in `semantic.rs`.
    """
    uri = f"file:{urllib.request.pathname2url(os.path.abspath(db_path))}?mode=ro&immutable=1"
    conn = sqlite3.connect(uri, uri=True)
    try:
        match = fts_match_expression(query)
        if not match:
            return []
        sql = "SELECT memory_id FROM memories_fts WHERE memories_fts MATCH ?"
        params: list[object] = [match]
        if agent_id:
            sql += " AND agent_id = ?"
            params.append(agent_id)
        sql += " ORDER BY rank LIMIT ?"
        params.append(limit)
        return [r[0] for r in conn.execute(sql, params).fetchall()]
    except sqlite3.OperationalError:
        # No `memories_fts` table, or a MATCH expression FTS5 rejects.
        # Either way the lexical arm has nothing to say about this query; returning [] lets the run continue with the other arms rather than aborting a several-hundred-call judging session.
        return []
    finally:
        conn.close()


def fts_match_expression(query: str) -> str:
    """Turn free text into an FTS5 MATCH expression that cannot be a syntax error.

    Every token is double-quoted, so `AND`, `NEAR`, `*`, `-` and stray quotes in a real user message are data rather than operators.
    Tokens are OR-ed, matching FTS5's default behaviour for a bare token list while making the intent explicit.
    """
    tokens = [t for t in "".join(c if c.isalnum() else " " for c in query).split() if t]
    if not tokens:
        return ""
    return " OR ".join(f'"{t}"' for t in tokens)


# ── Arms ─────────────────────────────────────────────────────────────────────────────────────────────────


@dataclass
class Arm:
    """One retrieval strategy under test.

    `stable_under_embedder_change` marks an arm whose score cannot move when only the vector arm changes.
    Those arms are the instrument for `--noise-floor`: any movement they show between two such runs is pool churn, not signal.
    """

    name: str
    rank: Callable[[str], list[str]]
    stable_under_embedder_change: bool = False


def cosine_similarity(a: Sequence[float], b: Sequence[float]) -> float | None:
    """Port of `librefang_types::memory::cosine_similarity`, `None` semantics included.

    Returning `None` rather than 0.0 for an incomparable pair is load-bearing on both sides: 0.0 means "fully dissimilar", so folding "not comparable" into it silently corrupts the ranking (#3536).
    """
    if len(a) != len(b) or not a:
        return None
    dot = norm_a = norm_b = 0.0
    for x, y in zip(a, b):
        dot += x * y
        norm_a += x * x
        norm_b += y * y
    denom = math.sqrt(norm_a) * math.sqrt(norm_b)
    if denom < 1e-12:
        return None
    return dot / denom


def cosine_arm(docs: Sequence[Document], embed: Callable[[str], Sequence[float]], top_k: int) -> Callable[[str], list[str]]:
    """The embedding path the runtime actually takes: cosine against the stored vector, one sort key, no tiebreak.

    `semantic.rs` sorts on `similarity` alone and gives an unscorable row `f32::NEG_INFINITY`.
    Reproduced here by dropping unscorable rows entirely, which is the same outcome once a `min_similarity` floor or a top-k cut is applied.
    """

    def rank(query: str) -> list[str]:
        qe = embed(query)
        scored: list[tuple[float, str]] = []
        for doc in docs:
            if doc.embedding is None:
                continue
            sim = cosine_similarity(qe, doc.embedding)
            if sim is not None:
                scored.append((sim, doc.id))
        scored.sort(key=lambda p: (-p[0], p[1]))
        return [doc_id for _, doc_id in scored[:top_k]]

    return rank


def fts_arm(db_path: str, top_k: int, agent_id: str | None) -> Callable[[str], list[str]]:
    """The lexical path: FTS5 bm25 over `memories_fts`."""

    def rank(query: str) -> list[str]:
        return fts_rank_ids(db_path, query, top_k, agent_id)

    return rank


def reciprocal_rank_fusion(rankings: Sequence[Sequence[str]], weights: Sequence[float], k: int, top_k: int) -> list[str]:
    """Weighted RRF over several ranked id lists.

    Fusion is offered because the RFC proposed it, not because it is recommended: across five runs it never reached significance over cosine alone in either direction.
    It is here so the next person can settle that on their own corpus instead of inheriting either verdict.
    """
    scores: dict[str, float] = {}
    for ranking, weight in zip(rankings, weights):
        for position, doc_id in enumerate(ranking):
            scores[doc_id] = scores.get(doc_id, 0.0) + weight / (k + position + 1)
    ordered = sorted(scores.items(), key=lambda p: (-p[1], p[0]))
    return [doc_id for doc_id, _ in ordered[:top_k]]


# ── Judging ──────────────────────────────────────────────────────────────────────────────────────────────


def build_pool(rankings: dict[str, Sequence[str]], top_k: int, rng: random.Random) -> list[str]:
    """Union every arm's top-k, then shuffle.

    Two properties the judge depends on and one the reader does.
    The pool is a *set* union, so a document retrieved by three arms is graded once and every arm reads the same grade — that is what makes the per-query differences paired.
    The shuffle hides provenance and position, so the judge cannot infer which arm produced a document or how highly it ranked it.
    And the shuffle is seeded, so a re-run with a warm judge cache reproduces the same pool exactly.

    The pool being the union is also why this method has a noise floor: adding an arm changes what the judge sees for every arm, so scores move even for arms that did not change.
    """
    seen: list[str] = []
    marked: set[str] = set()
    for arm_name in sorted(rankings):
        for doc_id in list(rankings[arm_name])[:top_k]:
            if doc_id not in marked:
                marked.add(doc_id)
                seen.append(doc_id)
    rng.shuffle(seen)
    return seen


def parse_judge_grade(raw: str) -> int | None:
    """Parse a judge response into a grade, or `None` to discard the pair.

    Out-of-range and unparseable responses are discarded *whole* rather than clamped.
    Clamping a 26 to a 3 substitutes plausible data for bad data, and the resulting number looks exactly like a real judgment; that cost @nevgenov 2 of 80 queries on one corpus and it is better to lose them visibly.
    A query is dropped from the run entirely if any of its pairs fail, so every arm is scored over the same pairs and the comparison stays paired.
    """
    text = raw.strip()
    if not text:
        return None
    digits = "".join(c for c in text if c.isdigit() or c == "-")
    if not digits:
        return None
    try:
        value = int(digits)
    except ValueError:
        return None
    if value < GRADE_MIN or value > GRADE_MAX:
        return None
    return value


class JudgeCache:
    """On-disk judgment cache keyed by (judge model, query, document text).

    Keyed by content, not by document id, so a re-run after the corpus changed does not silently reuse a grade for text that no longer exists.
    Keyed by judge model, so switching judges — the limitation every run in #7756 carries, since all five shared one — starts from cold rather than mixing two judges into one number.

    This is the difference between "adding an arm costs one full re-spend" and "adding an arm costs the pairs it newly contributes", which is the practical reason five runs was as far as anyone got.
    """

    def __init__(self, path: str | None) -> None:
        self.path = path
        self.entries: dict[str, int] = {}
        if path and os.path.exists(path):
            with open(path, "r", encoding="utf-8") as handle:
                self.entries = json.load(handle)

    @staticmethod
    def key(judge_model: str, query: str, document: str) -> str:
        digest = hashlib.sha256()
        digest.update(judge_model.encode("utf-8"))
        digest.update(b"\x00")
        digest.update(query.encode("utf-8"))
        digest.update(b"\x00")
        digest.update(document.encode("utf-8"))
        return digest.hexdigest()

    def get(self, key: str) -> int | None:
        return self.entries.get(key)

    def put(self, key: str, grade: int) -> None:
        self.entries[key] = grade

    def flush(self) -> None:
        if not self.path:
            return
        tmp = f"{self.path}.tmp"
        with open(tmp, "w", encoding="utf-8") as handle:
            json.dump(self.entries, handle)
        os.replace(tmp, self.path)


def http_json(url: str, payload: dict, api_key: str | None, timeout: int = 120) -> dict:
    """POST JSON and return JSON, using only the standard library."""
    body = json.dumps(payload).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"
    request = urllib.request.Request(url, data=body, headers=headers, method="POST")
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


# ── Metrics ──────────────────────────────────────────────────────────────────────────────────────────────


def dcg(gains: Iterable[float]) -> float:
    """Discounted cumulative gain with the standard `(2^g - 1) / log2(i + 2)` formulation."""
    return sum((2.0**g - 1.0) / math.log2(index + 2.0) for index, g in enumerate(gains))


def ndcg_at_k(ranked_ids: Sequence[str], grades: dict[str, int], k: int = NDCG_K) -> float:
    """nDCG@k for one arm on one query.

    The ideal ranking is taken over the judged pool, which is the union of every arm's list.
    So a perfect score means "this arm put the pool's best documents on top", not "this arm found everything relevant in the corpus" — the corpus is never exhaustively judged and cannot be at this size.
    An arm that retrieves fewer than k documents is not padded: the missing slots contribute zero gain, which is the correct penalty for returning nothing.
    """
    actual = dcg(grades.get(doc_id, 0) for doc_id in ranked_ids[:k])
    ideal = dcg(sorted(grades.values(), reverse=True)[:k])
    if ideal <= 0.0:
        return 0.0
    return actual / ideal


def bootstrap_paired_ci(
    diffs: Sequence[float],
    iterations: int = BOOTSTRAP_ITERATIONS,
    alpha: float = BOOTSTRAP_ALPHA,
    seed: int = 0,
) -> tuple[float, float, float]:
    """Bootstrap a confidence interval over paired per-query differences.

    Resamples *queries*, not observations.
    Between-query variance dwarfs between-arm variance on corpora of this size, so an unpaired comparison cannot detect the effect at all — the interval is swamped by how hard each query happens to be.
    Pairing removes that variance because both arms are scored on the same query against the same judged pool.

    Returns `(mean, low, high)`.
    The difference is significant at `alpha` when the interval excludes zero, and is a *result* only when it also clears the noise floor.
    """
    if not diffs:
        raise ValueError("paired bootstrap needs at least one per-query difference")
    rng = random.Random(seed)
    n = len(diffs)
    means: list[float] = []
    for _ in range(iterations):
        total = 0.0
        for _ in range(n):
            total += diffs[rng.randrange(n)]
        means.append(total / n)
    means.sort()
    low_index = int((alpha / 2.0) * iterations)
    high_index = min(iterations - 1, int((1.0 - alpha / 2.0) * iterations))
    return (sum(diffs) / n, means[low_index], means[high_index])


def verdict(low: float, high: float, noise_floor: float) -> str:
    """Classify a comparison.

    Three outcomes, and the middle one is the one people skip.

    An interval that excludes zero but sits inside the noise floor is *significant and not a result*: the method itself moves scores by that much when only the pool composition changes.
    """
    if low <= 0.0 <= high:
        return "noise"
    if max(abs(low), abs(high)) < noise_floor:
        return "significant but below the noise floor"
    return "significant"


# ── Reporting ────────────────────────────────────────────────────────────────────────────────────────────


@dataclass
class RunResult:
    arms: list[str] = field(default_factory=list)
    stable_arms: list[str] = field(default_factory=list)
    #: arm -> query -> nDCG@10
    per_query: dict[str, dict[str, float]] = field(default_factory=dict)
    queries_counted: int = 0
    queries_dropped: int = 0
    pairs_judged: int = 0
    noise_floor: float = DEFAULT_NOISE_FLOOR
    metadata: dict = field(default_factory=dict)

    def mean_ndcg(self) -> dict[str, float]:
        return {
            arm: (sum(scores.values()) / len(scores) if scores else 0.0)
            for arm, scores in self.per_query.items()
        }

    def to_json(self) -> dict:
        return {
            "arms": self.arms,
            "stable_arms": self.stable_arms,
            "per_query": self.per_query,
            "queries_counted": self.queries_counted,
            "queries_dropped": self.queries_dropped,
            "pairs_judged": self.pairs_judged,
            "noise_floor": self.noise_floor,
            "mean_ndcg": self.mean_ndcg(),
            "metadata": self.metadata,
        }


def render_markdown(result: RunResult, seed: int) -> str:
    """Render the result as a markdown table.

    The output format is a deliberate choice: the five runs that produced everything known about this subsystem were reported by pasting tables into a GitHub issue, and that is the workflow this tool is meant to keep working.
    """
    means = result.mean_ndcg()
    if not means:
        return "No arms scored.\n"
    ordered = sorted(means.items(), key=lambda p: (-p[1], p[0]))
    leader, leader_score = ordered[0]
    lines = [
        f"{result.queries_counted} queries counted "
        f"({result.queries_dropped} dropped), {result.pairs_judged} judged pairs, "
        f"seed {seed}, noise floor {result.noise_floor:.3f}.",
        "",
        "| arm | nDCG@10 | vs leader |",
        "|---|---|---|",
    ]
    for arm, score in ordered:
        if arm == leader:
            lines.append(f"| {arm} | {score:.4f} | — |")
            continue
        diffs = [
            result.per_query[leader][query] - result.per_query[arm][query]
            for query in sorted(result.per_query[leader])
            if query in result.per_query[arm]
        ]
        mean, low, high = bootstrap_paired_ci(diffs, seed=seed)
        lines.append(
            f"| {arm} | {score:.4f} | +{mean:.4f} [{low:+.4f}, {high:+.4f}] "
            f"{verdict(low, high, result.noise_floor)} |"
        )
    lines.append("")
    if result.queries_counted < MIN_USEFUL_QUERIES:
        lines.append(
            f"**Only {result.queries_counted} queries counted.** "
            f"Below about {MIN_USEFUL_QUERIES} the bootstrap has too few distinct per-query differences to "
            "resample, so treat every interval here as illustrative rather than as evidence."
        )
        lines.append("")
    lines.append(
        f"Leader: {leader} at {leader_score:.4f}. "
        "Intervals are bootstrapped over paired per-query differences against the leader; "
        "a difference below the noise floor is not a result even when the interval excludes zero."
    )
    return "\n".join(lines) + "\n"


def noise_floor_from_runs(run_a: dict, run_b: dict, stable_arms: Sequence[str]) -> tuple[float, dict[str, float]]:
    """Measure this method's noise floor from two runs, using arms that could not have changed.

    An arm declared stable — a lexical arm, when the changed variable is the embedding model — has no path by which its own scores can move between the two runs.
    Whatever movement it shows is the judged pool shifting underneath it, because the pool is the union of all arms and one of them changed.
    That movement is the resolution limit of the whole method, and every difference below it should be discounted, including differences the runs themselves report as significant.
    """
    means_a = run_a.get("mean_ndcg", {})
    means_b = run_b.get("mean_ndcg", {})
    movements: dict[str, float] = {}
    for arm in stable_arms:
        if arm in means_a and arm in means_b:
            movements[arm] = means_b[arm] - means_a[arm]
    if not movements:
        raise SystemExit(
            "none of the declared stable arms appear in both runs; "
            f"run A has {sorted(means_a)}, run B has {sorted(means_b)}"
        )
    return max(abs(v) for v in movements.values()), movements


# ── Run ──────────────────────────────────────────────────────────────────────────────────────────────────


def embed_texts(args: argparse.Namespace, texts: Sequence[str]) -> list[Sequence[float]]:
    """Embed a batch of texts through the configured OpenAI-compatible endpoint."""
    url = args.embed_url or os.environ.get("LIBREFANG_EVAL_EMBED_URL")
    model = args.embed_model or os.environ.get("LIBREFANG_EVAL_EMBED_MODEL")
    key = os.environ.get("LIBREFANG_EVAL_EMBED_KEY")
    if not url or not model:
        raise SystemExit(
            "embedding needs an endpoint: set LIBREFANG_EVAL_EMBED_URL and LIBREFANG_EVAL_EMBED_MODEL "
            "(or --embed-url / --embed-model)"
        )
    response = http_json(url, {"model": model, "input": list(texts)}, key)
    return [item["embedding"] for item in response["data"]]


def reembed_corpus(docs: Sequence[Document], args: argparse.Namespace) -> list[Document]:
    """Rebuild every document vector with the configured embedding model, caching to disk.

    This is what makes the control run possible — the run that overturned the headline finding on #7756 by changing only the embedder.
    It is also the run that must never be done halfway: a query embedded by one model and scored against vectors written by another compares two unrelated coordinate systems and produces confident nonsense.
    So this replaces *every* vector or fails; it never mixes stored and re-embedded rows.

    `--passage-prefix` is applied here and `--query-prefix` at query time, because that asymmetry is the whole point of the e5-family contract.
    The cache is keyed by model, prefix and document text, so re-running a judged sweep after the first embedding pass costs nothing.
    """
    cache: dict[str, list[float]] = {}
    if args.embed_cache and os.path.exists(args.embed_cache):
        with open(args.embed_cache, "r", encoding="utf-8") as handle:
            cache = json.load(handle)
    model = args.embed_model or os.environ.get("LIBREFANG_EVAL_EMBED_MODEL", "")
    prefix = args.passage_prefix

    def cache_key(text: str) -> str:
        digest = hashlib.sha256()
        digest.update(model.encode("utf-8"))
        digest.update(b"\x00")
        digest.update(prefix.encode("utf-8"))
        digest.update(b"\x00")
        digest.update(text.encode("utf-8"))
        return digest.hexdigest()

    pending = [doc for doc in docs if cache_key(doc.content) not in cache]
    print(f"re-embedding {len(pending)} of {len(docs)} rows with {model!r}", file=sys.stderr)
    for start in range(0, len(pending), args.embed_batch):
        batch = pending[start : start + args.embed_batch]
        vectors = embed_texts(args, [f"{prefix}{doc.content}" for doc in batch])
        if len(vectors) != len(batch):
            raise SystemExit(
                f"embedding endpoint returned {len(vectors)} vectors for {len(batch)} inputs; "
                "refusing to guess which vector belongs to which document"
            )
        for doc, vector in zip(batch, vectors):
            cache[cache_key(doc.content)] = list(vector)
        if args.embed_cache:
            tmp = f"{args.embed_cache}.tmp"
            with open(tmp, "w", encoding="utf-8") as handle:
                json.dump(cache, handle)
            os.replace(tmp, args.embed_cache)
        print(f"  {min(start + args.embed_batch, len(pending))}/{len(pending)}", file=sys.stderr)

    return [
        Document(
            id=doc.id,
            content=doc.content,
            scope=doc.scope,
            source=doc.source,
            embedding=tuple(cache[cache_key(doc.content)]),
        )
        for doc in docs
    ]


def make_embedder(args: argparse.Namespace) -> Callable[[str], Sequence[float]]:
    """Embed a query through an OpenAI-compatible `/v1/embeddings` endpoint.

    `--query-prefix` exists because a large part of the current embedding landscape, the e5 family among it, is trained on an asymmetric `query:` / `passage:` pair.
    The daemon cannot send prefixes today (#7912, gap 3), so measuring what they are worth is only possible here — and omitting them when the model expects them narrows the relevant-vs-irrelevant margin enough to produce a false confirmation of whatever hypothesis is being tested.
    """
    url = args.embed_url or os.environ.get("LIBREFANG_EVAL_EMBED_URL")
    model = args.embed_model or os.environ.get("LIBREFANG_EVAL_EMBED_MODEL")
    key = os.environ.get("LIBREFANG_EVAL_EMBED_KEY")
    if not url or not model:
        raise SystemExit(
            "the cosine arm needs an embedding endpoint: set LIBREFANG_EVAL_EMBED_URL and "
            "LIBREFANG_EVAL_EMBED_MODEL (or --embed-url / --embed-model)"
        )
    cache: dict[str, Sequence[float]] = {}

    def embed(text: str) -> Sequence[float]:
        prefixed = f"{args.query_prefix}{text}" if args.query_prefix else text
        if prefixed in cache:
            return cache[prefixed]
        response = http_json(url, {"model": model, "input": prefixed}, key)
        vector = response["data"][0]["embedding"]
        cache[prefixed] = vector
        return vector

    return embed


def make_judge(args: argparse.Namespace) -> Callable[[str, str], int | None]:
    url = args.judge_url or os.environ.get("LIBREFANG_EVAL_JUDGE_URL")
    model = args.judge_model or os.environ.get("LIBREFANG_EVAL_JUDGE_MODEL")
    key = os.environ.get("LIBREFANG_EVAL_JUDGE_KEY")
    if not url or not model:
        raise SystemExit(
            "judging needs an endpoint: set LIBREFANG_EVAL_JUDGE_URL and LIBREFANG_EVAL_JUDGE_MODEL "
            "(or --judge-url / --judge-model)"
        )
    cache = JudgeCache(args.judge_cache)

    def judge(query: str, document: str) -> int | None:
        clipped = document[:JUDGE_DOC_MAX_CHARS]
        cache_key = JudgeCache.key(model, query, clipped)
        cached = cache.get(cache_key)
        if cached is not None:
            return cached
        payload = {
            "model": model,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": JUDGE_SYSTEM_PROMPT},
                {"role": "user", "content": f"User message:\n{query}\n\nStored memory fragment:\n{clipped}\n\nGrade:"},
            ],
        }
        try:
            response = http_json(url, payload, key)
            raw = response["choices"][0]["message"]["content"]
        except (urllib.error.URLError, KeyError, IndexError, json.JSONDecodeError) as exc:
            print(f"  judge call failed, discarding pair: {exc}", file=sys.stderr)
            return None
        grade = parse_judge_grade(raw)
        if grade is None:
            print(f"  unparseable judge response, discarding pair: {raw!r}", file=sys.stderr)
            return None
        cache.put(cache_key, grade)
        cache.flush()
        return grade

    return judge


def run(args: argparse.Namespace) -> int:
    rng = random.Random(args.seed)
    docs = load_corpus(args.corpus, args.agent)
    if not docs:
        raise SystemExit("corpus contains no live memory rows")
    by_id = {doc.id: doc for doc in docs}
    print(f"corpus: {len(docs)} rows, {sum(1 for d in docs if d.embedding)} with a stored vector", file=sys.stderr)

    if args.reembed:
        docs = reembed_corpus(docs, args)
        by_id = {doc.id: doc for doc in docs}

    with open(args.queries, "r", encoding="utf-8") as handle:
        queries = [line.strip() for line in handle if line.strip()]
    if not queries:
        raise SystemExit(f"no queries in {args.queries}")

    requested = [name.strip() for name in args.arms.split(",") if name.strip()]
    arms: list[Arm] = []
    for name in requested:
        if name == "cosine":
            arms.append(Arm("cosine", cosine_arm(docs, make_embedder(args), args.top_k)))
        elif name == "fts5":
            arms.append(Arm("fts5", fts_arm(args.corpus, args.top_k, args.agent), stable_under_embedder_change=True))
        elif name == "rrf":
            cosine_rank = cosine_arm(docs, make_embedder(args), args.top_k)
            lexical_rank = fts_arm(args.corpus, args.top_k, args.agent)
            weights = [args.rrf_weight, 1.0 - args.rrf_weight]
            arms.append(
                Arm(
                    f"rrf {args.rrf_weight:.1f}/{1.0 - args.rrf_weight:.1f}",
                    lambda q, cr=cosine_rank, lr=lexical_rank, w=weights: reciprocal_rank_fusion(
                        [cr(q), lr(q)], w, args.rrf_k, args.top_k
                    ),
                )
            )
        else:
            raise SystemExit(f"unknown arm: {name} (known: cosine, fts5, rrf)")

    judge = make_judge(args)
    result = RunResult(
        arms=[arm.name for arm in arms],
        stable_arms=[arm.name for arm in arms if arm.stable_under_embedder_change],
        noise_floor=args.noise_floor_value,
        metadata={
            "corpus": os.path.basename(args.corpus),
            "rows": len(docs),
            "top_k": args.top_k,
            "query_prefix": args.query_prefix,
            "passage_prefix": args.passage_prefix,
            "reembedded": args.reembed,
            "embed_model": args.embed_model or os.environ.get("LIBREFANG_EVAL_EMBED_MODEL", ""),
            "judge_model": args.judge_model or os.environ.get("LIBREFANG_EVAL_JUDGE_MODEL", ""),
        },
    )
    result.per_query = {arm.name: {} for arm in arms}

    for index, query in enumerate(queries, start=1):
        print(f"[{index}/{len(queries)}] {query[:70]}", file=sys.stderr)
        rankings = {arm.name: arm.rank(query) for arm in arms}
        pool = build_pool(rankings, args.top_k, rng)
        if not pool:
            result.queries_dropped += 1
            continue
        grades: dict[str, int] = {}
        dropped = False
        for doc_id in pool:
            grade = judge(query, by_id[doc_id].content)
            if grade is None:
                # One bad pair drops the whole query.
                # Scoring the remaining arms over a pool the others were not scored on would break the pairing the intervals depend on.
                dropped = True
                break
            grades[doc_id] = grade
        if dropped:
            result.queries_dropped += 1
            continue
        result.pairs_judged += len(grades)
        result.queries_counted += 1
        for arm in arms:
            result.per_query[arm.name][query] = ndcg_at_k(rankings[arm.name], grades, NDCG_K)

    report = render_markdown(result, args.seed)
    print(report)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as handle:
            json.dump(result.to_json(), handle, indent=2)
        print(f"per-query scores written to {args.out}", file=sys.stderr)
    return 0


def noise_floor_command(args: argparse.Namespace) -> int:
    with open(args.noise_floor[0], "r", encoding="utf-8") as handle:
        run_a = json.load(handle)
    with open(args.noise_floor[1], "r", encoding="utf-8") as handle:
        run_b = json.load(handle)
    declared = [name.strip() for name in args.stable_arms.split(",") if name.strip()]
    if not declared:
        declared = run_a.get("stable_arms", [])
    floor, movements = noise_floor_from_runs(run_a, run_b, declared)
    print("Movement in arms that could not have changed between these two runs:")
    for arm, delta in sorted(movements.items()):
        print(f"  {arm:24s} {delta:+.4f}")
    print(f"\nNoise floor for this setup: {floor:.4f}")
    print("Discount any reported difference smaller than that, including differences reported as significant.")
    return 0


# ── Self-test ────────────────────────────────────────────────────────────────────────────────────────────


class SelfTest(unittest.TestCase):
    """Hermetic coverage of the arithmetic.

    No network, no corpus, no secret, about a second.

    This is the half of the harness that can be wrong without anyone noticing, because a wrong nDCG or a mis-specified bootstrap produces numbers that look exactly like right ones.
    """

    def test_ndcg_perfect_ranking_scores_one(self) -> None:
        grades = {"a": 3, "b": 2, "c": 1, "d": 0}
        self.assertAlmostEqual(ndcg_at_k(["a", "b", "c", "d"], grades), 1.0)

    def test_ndcg_reversed_ranking_scores_below_perfect(self) -> None:
        grades = {"a": 3, "b": 2, "c": 1, "d": 0}
        self.assertLess(ndcg_at_k(["d", "c", "b", "a"], grades), ndcg_at_k(["a", "b", "c", "d"], grades))

    def test_ndcg_is_zero_when_nothing_in_the_pool_is_relevant(self) -> None:
        self.assertEqual(ndcg_at_k(["a", "b"], {"a": 0, "b": 0}), 0.0)

    def test_ndcg_does_not_pad_a_short_ranking(self) -> None:
        """An arm that returns two documents is penalised for the eight slots it left empty."""
        grades = {f"d{i}": 3 for i in range(NDCG_K)}
        short = ndcg_at_k(["d0", "d1"], grades)
        full = ndcg_at_k([f"d{i}" for i in range(NDCG_K)], grades)
        self.assertLess(short, full)
        self.assertAlmostEqual(full, 1.0)

    def test_ndcg_ignores_documents_beyond_k(self) -> None:
        grades = {f"d{i}": 3 for i in range(NDCG_K + 5)}
        ranked = [f"d{i}" for i in range(NDCG_K + 5)]
        self.assertAlmostEqual(ndcg_at_k(ranked, grades, NDCG_K), 1.0)

    def test_ndcg_treats_an_unjudged_document_as_zero_gain(self) -> None:
        self.assertEqual(ndcg_at_k(["missing"], {"a": 3}), 0.0)

    def test_paired_bootstrap_interval_brackets_the_mean(self) -> None:
        diffs = [0.05, 0.06, 0.04, 0.05, 0.07, 0.03, 0.05, 0.06]
        mean, low, high = bootstrap_paired_ci(diffs, iterations=2000, seed=7)
        self.assertAlmostEqual(mean, sum(diffs) / len(diffs))
        self.assertLess(low, mean)
        self.assertGreater(high, mean)

    def test_paired_bootstrap_detects_a_consistent_small_effect(self) -> None:
        """The whole reason for pairing: a small effect present on every query is significant even though the per-query scores themselves are all over the place."""
        diffs = [0.02] * 40
        _, low, high = bootstrap_paired_ci(diffs, iterations=2000, seed=7)
        self.assertGreater(low, 0.0)
        self.assertLess(high, 0.05)

    def test_paired_bootstrap_calls_a_wandering_difference_noise(self) -> None:
        rng = random.Random(11)
        diffs = [rng.uniform(-0.3, 0.3) for _ in range(40)]
        _, low, high = bootstrap_paired_ci(diffs, iterations=2000, seed=7)
        self.assertLessEqual(low, 0.0)
        self.assertGreaterEqual(high, 0.0)

    def test_paired_bootstrap_is_deterministic_for_a_seed(self) -> None:
        diffs = [0.1, -0.2, 0.3, 0.05]
        first = bootstrap_paired_ci(diffs, iterations=500, seed=3)
        second = bootstrap_paired_ci(diffs, iterations=500, seed=3)
        self.assertEqual(first, second)

    def test_paired_bootstrap_rejects_an_empty_input(self) -> None:
        with self.assertRaises(ValueError):
            bootstrap_paired_ci([])

    def test_verdict_separates_significant_from_below_the_floor(self) -> None:
        self.assertEqual(verdict(-0.01, 0.05, 0.04), "noise")
        self.assertEqual(verdict(0.005, 0.02, 0.04), "significant but below the noise floor")
        self.assertEqual(verdict(0.05, 0.12, 0.04), "significant")

    def test_pool_is_a_union_so_a_shared_document_is_graded_once(self) -> None:
        pool = build_pool({"a": ["x", "y"], "b": ["y", "z"]}, 10, random.Random(1))
        self.assertEqual(sorted(pool), ["x", "y", "z"])

    def test_pool_shuffle_is_seeded_and_reproducible(self) -> None:
        rankings = {"a": [f"d{i}" for i in range(10)], "b": [f"e{i}" for i in range(10)]}
        first = build_pool(rankings, 10, random.Random(42))
        second = build_pool(rankings, 10, random.Random(42))
        self.assertEqual(first, second)

    def test_pool_shuffle_actually_hides_provenance(self) -> None:
        """Insertion order is arm-then-rank; if the shuffle were a no-op the judge could read both off the pool."""
        rankings = {"a": [f"d{i}" for i in range(20)], "b": [f"e{i}" for i in range(20)]}
        pool = build_pool(rankings, 20, random.Random(5))
        insertion_order = [f"d{i}" for i in range(20)] + [f"e{i}" for i in range(20)]
        self.assertNotEqual(pool, insertion_order)

    def test_pool_respects_top_k(self) -> None:
        pool = build_pool({"a": [f"d{i}" for i in range(50)]}, 10, random.Random(1))
        self.assertEqual(len(pool), 10)

    def test_judge_grade_parses_a_bare_integer(self) -> None:
        self.assertEqual(parse_judge_grade("2"), 2)
        self.assertEqual(parse_judge_grade("  3\n"), 3)
        self.assertEqual(parse_judge_grade("0"), 0)

    def test_judge_grade_discards_out_of_range_rather_than_clamping(self) -> None:
        """A 26 clamped to 3 is indistinguishable from a real 3, which is why it is dropped instead."""
        self.assertIsNone(parse_judge_grade("26"))
        self.assertIsNone(parse_judge_grade("-1"))

    def test_judge_grade_discards_unparseable(self) -> None:
        self.assertIsNone(parse_judge_grade(""))
        self.assertIsNone(parse_judge_grade("I would say it is quite relevant"))

    def test_cosine_matches_the_rust_none_semantics(self) -> None:
        self.assertIsNone(cosine_similarity([1.0, 0.0], [1.0, 0.0, 0.0]))
        self.assertIsNone(cosine_similarity([], []))
        self.assertIsNone(cosine_similarity([0.0, 0.0], [1.0, 1.0]))
        self.assertAlmostEqual(cosine_similarity([1.0, 0.0], [1.0, 0.0]), 1.0)
        self.assertAlmostEqual(cosine_similarity([1.0, 0.0], [0.0, 1.0]), 0.0)

    def test_embedding_blob_round_trips_the_rust_encoding(self) -> None:
        values = [0.5, -0.25, 1.0]
        blob = struct.pack(f"<{len(values)}f", *values)
        decoded = decode_embedding(blob)
        assert decoded is not None
        for got, want in zip(decoded, values):
            self.assertAlmostEqual(got, want, places=6)

    def test_embedding_blob_drops_a_trailing_partial_float_like_rust_does(self) -> None:
        blob = struct.pack("<2f", 1.0, 2.0) + b"\x00\x00"
        decoded = decode_embedding(blob)
        assert decoded is not None
        self.assertEqual(len(decoded), 2)

    def test_embedding_blob_of_none_or_empty_is_none(self) -> None:
        self.assertIsNone(decode_embedding(None))
        self.assertIsNone(decode_embedding(b""))
        self.assertIsNone(decode_embedding(b"\x00\x00"))

    def test_fts_match_expression_quotes_every_token(self) -> None:
        self.assertEqual(fts_match_expression("hello world"), '"hello" OR "world"')

    def test_fts_match_expression_neutralises_fts5_operators(self) -> None:
        """`AND`, `NEAR`, `*`, `-` and a stray quote in a real user message must be data, not syntax."""
        self.assertEqual(fts_match_expression('a AND b* -c "d'), '"a" OR "AND" OR "b" OR "c" OR "d"')
        self.assertEqual(fts_match_expression("!!! ???"), "")

    def test_rrf_prefers_a_document_both_arms_rank(self) -> None:
        fused = reciprocal_rank_fusion([["a", "b"], ["b", "c"]], [0.5, 0.5], RRF_K, 3)
        self.assertEqual(fused[0], "b")

    def test_rrf_weighting_shifts_the_result(self) -> None:
        cosine_only = reciprocal_rank_fusion([["a"], ["z"]], [1.0, 0.0], RRF_K, 1)
        lexical_only = reciprocal_rank_fusion([["a"], ["z"]], [0.0, 1.0], RRF_K, 1)
        self.assertEqual(cosine_only, ["a"])
        self.assertEqual(lexical_only, ["z"])

    def test_reembed_refuses_a_short_batch_rather_than_misaligning_vectors(self) -> None:
        """An endpoint that returns fewer vectors than inputs must abort, not zip and truncate.

        Silently pairing vector[i] with document[i] over a short list attaches real embeddings to the wrong documents, and every downstream number stays plausible.
        """
        import argparse as _argparse

        docs = [Document(id=f"m{i}", content=f"text {i}", scope="episodic", source="c", embedding=None) for i in range(4)]
        args = _argparse.Namespace(
            embed_cache=None, embed_model="fake", embed_batch=4, passage_prefix="", embed_url="http://x"
        )
        global embed_texts
        original = embed_texts
        try:
            embed_texts = lambda _a, texts: [[1.0, 0.0] for _ in range(len(texts) - 1)]  # noqa: E731
            with self.assertRaises(SystemExit):
                reembed_corpus(docs, args)
        finally:
            embed_texts = original

    def test_reembed_replaces_every_vector_and_never_mixes_spaces(self) -> None:
        import argparse as _argparse

        docs = [
            Document(id="m0", content="a", scope="episodic", source="c", embedding=(9.0, 9.0)),
            Document(id="m1", content="b", scope="episodic", source="c", embedding=None),
        ]
        args = _argparse.Namespace(
            embed_cache=None, embed_model="fake", embed_batch=8, passage_prefix="passage: ", embed_url="http://x"
        )
        global embed_texts
        original = embed_texts
        try:
            seen: list[str] = []

            def fake(_a: object, texts: Sequence[str]) -> list[Sequence[float]]:
                seen.extend(texts)
                return [[1.0, 2.0] for _ in texts]

            embed_texts = fake
            rebuilt = reembed_corpus(docs, args)
        finally:
            embed_texts = original
        self.assertEqual(seen, ["passage: a", "passage: b"])
        self.assertTrue(all(doc.embedding == (1.0, 2.0) for doc in rebuilt))

    def test_noise_floor_is_the_largest_movement_of_an_unchanged_arm(self) -> None:
        run_a = {"mean_ndcg": {"fts5": 0.6642, "cosine": 0.6025}}
        run_b = {"mean_ndcg": {"fts5": 0.6421, "cosine": 0.6984}}
        floor, movements = noise_floor_from_runs(run_a, run_b, ["fts5"])
        self.assertAlmostEqual(floor, 0.0221, places=4)
        self.assertAlmostEqual(movements["fts5"], -0.0221, places=4)

    def test_noise_floor_refuses_when_no_stable_arm_is_shared(self) -> None:
        with self.assertRaises(SystemExit):
            noise_floor_from_runs({"mean_ndcg": {"a": 1.0}}, {"mean_ndcg": {"b": 1.0}}, ["fts5"])

    def test_report_marks_a_sub_floor_difference_as_not_a_result(self) -> None:
        result = RunResult(
            arms=["fast", "slow"],
            per_query={
                "fast": {f"q{i}": 0.60 for i in range(30)},
                "slow": {f"q{i}": 0.58 for i in range(30)},
            },
            queries_counted=30,
            pairs_judged=300,
        )
        report = render_markdown(result, seed=1)
        self.assertIn("below the noise floor", report)

    def test_report_flags_a_query_set_too_small_for_the_bootstrap(self) -> None:
        result = RunResult(
            arms=["a", "b"],
            per_query={"a": {"q1": 0.9, "q2": 0.8}, "b": {"q1": 0.1, "q2": 0.2}},
            queries_counted=2,
            pairs_judged=8,
        )
        self.assertIn("Only 2 queries counted", render_markdown(result, seed=1))

    def test_report_does_not_flag_a_query_set_large_enough(self) -> None:
        result = RunResult(
            arms=["a", "b"],
            per_query={
                "a": {f"q{i}": 0.9 for i in range(MIN_USEFUL_QUERIES)},
                "b": {f"q{i}": 0.1 for i in range(MIN_USEFUL_QUERIES)},
            },
            queries_counted=MIN_USEFUL_QUERIES,
            pairs_judged=300,
        )
        self.assertNotIn("queries counted.**", render_markdown(result, seed=1))

    def test_report_names_the_leader_and_compares_everything_to_it(self) -> None:
        result = RunResult(
            arms=["a", "b"],
            per_query={"a": {"q1": 0.9, "q2": 0.8}, "b": {"q1": 0.1, "q2": 0.2}},
            queries_counted=2,
            pairs_judged=8,
        )
        report = render_markdown(result, seed=1)
        self.assertIn("Leader: a", report)
        self.assertIn("| a | 0.8500 | — |", report)


def self_test() -> int:
    suite = unittest.TestLoader().loadTestsFromTestCase(SelfTest)
    runner = unittest.TextTestRunner(verbosity=2)
    return 0 if runner.run(suite).wasSuccessful() else 1


# ── CLI ──────────────────────────────────────────────────────────────────────────────────────────────────


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Compare memory retrieval strategies on a real corpus with an LLM judge.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Never runs in CI beyond --self-test. Needs a live judge and a live embedding endpoint.",
    )
    parser.add_argument("command", nargs="?", default="run", choices=["run"], help="what to do (default: run)")
    parser.add_argument("--self-test", action="store_true", help="run the hermetic unit tests and exit")
    parser.add_argument(
        "--noise-floor",
        nargs=2,
        metavar=("RUN_A.json", "RUN_B.json"),
        help="measure this method's noise floor from two runs and exit",
    )
    parser.add_argument(
        "--stable-arms",
        default="",
        help="comma-separated arms that could not have changed between the two --noise-floor runs",
    )
    parser.add_argument("--corpus", help="path to a COPY of the memory SQLite database (opened read-only)")
    parser.add_argument("--queries", help="file with one query per line; use real user messages, not generated ones")
    parser.add_argument("--agent", help="restrict the corpus to one agent_id")
    parser.add_argument("--arms", default="cosine,fts5,rrf", help="comma-separated arms: cosine, fts5, rrf")
    parser.add_argument("--top-k", type=int, default=NDCG_K, help=f"documents per arm per query (default {NDCG_K})")
    parser.add_argument("--rrf-k", type=int, default=RRF_K, help=f"RRF damping constant (default {RRF_K})")
    parser.add_argument("--rrf-weight", type=float, default=0.5, help="weight on the cosine arm in rrf (default 0.5)")
    parser.add_argument("--seed", type=int, default=0, help="seeds the pool shuffle and the bootstrap")
    parser.add_argument(
        "--noise-floor-value",
        type=float,
        default=DEFAULT_NOISE_FLOOR,
        help=f"differences below this are reported as not-a-result (default {DEFAULT_NOISE_FLOOR})",
    )
    parser.add_argument("--judge-url", help="OpenAI-compatible chat completions endpoint")
    parser.add_argument("--judge-model", help="judge model id")
    parser.add_argument("--judge-cache", default=".memory-eval-judgments.json", help="on-disk judgment cache")
    parser.add_argument("--embed-url", help="OpenAI-compatible embeddings endpoint")
    parser.add_argument("--embed-model", help="embedding model id; must match the model that built the stored vectors")
    parser.add_argument("--query-prefix", default="", help="prefix for query embeddings, e.g. 'query: ' for e5")
    parser.add_argument("--passage-prefix", default="", help="prefix for document embeddings, e.g. 'passage: ' for e5")
    parser.add_argument(
        "--reembed",
        action="store_true",
        help="rebuild every corpus vector with --embed-model instead of using the stored ones; "
        "required to compare a different embedder, and never mixed with stored vectors",
    )
    parser.add_argument("--embed-cache", default=".memory-eval-embeddings.json", help="on-disk corpus vector cache")
    parser.add_argument("--embed-batch", type=int, default=32, help="documents per embedding request (default 32)")
    parser.add_argument("--out", help="write per-query scores as JSON here")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.self_test:
        return self_test()
    if args.noise_floor:
        return noise_floor_command(args)
    if not args.corpus or not args.queries:
        build_parser().print_help()
        print("\nerror: --corpus and --queries are required for a run", file=sys.stderr)
        return 2
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
