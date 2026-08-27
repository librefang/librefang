# Episodic memory lifecycle

The `memories` table has one scope, `episodic`, that behaves unlike the others: it is the table default (`scope TEXT NOT NULL DEFAULT 'episodic'`) and it is written by the agent loop itself, one row per turn, rather than by a deliberate `memory_store` call.
This document states what that layer is for and what removes rows from it, because before #7911 the answer to the second question was "nothing".

## What writes it

`crates/librefang-runtime/src/agent_loop/end_turn.rs` composes `[Past exchange]\nThem: …\nYou: …` from the finished turn and hands it to `remember_interaction_best_effort` (`agent_loop/prompt.rs`), which writes straight to `MemorySubstrate`.
The only gate is `!is_fork && !incognito`.
There is no relevance test, no extraction threshold and no category — the row is the raw exchange.

This is a *parallel* writer to the proactive-memory extractor, not an input to it.
`ProactiveMemoryStore::auto_memorize` reads the live turn out of `ctx.session.messages`, never the stored episodic rows, so the arrow from episodic to semantic that this architecture is usually described as having does not exist in code.
Anything that changes that is a distillation feature, and none is implemented today.

## What removes it

Three mechanisms, in the order they touch a row.

**Time-based decay** — `crates/librefang-memory/src/decay.rs::run_decay`, scheduled by the kernel's `memory_decay` task.
Soft-deletes (`deleted = 1`) any episodic row whose `accessed_at` is older than `[memory.decay] episodic_ttl_days`, default 90.
The whole sweep is off unless `[memory.decay] enabled = true`.

The TTL is measured from **last access, not creation**, and `SemanticStore::recall_with_embedding` refreshes `accessed_at` on every genuine recall.
A row that keeps being retrieved keeps living however old it is; a row nobody has needed for a quarter is the one that goes.
That is what makes a purely time-based policy safe for a layer whose job is "what did we talk about" — it retires the tail, not the working set.

`episodic_ttl_days` is deliberately longer than the AGENT default of 30, because an episodic row is often the only record that a conversation happened at all.
Setting it to `0` disables expiry for this scope only; the other scopes keep sweeping.

**Hard delete** — `prune_soft_deleted_memories`, on the daily retention sweep, removes rows soft-deleted more than `[memory] soft_delete_retention_days` ago (default 30) and reclaims the embedding BLOB.
Decay alone stops a row being recalled; this is what stops it costing bytes.

**Per-agent cap** — `ProactiveMemoryStore::evict_if_over_cap` evicts the lowest-confidence rows once an agent exceeds `max_memories_per_agent`, counting every scope.
Note that the per-turn episodic writer does not go through the proactive store, so this cap is only enforced on the turns where the extractor also stored something.
It is a backstop against a runaway store, not the retention policy.

## What bounds a single row

`[memory] max_episodic_chars`, default 8000, caps the text of one per-turn episodic row.
`0` disables the cap.

The cap exists because the per-turn writer inlines whatever the turn produced.
When a channel adapter renders an attachment into the user message — a transcribed PDF, a pasted log — the whole document used to become one memory row and one embedding request; the largest row reported on #7911 was 201765 characters, and it had been retrieved 63 times.

The budget is applied before the embedding call, so it bounds embedding cost as well as stored bytes, and it is split across the two halves of the exchange by `budget_interaction_halves` (`agent_loop/message.rs`): a side that fits inside its half keeps everything and releases the remainder to the other side.
That is what stops a 200 KB user message from truncating the agent's reply away — the reply is usually short, takes its full length out of the budget, and the attachment absorbs the rest.
Cuts land on `char` boundaries and append `… [truncated]`, so a recalled row reads as an excerpt rather than as a sentence the user stopped mid-way through.

## What is deliberately not here

**Distillation.** Nothing summarises a set of episodic rows into a semantic fact and retires the originals.
Auto-Dream is an agent turn prompted to use `memory_recall` / `memory_store`, so its throughput is whatever the model decides to do that night — measured at two orders of magnitude below the write rate on the instance in #7911 — and it is not a transform over the episodic layer.
Until a deterministic pass exists, the honest description of this layer is: **a bounded, self-expiring buffer of recent exchanges**, and facts come from the live turn via the extractor, not from re-reading it.

**Write-time gating.** Every non-fork, non-incognito turn is still stored.
Gating on the extractor's signal would cut volume at the source but would also mean a turn that produced no fact leaves no trace, removing the only capability this layer provides.
The size cap is the gate that was uncontroversial; the relevance gate is still an open policy question.
