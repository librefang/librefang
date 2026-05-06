# Cron Session Sizing

Persistent cron jobs (`session_mode = "persistent"`, the default) all share one
dedicated session per agent — `SessionId::for_channel(agent, "cron")`. Without
intervention, every fire appends to that session forever. Eventually the
provider rejects the request with a context-window 400 and the job stops
working until an operator manually clears the session.

LibreFang ships three knobs that let you bound that growth, plus an
observability surface so you can see it coming.

## Knobs

All three live on `KernelConfig` (i.e. `~/.librefang/config.toml`, top level)
and apply only to **Persistent** cron sessions. Jobs configured with
`session_mode = "new"` skip the prune path entirely — every fire gets a fresh
session, so growth cannot accumulate across fires.

### `cron_session_max_messages` *(introduced in #2989)*

Drop the oldest messages from the front of the persistent cron session before
each fire if the session has more than `N` messages.

- `None` (default) — disabled.
- `Some(0)` — treated as `None` (disable; **not** "trim to zero").
- `Some(n)` where `n < 4` — clamped up to `4` with a `WARN` log; smaller values
  silently destroy enough history to break prompt-cache reuse and tool-result
  referencing.
- `Some(n)` otherwise — keep the most recent `n` messages.

### `cron_session_max_tokens` *(introduced in #2989)*

Same behaviour but estimated-token budget instead of message count. Drops the
oldest message in a loop until the estimated token count of the session falls
below `N`. Tokens are estimated via
`librefang_runtime::compactor::estimate_token_count` (CJK-aware char-weighted
heuristic — same accounting the message-history-trim path uses; no external
tokenizer dependency).

- `None` (default) — disabled.
- `Some(0)` — treated as `None`.
- `Some(n)` — apply as a rolling token window.

