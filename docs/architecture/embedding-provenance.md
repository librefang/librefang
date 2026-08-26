# Embedding provenance

`memories.embedding` is a vector in one particular model's space.
Nothing about the BLOB says which one, and a cosine between two spaces is a number rather than a similarity — so the store has to record the model that produced each vector, and the daemon has to notice when the configured model no longer matches what is on disk.

## The failure this exists to stop

`cosine_similarity` (`crates/librefang-types/src/memory.rs`) returns `None` when the two vectors have different lengths, and that length check was the only guard the embedding path had.
It catches a change in dimensionality and nothing else.

Two 1024-dimensional models — the measurement on #7912 compared `bge-m3` against `multilingual-e5-large` — slip straight past it.
An operator editing one line of `[memory] embedding_model` got no error, no warning, and no failed health check: the embedding subsystem is genuinely healthy in the only sense it could report, because it is returning vectors of the right shape.
What changed is that every pre-existing row's similarity became meaningless, and retrieval quietly went random for the whole historical corpus.

## What identifies a vector space

The stamp is `provider/model` as **resolved at boot**, not `config.memory.embedding_model` as written.
Those differ routinely:

- when the configured string is one of the two built-in placeholders (`all-MiniLM-L6-v2`, `text-embedding-3-small`), `kernel/boot.rs` substitutes `default_embedding_model_for_provider(provider)`;
- with no `embedding_provider` set, `detect_embedding_provider()` picks one from the environment, so the same config file resolves differently on two machines.

Only the resolved pair identifies the space a vector belongs to, so only the resolved pair is worth storing.

## Lifecycle

**Boot.** The kernel resolves the driver, calls `MemorySubstrate::set_embedding_model`, then runs `embedding_model_census()` — a `GROUP BY embedding_model` over live rows that carry a vector — and emits a single `WARN` naming every model in the store that is not the active one, with its row count.
A census failure downgrades to a warn and boot continues; provenance is an observability feature and must not be able to stop a daemon starting.

**Write.** `SemanticStore::remember_with_embedding_and_peer` stamps the active model onto any row it writes *with* a vector.
A text-only row is left unstamped: it has no embedding space to belong to, and stamping it would inflate the census with rows a model change cannot affect.

**Recall.** `recall_impl` reads the stamp alongside the vector and refuses to score a candidate whose stamp is a *different* model — that fragment's `similarity` stays `None`, which is exactly the treatment a dimension-mismatched vector already received: it sorts to the bottom on the `NEG_INFINITY` sentinel, and a `min_similarity` floor drops it, because an unmeasured row cannot clear a measured minimum.
One log line per recall, not per row.

**Dedup.** `get_embeddings_batch` withholds vectors stamped with a different model.
Its only caller, `ProactiveMemoryStore::find_duplicates`, compares two *stored* vectors and merges the rows that score above the threshold; a merge is destructive, so those pairs drop to the Jaccard word-overlap fallback rather than being merged on a meaningless score.

## `NULL` means comparable

Rows written before the v51 stamp carry `NULL`, and every path above treats them as comparable.

This is the deliberate choice and not an oversight.
The overwhelmingly common case is an operator who has never changed their embedding model, for whom every historical row is perfectly valid; silently dropping their entire corpus out of retrieval on upgrade would be a far worse default than the risk it removes.
The census reports them under `(unstamped, pre-v51)` so the size of that population is visible rather than assumed.

## What is not covered

**The external vector store.** With `[memory] vector_backend = "http"`, similarity is computed by the backend and the SQLite rows are only hydration. Provenance for that path belongs to the backend's own index and is not something this stamp can enforce.

**Re-embedding.** There is no sweep that re-embeds stale rows against the new model. Detection is what exists; the repair is still "restore the previous `embedding_model`", or delete and let the corpus rebuild.

**Asymmetric query/passage prefixes.** A large part of the current embedding landscape is trained on a `query:` / `passage:` pair, and core sends raw text. That is a change to the embedding drivers rather than to the store, and #7912 measures it as worth about +0.049 nDCG@10 on conversational memory.
