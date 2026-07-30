# RFC: multi-replica deployment architecture

**Status:** proposal, not accepted. No implementation work should start from this document until a maintainer signs off on the storage and coordination decisions in [Decisions](#decisions) — they change a core dependency.

**Scope:** what it would take for LibreFang to run as more than one daemon replica, and what the repository should promise until then.

**Companion:** [`deploy/kubernetes/`](../../deploy/kubernetes/README.md) is the supported single-replica baseline. This document is the answer to "why is `replicas` pinned to 1 there?".

Refs #6634. Depends on nothing; blocks any HA claim.

## The short version

Replacing SQLite is necessary and nowhere near sufficient.

Twenty-four named background workers, the per-session execution locks, the audit hash chain, the cost-reservation ledger, and every in-process cache assume exactly one authoritative daemon.
A second replica does not degrade gracefully against any of them — it duplicates scheduled work, interleaves writes into one session's history, enforces roughly N× the configured budget, and forks the audit chain into branches that no longer verify.

So the proposal is **not** active-active.
It is a four-phase path where each phase is independently shippable, single-node SQLite keeps working as the default at every step, and `replicas > 1` is documented as supported only after the phase that makes external side effects idempotent.

## Current constraints, verified against the tree

### The hard stop

`run_daemon` takes an exclusive `flock` on `$LIBREFANG_HOME/daemon.lock` (`crates/librefang-api/src/server.rs`, `acquire_daemon_lock`).
Two pods sharing one volume cannot both boot; the second exits with a lock error.
This is a deliberate guard, not an accident, and it is the only thing standing between an operator who types `replicas: 2` and the failure modes below.

It is also not a *sufficient* guard, because it only binds replicas that share a filesystem.
Give each replica its own PVC and the lock succeeds on both while every problem below is still present — which is exactly why "just use ReadWriteMany" and "just give each pod its own volume" are both wrong answers.

### Singleton background workers

Every one of these is spawned once per daemon process and assumes it is the only instance.
Names are the `spawn_logged` labels, so they are greppable:

| Worker | What duplicates under N replicas |
| --- | --- |
| `cron_scheduler`, `cron_agent_turn` | Every job fires N times per schedule. |
| `trigger_dispatch` | Every matching event dispatches N agent turns. |
| `heartbeat_monitor` | N independent liveness verdicts on the same agents. |
| `approval_resolution` | A single approval can be resolved N times. |
| `post_loop_compaction` | Concurrent compaction of one session's history. |
| `auto_memorize` | N copies of each captured memory. |
| `memory_consolidation`, `memory_decay` | Concurrent rewrites of the same rows. |
| `gc_sweep`, `upload_cleanup`, `session_retention_cleanup`, `audit_retention_trim`, `audit_log_pruner`, `metering_cleanup` | Retention sweeps racing each other's deletes. |
| `mcp_health_loop`, `mcp_reconnect`, `connect_mcp_servers` | N stdio MCP child processes per configured server. |
| `local_provider_probe`, `openrouter_catalog_error_refresh` | N× probe traffic to providers. |
| `background_agents_staggered_start` | The stagger is per-process, so N replicas un-stagger each other. |
| `owner_notify` | The operator gets N copies of each notification. |
| `a2a_discover_external` | N registrations of the same remote agent. |
| `ofp_node` | N nodes claiming one identity. |

The stdio-MCP row is the sharpest of these: those are child processes with their own credentials and their own side effects, so "N copies" means N shells, not N reads.

### Execution ownership

Agent turns serialize on `agent_msg_locks` / `session_msg_locks` inside `send_message_full` — in-process `Mutex` maps.
They are correct within one daemon and invisible across daemons.
Two replicas handed the same `(agent_id, session_id)` will both read the history, both append, and the loser's turn silently vanishes from the transcript while its tool calls have already executed.

### Audit chain

`librefang-runtime-audit` builds a hash-linked chain: each entry stores `prev_hash` and a `hash` over its content plus that `prev_hash`, and the tip advances per write (`record_with_context`).
The tip is per-process.
Two replicas appending concurrently produce two branches from one ancestor; `verify_integrity` then reports the chain as broken, and there is no merge — a hash chain has no join operation.

### Budgets

`MeteringEngine` holds a `CostReservationLedger` of pre-charged-but-unsettled spend **in memory** (`crates/librefang-kernel-metering/src/lib.rs`).
Persisted usage is shared through SQLite, but the reservation window is not.
N replicas therefore admit up to N concurrent calls that each believe they fit under the cap.
The overshoot is bounded by the reservation window, not by the cap.

### Storage

`MemorySubstrate` is a concrete struct holding `Pool<SqliteConnectionManager>` directly (`crates/librefang-memory/src/substrate.rs`) — not a trait.
There is no backend seam to implement a second store behind, which is the bulk of Phase 1's cost.

Two useful exceptions already exist and should be the model: `IdempotencyStore` (`crates/librefang-memory/src/idempotency.rs`) is a trait with a SQLite impl, and the vector store already has a second backend (`http_vector_store.rs`, selected by `memory.vector_backend`).

### Process-local state that is not a database

- OAuth token store: a `LazyLock<HashMap>` keyed by upstream subject (`crates/librefang-api/src/oauth.rs`). A refresh issued to replica B cannot see a token stored by replica A.
- OAuth state-token HMAC key: `LIBREFANG_STATE_SECRET`, or a random per-process UUID when unset. Without the env var, a login started on A and called back on B fails.
- Dashboard sessions: an in-memory `active_sessions` map. A logged-in user hitting a different replica is anonymous.
- WebSocket / SSE session streams: `session_stream_hub` broadcast channels are per-process, so a client connected to A never sees events produced on B.
- Provider probe cache, model catalog, web fetch cache: per-process, so N× upstream traffic and N different views of provider health.

## Decisions

These are the decisions the RFC asks a maintainer to accept or reject. Each is a recommendation, not an option list.

### D1 — Target shape: stateless API replicas around a leader-elected execution plane

**Not** active-active agent execution.

An agent turn is a long-running operation with irreversible external side effects (shell commands, HTTP writes, channel messages).
Distributing those across peers requires per-session fencing *and* idempotent side effects before it is safe at all, so making it the phase-1 target buries the project in coordination work before anything ships.

The shape instead:

- **Every replica** serves the HTTP API, including all reads, and accepts writes that are pure database operations.
- **One elected leader** runs the execution plane: the 24 background workers, cron, trigger dispatch, and agent turns.
- **Followers forward** work that must execute (an agent turn, a tool call) to the leader through a durable queue, rather than executing it locally.

That yields the properties operators actually ask for — API availability across a pod restart, rolling updates without a request gap, horizontal read scaling — without claiming distributed execution semantics the system does not have.

Phase 3 then relaxes "one leader for everything" to "one owner per session", which is where real execution scale-out lives.

### D2 — Storage: PostgreSQL, behind a new backend trait, with SQLite remaining the default

⚠️ **This is the decision that needs explicit maintainer approval**, because it adds a core dependency. Nothing else in this RFC is as consequential.

Why Postgres rather than "SQLite plus something":

- Transactions across the tables a turn touches (history, usage, audit, approvals) are already assumed by the code; any backend has to keep them.
- Advisory locks (`pg_try_advisory_lock`) give leader election and per-session leases as a native primitive, with no second system to operate.
- `SELECT … FOR UPDATE SKIP LOCKED` gives the follower→leader work queue with no broker.
- `LISTEN`/`NOTIFY` gives cross-replica event fan-out, which is what `session_stream_hub` needs to stop being process-local.
- pgvector covers `SemanticStore` without keeping a separate vector service alive.
- Managed HA is available from every cloud and from CloudNativePG in-cluster, so DR is not a bespoke problem.

Rejected alternatives, with the reason:

- **etcd or Redis for coordination + SQLite for data** — two failure domains, two backup stories, and the data plane still cannot be shared.
- **SQLite on ReadWriteMany** — the WAL depends on POSIX locking and `mmap`-visible shared memory that NFS and CIFS commonly implement incorrectly or not at all. `flock` being a silent no-op is the worst case: `daemon.lock` passes and both daemons corrupt each other.
- **rqlite / dqlite / LiteFS** — keeps the SQLite API but adds Raft semantics (single-writer, leader redirection, no cross-node transactions) that the code does not currently model, so the migration cost lands anyway without the query-level upside.
- **A bespoke sync layer over SQLite** — this is inventing a distributed database. No.

Compatibility, and it is load-bearing: **single-node SQLite stays the default and stays supported indefinitely.**
`librefang start` on a laptop must never require a Postgres. The backend is selected by config (`[memory] backend = "sqlite" | "postgres"`), Postgres is required only for `replicas > 1`, and the single-node path keeps its own CI lane so it cannot rot.

### D3 — Coordination: Postgres advisory locks, not the Kubernetes Lease API

The Kubernetes `coordination.k8s.io/Lease` API is the obvious choice on Kubernetes and the wrong choice here, because LibreFang also runs under systemd, Docker Compose, Fly, and Railway.
Coordination anchored in the database works identically everywhere, needs no ServiceAccount or RBAC, and cannot disagree with the data plane about who holds what — the lock and the rows it protects commit against the same server.

- **Leader election:** a `daemon_leases` row holding `(lease_name, holder_id, fencing_token, expires_at)`, renewed on an interval well inside the TTL, acquired by `UPDATE … WHERE expires_at < now()` which atomically bumps `fencing_token`.
- **Per-session ownership:** the same table keyed by `(agent_id, session_id)`.
- **Work queue:** `FOR UPDATE SKIP LOCKED` over a `work_items` table.

`daemon.lock` stays exactly as it is for the SQLite backend. It is the correct guard for the single-node case and cheap to keep.

### D4 — Fencing over heartbeats

A lease that has expired does not mean the holder has stopped.
A GC pause, a paused container, or a partitioned node produces a *zombie owner*: still executing, still convinced it holds the session, about to write.

Every state-mutating write from the execution plane therefore carries the fencing token it was granted, and the write is conditional on that token still being current:

```sql
UPDATE session_history SET … 
 WHERE session_id = $1 AND $2 >= (SELECT fencing_token FROM daemon_leases WHERE …)
```

A zombie's token is stale by construction, so its write affects zero rows and it learns it has been fenced from the row count.
Heartbeat-only designs cannot detect this — they ask "am I alive?" when the question is "am I still the owner?".

### D5 — Idempotency keys on every external side effect

At-least-once delivery plus deduplication, not exactly-once — exactly-once does not exist across a process boundary.

Before a tool call or channel send, the executor writes an intent row keyed by a deterministic idempotency key derived from `(session_id, turn_seq, tool_call_index, args_hash)`, and marks it settled on completion.
Recovery after a pod loss re-drives the turn, finds the settled intent, and skips re-execution.
An unsettled intent whose outcome is unknown is surfaced to the operator rather than blindly retried, because "did the shell command run?" is not a question the daemon can answer for itself.

`IdempotencyStore` already exists for HTTP request replay. This is the same idea one layer down, and should share the table shape so there is one concept to reason about.

### D6 — Budget: convert the in-memory reservation into a database reservation

`CostReservationLedger` becomes a `cost_reservations` table. A pre-charge is an insert inside the same transaction that admits the call; settlement updates it; a crashed replica's reservations expire by TTL and are swept by the leader.
Cross-replica enforcement is then exact at the reservation boundary rather than N× the cap.

### D7 — Everything process-local becomes shared or becomes explicitly per-replica

Each item from [Process-local state](#process-local-state-that-is-not-a-database) gets a decision, not a default:

| State | Decision |
| --- | --- |
| OAuth token store | Move to the database, keyed by local user (this also fixes the cross-user leak in #6629 — that fix should not wait for this RFC). |
| `LIBREFANG_STATE_SECRET` | Becomes **mandatory** when `replicas > 1`, extending a guard that already exists: boot already refuses when `[external_auth] enabled = true` and the value is missing or not base64-decoding to 32 bytes (`validate_state_secret_env` in `crates/librefang-kernel/src/kernel/boot.rs`), for exactly this reason. The multi-replica case drops the `external_auth` condition. |
| Dashboard sessions | Move to the database, or require sticky sessions at the ingress. Prefer the database — sticky sessions are an ingress feature the project cannot assume. |
| `session_stream_hub` | Fan out via `LISTEN`/`NOTIFY` so a client connected to any replica sees every event. |
| Provider probe cache, model catalog, web cache | Stay per-replica. They are caches: N× upstream probe traffic is a cost, not a correctness bug. Document the multiplier. |

## Consistency and failure model

What the system would guarantee, stated so it can be tested rather than assumed.

**Single-replica mode (default, SQLite or Postgres).** Linearizable. One process, one lock hierarchy. Unchanged from today.

**Multi-replica mode (Postgres only).**

- *Per-session linearizability.* All writes to one `(agent_id, session_id)` are totally ordered, enforced by the lease plus fencing. Sessions are independent and unordered relative to each other.
- *Read-your-writes per client*, provided the client's reads route to a replica that has caught up. With synchronous replication or reads-from-primary, that is unconditional; with async replicas it is not, so reads default to the primary until an operator opts into replica reads.
- *Audit chain: exactly one appender.* The chain is a strictly serial structure, so appends stay leader-only even in Phase 4. This is a permanent design constraint, not a phase.
- *Budgets: exact at the reservation boundary.* Overshoot is bounded by in-flight reservations, and reservations are database rows.
- *External side effects: at-least-once with deduplication.* Effectively-once for anything reached through the idempotency layer. Anything outside it (an MCP server with its own side effects we cannot key) keeps at-least-once semantics and must be documented as such.

**Failure scenarios.**

| Failure | Behaviour |
| --- | --- |
| Follower pod lost | No impact beyond in-flight requests to that pod. |
| Leader pod lost | Sessions it owned are unowned until their leases expire (bounded by TTL); a new leader adopts them and re-drives interrupted turns through the idempotency layer. Cron jobs due during the gap fire late, once. |
| Leader partitioned but alive (zombie) | Its writes are fenced and fail. It observes the failure and steps down. No split-brain writes. |
| Database failover | All replicas lose connections and fail readiness; `/api/ready` reports 503, the Service drops every endpoint, and the daemons reconnect. No pod restarts, because liveness stays on `/api/health`. |
| Clock skew between replicas | Leases use database time (`now()`), not replica wall-clocks, so skew does not affect ownership. |
| Turn interrupted mid-tool-call | The intent row is unsettled and the outcome is unknown. Surfaced to the operator; not auto-retried. |

## Architecture

Today:

```
                     ┌──────────────────────────────┐
   HTTP ────────────▶│  librefang daemon (1 replica)│
                     │                              │
                     │  axum API                    │
                     │  kernel: 24 bg workers       │
                     │  agent_msg_locks (in-proc)   │
                     │  MeteringEngine + ledger     │
                     │  audit chain tip (in-proc)   │
                     │  OAuth tokens (in-proc)      │
                     └──────────────┬───────────────┘
                                    │ exclusive flock
                     ┌──────────────▼───────────────┐
                     │  /data  (SQLite WAL, RWO PVC)│
                     └──────────────────────────────┘
```

Phase 2 target — API scales, execution does not:

```
   HTTP ──▶ Service ──┬──▶ replica A (leader)   ──┐   runs: bg workers, cron,
                      │      API + execution      │   triggers, agent turns
                      ├──▶ replica B (follower) ──┤   API only; enqueues work
                      └──▶ replica C (follower) ──┤   API only; enqueues work
                                                  │
                                    ┌─────────────▼──────────────┐
                                    │  PostgreSQL                │
                                    │   daemon_leases (fencing)  │
                                    │   work_items (SKIP LOCKED) │
                                    │   history / usage / audit  │
                                    │   cost_reservations        │
                                    │   LISTEN/NOTIFY fan-out    │
                                    └────────────────────────────┘
```

Phase 4 target — ownership per session rather than one leader for everything:

```
   replica A ── owns sessions {s1, s4}  ─┐
   replica B ── owns sessions {s2}      ─┼──▶ PostgreSQL
   replica C ── owns sessions {s3, s5}  ─┘      per-session leases + fencing
                                                idempotency keys on side effects
   still leader-only: audit append, retention sweeps, cron scheduling
```

Session lease lifecycle:

```
  unowned ──acquire(token=N+1)──▶ owned ──renew──▶ owned
     ▲                              │                │
     │                        release│          TTL expired
     └──────────────────────────────◀┘                │
                                                      ▼
                                            expired ──adopt(token=N+2)──▶ owned
                                                          │
                                            old holder's writes at token N+1
                                            now affect 0 rows → fenced, steps down
```

## Phased plan

Each phase ships on its own and leaves single-node SQLite working. No phase is a prerequisite for using LibreFang.

**Phase 0 — document the boundary. Done (#6635, #6632, #6633).**
Single-replica manifests, `replicas: 1` asserted in CI, distinct liveness/readiness contracts, restricted Pod Security. The repository now says what it supports.

**Phase 1 — storage seam.**
Extract a `MemoryBackend` trait from `MemorySubstrate`, following the `IdempotencyStore` and `http_vector_store` precedent. Implement Postgres behind it (pgvector for `SemanticStore`). Add versioned migrations for both backends. `[memory] backend` selects; default unchanged. CI runs the full suite against both.
Ships value on its own: an external database with real backups and managed HA, without any multi-replica claim.

**Phase 2 — leader election and the work queue.**
`daemon_leases` and `work_items`. All 24 workers become leader-only. Followers enqueue rather than execute. `LIBREFANG_STATE_SECRET` mandatory above one replica; OAuth tokens and dashboard sessions move to the database; `session_stream_hub` fans out via NOTIFY. Budget reservations become rows.
At the end of this phase `replicas > 1` *works* but is still undocumented, because side effects are not yet deduplicated.

**Phase 3 — per-session ownership.**
Session leases with fencing tokens, acquired on first turn and renewed for the turn's duration. Routing discovers the owner through the lease table; a request landing on a non-owner is forwarded rather than executed. Turn recovery adopts orphaned sessions after TTL.

**Phase 4 — idempotent side effects, then document HA.**
Intent rows for tool calls and channel sends, keyed as in D5. Recovery re-drives turns through the dedup layer. Chaos tests below must pass. Only then does documentation say `replicas > 1` is supported, and only then does the manifest comment in `deploy/kubernetes/base/statefulset.yaml` change.

## Migration and rollback

**Forward.** `librefang migrate --to postgres` performs an offline copy: stop the daemon, read the SQLite substrate, write Postgres, verify row counts and the audit chain, then boot against the new backend. Offline and not live, because a hash chain cannot be copied consistently while it is being appended to.

**Rollback.** Every phase is reversible by configuration, and rollback is only safe from a phase to the one before it:

- Phase 1: point `[memory] backend` back at SQLite. Requires a reverse export, so keep the pre-migration SQLite file until the operator is confident.
- Phase 2: scale to 1 replica. The single remaining daemon is trivially the leader; the queue drains; nothing needs undoing.
- Phase 3/4: scale to 1 replica. Session leases and intent rows become no-ops with one owner.

Rollback across the Phase 1 boundary after Phase 2 data has accumulated needs the export path, which is why the export must ship *with* Phase 1 rather than after it.

## Observability

New signals, because none of the above is debuggable without them:

- `librefang_leader{lease}` — 1 on the holder, 0 elsewhere. A sum ≠ 1 is a split-brain alert.
- `librefang_lease_acquisitions_total{lease}` — leadership churn; a rising rate means TTL is too tight for the environment.
- `librefang_fenced_writes_total` — should be 0. Non-zero means a zombie was caught, which is the system working, and is still worth paging on.
- `librefang_work_queue_depth`, `librefang_work_queue_latency_seconds` — follower→leader backpressure.
- `librefang_session_lease_age_seconds` — a lease older than the longest plausible turn is a leaked lease.
- `librefang_idempotent_skips_total` — deduplication actually firing during recovery.

`/api/ready` gains the backend connection as a required check; the leader status is deliberately *not* part of readiness, because a follower is ready to serve API traffic.

## Disaster recovery

- **Backups** are the database's, taken with its native tooling (`pg_dump` or continuous archiving). The single-node SQLite path keeps file-level backup of `/data`.
- **RPO/RTO** are the operator's storage decision, not the daemon's. Document that continuous archiving is what makes RPO small, and that a nightly dump means "up to a day of agent history".
- **Restore drill** must be part of the acceptance criteria, not a document: restore into a scratch database, boot one replica against it, verify the audit chain end to end, confirm no cron job double-fires from the restored state.
- **The audit chain is the integrity anchor.** A restore that breaks `verify_integrity` is a failed restore, and the tooling should say so rather than booting anyway.

## Test plan

Integration tests, per phase:

- Phase 1: the entire existing suite, green against both backends. Migration round-trip preserves row counts and chain integrity.
- Phase 2: N replicas, one cron job, assert exactly one fire per schedule. Kill the leader mid-tick, assert the job fires once, late. Assert follower-enqueued work executes exactly once.
- Phase 3: two replicas race one session, assert the non-owner forwards and the transcript has no interleaving. Kill the owner mid-turn, assert adoption after TTL and no lost history.
- Phase 4: interrupt a turn between tool intent and settlement, assert recovery does not re-execute. Assert an unsettled unknown-outcome intent surfaces to the operator instead of silently retrying.

Chaos scenarios, all of which must pass before HA is documented:

| Scenario | Assertion |
| --- | --- |
| SIGKILL the leader mid-cron-tick | Job fires once total, late. No duplicate side effects. |
| Partition the leader from Postgres, keep it running | Its writes are fenced (`fenced_writes_total` > 0), it steps down, a new leader adopts. No split-brain rows. |
| Pause the leader container past lease TTL, then resume | Same as above. This is the zombie case that heartbeats miss, so it is the test that matters most. |
| Postgres failover | All replicas fail readiness, none restart, all reconnect. `/api/health` stays 200 throughout. |
| Kill a replica mid-agent-turn | Turn recovered by a new owner; tool calls not re-executed; budget reservation released. |
| Rolling update under sustained load | No 5xx to clients; exactly-once cron; no chain divergence. |
| Introduce 5s clock skew between replicas | No ownership change. Leases use database time. |
| Run 3 replicas against a hard budget cap with concurrent turns | Total spend does not exceed cap + one reservation window. |

## Readiness criteria for documenting `replicas > 1`

Measurable, so the decision is not a judgement call:

1. Every chaos scenario above passes in CI, repeatedly, not once.
2. `librefang_fenced_writes_total` proves fencing fires in the zombie test — a test that never fences is not evidence.
3. A 24-hour soak with 3 replicas and rolling restarts shows zero duplicate externally visible side effects.
4. `verify_integrity` passes on the audit chain after every chaos scenario.
5. Budget overshoot measured under concurrency is bounded by one reservation window.
6. A restore drill from backup boots and verifies clean.
7. The single-node SQLite path still passes its full CI lane, unchanged.

Until all seven hold, `deploy/kubernetes/` documents `replicas: 1` and the manifest keeps the comment saying so.

## Open questions

Genuinely undecided, and each needs an answer before the phase that depends on it:

- **Do followers forward agent turns, or reject them with a redirect?** Forwarding is transparent to clients; redirecting is simpler and pushes routing to the client. Phase 2 can ship either.
- **Should `replicas > 1` require an explicit config opt-in** (`[cluster] enabled = true`) so an operator cannot get multi-replica semantics by editing one number? Leaning yes.
- **How is a stdio MCP server shared?** One child process per replica means N processes with N sets of credentials. Leader-only MCP is the simple answer but makes tool calls leader-only in Phase 3, partly undoing the phase.
- **Does the fencing token belong in the row, or in a per-session sequence?** A sequence is cheaper to compare but harder to reason about across a lease table rewrite.
- **How much of `librefang-runtime`'s per-process state is reachable at all?** The 24 workers are enumerated; the caches inside the runtime crate are not, and Phase 2 needs that inventory before it can claim completeness.