Both caps run together: `cron_session_max_messages` first, then
`cron_session_max_tokens`. Pruning is serialized through the per-session mutex
so two cron fires for the same agent cannot clobber each other's keep-set
(#3443).

### `cron_session_warn_fraction` *(introduced in #3693)*

Fraction of the effective token budget at which the kernel emits a
`tracing::warn!` after pruning. Catches drift before the provider returns 400.

- Default: `Some(0.8)` — warn at 80% of the budget.
- `None`, `<= 0.0`, `> 1.0`, NaN, or Inf — disable (silent).

The "effective budget" is resolved as:

1. `cron_session_max_tokens` if set, else
2. `cron_session_warn_total_tokens` (default `Some(200_000)`) as a fallback
   ceiling so jobs that have not opted into pruning still get warnings, else
3. no budget → no warn.

The warn line is structured:

```
WARN cron session approaching context budget — consider lowering
     cron_session_max_tokens, enabling cron_session_max_messages, or
     setting session_mode = "new" on this job
agent_id=<uuid> session_id=<sid> job=<name> tokens=<n>
threshold=<n> budget=<n> messages=<n>
```

Hook your log pipeline to alert on it.

### `cron_session_warn_total_tokens` *(introduced in #3693)*

Fallback ceiling used by `cron_session_warn_fraction` when
`cron_session_max_tokens` is unset. Default `Some(200_000)` — matches the
typical Claude / GPT-4 long-context window. Set to `None` to disable the
fallback (warn fires only when an explicit `cron_session_max_tokens` is
configured).

## API observability

`GET /api/cron/jobs/{id}` and `GET /api/cron/jobs/{id}/status` return the
existing cron `JobMeta` augmented with two #3693 fields:

| Field                    | Type    | Meaning                                                                |
| ------------------------ | ------- | ---------------------------------------------------------------------- |
| `session_message_count`  | `usize` | Messages in the persistent `(agent, "cron")` session right now.        |
| `session_token_count`    | `u64`   | Estimated tokens for those messages (system prompt and tools excluded — same accounting as the prune path). |

Both fields are `0` when the job has never fired in `Persistent` mode (no
session exists yet). They are additive — older clients that ignore unknown
keys keep working.

The dashboard can graph these straight off the existing detail / status
endpoints; no separate metrics route is needed.

## Picking a strategy

| Scenario                                                                    | Recommendation                                                       |
| --------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Long chain of unrelated prompts (the cron job is "stateless per fire")      | `session_mode = "new"` on the job. Each fire gets a fresh session.   |
| Continuous state machine (the agent must remember prior fires)              | `Persistent` + set `cron_session_max_tokens` (e.g. `100_000`).       |
| You want a soft early-warning before any cap is reached                     | Leave `cron_session_warn_fraction` at default; watch for the WARN.   |
| Hard isolation between fires for safety / audit                             | `session_mode = "new"`; pruning knobs do not apply.                  |

## Compaction mode: `cron_session_compaction_mode` *(introduced in #3693 Gap 4)*

Controls **how** messages are removed when `cron_session_max_tokens` or
`cron_session_max_messages` is triggered.

### `"prune"` (default)

Drop the oldest messages from the front of the session in a loop until the
budget is satisfied. Fast and deterministic; identical to the pre-#3693
behaviour. No LLM call is involved.

### `"summarize_trim"`

Instead of silently discarding the messages that would be dropped, call the
configured compression LLM (the same auxiliary chain used by
`/compact` — `[llm.auxiliary] compression` if set, otherwise the default
driver) to produce a short summary. The summary is prepended as a synthetic
`assistant` message, followed by the most recent
`cron_session_compaction_keep_recent` messages verbatim.

```
[Cron session summary — <N> messages compacted]

<LLM-generated summary text>
```

If the LLM call fails (network error, empty response, provider 5xx), the
kernel **falls back to `prune`** and emits a `tracing::warn!`:

```
WARN cron SummarizeTrim: LLM summarization failed or returned empty; falling back to Prune
agent_id=<uuid> session_id=<sid> job=<name>
```

The fire is never blocked by a summarization failure.

### `cron_session_compaction_keep_recent`

Number of messages preserved verbatim **after** the summary (the "tail").
Only meaningful when `cron_session_compaction_mode = "summarize_trim"`.

- Default: `8`
- Minimum: `1` (values below are clamped to `1`)

**Example config.toml:**

```toml
cron_session_max_tokens        = 80000
cron_session_compaction_mode   = "summarize_trim"
cron_session_compaction_keep_recent = 10
```

## Picking a strategy (updated)

| Scenario                                                                       | Recommendation                                                                |
| ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| Long chain of unrelated prompts (the cron job is "stateless per fire")         | `session_mode = "new"` on the job. Each fire gets a fresh session.            |
| Continuous state machine (the agent must remember prior fires)                 | `Persistent` + set `cron_session_max_tokens` (e.g. `100_000`).               |
| Continuous state machine, context continuity matters                           | `Persistent` + `cron_session_compaction_mode = "summarize_trim"`.             |
| You want a soft early-warning before any cap is reached                        | Leave `cron_session_warn_fraction` at default; watch for the WARN.            |
| Hard isolation between fires for safety / audit                                | `session_mode = "new"`; pruning knobs do not apply.                           |

## Concurrency caveat

When an agent has **multiple cron jobs** and all of them use
`session_mode = "persistent"` (the default), every fire maps to the same shared
`(agent, "cron")` session.

Compaction is serialized through a **per-session mutex** (see the `prune_lock`
path in `kernel/mod.rs`), so two fires that both need to prune cannot race each
other's read-modify-write. However, the global `cron_lane` semaphore that caps
total in-flight cron fires is **not scoped per session** — it only limits how
many fires execute concurrently across the whole daemon.

`SummarizeTrim` amplifies the race window significantly: a plain prune takes
microseconds, while an LLM summary call may take several seconds. During that
window a second fire for the same agent can:

1. Acquire the per-session mutex (the first fire released it before calling the
   LLM).
2. Load the same session — which has not yet been compacted.
3. Run to completion and write back its response.
4. The first fire then writes its compacted session, silently overwriting the
   second fire's new messages (last-write-wins).

**Mitigation**: if an agent has multiple cron jobs that need independent context
or if missing a fire's output is unacceptable, configure those jobs (or the
agent) with `session_mode = "new"`. Each fire then gets its own deterministic,
isolated session and the shared `(agent, "cron")` session is never touched.
