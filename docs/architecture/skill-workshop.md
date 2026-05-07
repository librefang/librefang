# Skill workshop

Passive after-turn capture of reusable workflows (#3328). Detects when a
user is teaching the agent a rule (`from now on always run cargo fmt`,
`no, do it like X`, repeated tool sequences) and stores draft candidate
skills under `~/.librefang/skills/pending/` for human review. Approved
candidates are promoted into the active skill registry through the same
`evolution::create_skill` path that gates marketplace skills, so every
artefact visible to the agent's prompt has crossed the same security
boundary.

The whole subsystem is **default-off**. An agent only sees capture when
its `agent.toml` carries `[skill_workshop] enabled = true`. With the
default config (`enabled = false`), the after-turn hook does three cheap
synchronous checks and returns before touching the filesystem, SQLite,
or the LLM provider.

## The four-stage pipeline

```
AgentLoopEnd  (per non-fork turn)
     │
     ▼
┌─────────────────────────────────────────────────────────────────┐
│ 1. Hook gating  (SkillWorkshopTurnEndHook, mod.rs)              │
│    - event type == AgentLoopEnd                                 │
│    - !is_fork (skip auto-dream / planning forks)                │
│    - Weak<LibreFangKernel>::upgrade succeeds                    │
│    Returns inline. Cost when disabled: dashmap get + Arc clone. │
└─────────────────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────────────────────────┐
│ 2. Heuristic scan  (heuristic.rs)                               │
│    Three independent scanners; ANY match captures.              │
│    a. ExplicitInstruction — "from now on …", "always …", …      │
│       Filters out conversational subjects ("I", "we", "you")    │
│       and sentence positions other than start.                  │
│    b. UserCorrection      — "no, do it like …", "actually …", … │
│    c. RepeatedToolPattern — same tool sequence ≥ 3 turns        │
│       (length-1 patterns require ≥ 4 occurrences).              │
│    Pure regex + slice work; no IO. Returns `HeuristicHit` with  │
│    a draft name / description / prompt_context body.            │
└─────────────────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────────────────────────┐
│ 3. LLM review  (llm_review.rs, optional)                        │
│    Engaged only when `review_mode = "threshold_llm"` or `both`. │
│    Issues an `AuxTask::SkillWorkshopReview` request through the │
│    cheap-tier fallback chain (haiku → gpt-4o-mini → openrouter- │
│    haiku). Decisions:                                           │
│      • Accept   — heuristic verdict honoured; LLM may refine    │
│                   `name` / `description` (charset & length      │
│                   sanitised before write).                      │
│      • Reject   — candidate dropped before any disk write.      │
│      • Indeterminate — heuristic verdict honoured. Fail-closed: │
│                   parser error, missing cheap-tier credentials, │
│                   driver failure, or any multi-JSON output all  │
│                   land here. The LLM is a refinement, never a   │
│                   gate that an attacker can flip from disk-side │
│                   model output.                                 │
└─────────────────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────────────────────────┐
│ 4. Persist  (storage::save_candidate)                           │
│    a. Security gate — `SkillVerifier::scan_prompt_content` runs │
│       on `prompt_context`, `description`, and both provenance   │
│       excerpts. Critical hits abort with `SecurityBlocked`      │
│       BEFORE any temp file is written.                          │
│    b. Cap — `enforce_cap` evicts oldest by `captured_at` until  │
│       the new candidate fits under `max_pending`. Each eviction │
│       logs at INFO with `evicted_path` + `candidate_id` +       │
│       `captured_at`.                                            │
│    c. Atomic write — body → `<id>.toml.tmp` → fs::rename → done │
│       Crash between write and rename is reaped by               │
│       `prune_orphan_temp_files` at next daemon boot.            │
└─────────────────────────────────────────────────────────────────┘
```

The detached task is supervised — `tokio::spawn` is wrapped by the same
`supervised_spawn` helper that auto_dream uses, so a panic inside any
stage logs `error!` and unwinds without taking down the agent loop.

## Per-agent configuration

```toml
# agent.toml
[skill_workshop]
enabled        = true              # required to see any capture at all
auto_capture   = true              # default true; false short-circuits
                                   # before scanners run
approval_policy = "pending"        # "pending" | "auto"
review_mode    = "heuristic"       # "heuristic" | "threshold_llm" | "both"
max_pending    = 20                # 0 disables write entirely
```

| Field | Default | Effect |
|-------|---------|--------|
| `enabled` | `false` | Master switch. With `false`, the hook returns before scanners run. |
| `auto_capture` | `true` | Lets an enabled agent skip capture without disabling the whole hook (useful for live debugging of an agent that you don't want to disturb). |
| `approval_policy` | `"pending"` | `"pending"` parks candidates in `~/.librefang/skills/pending/<agent>/`. `"auto"` immediately promotes through `evolution::create_skill`, with the same security scan applied. |
| `review_mode` | `"heuristic"` | `"heuristic"` is regex-only. `"threshold_llm"` ALSO consults the cheap-tier LLM after the heuristic accepts; `"both"` runs LLM review even when heuristics say drop. |
| `max_pending` | `20` | Per-agent cap. `0` is honoured as "do not store" — the pipeline still runs but `save_candidate` returns `Ok(false)`. |

The hook re-reads the config from `AgentRegistry` on every fire, so
`agent.toml` edits take effect on the next turn without daemon restart.
This differs from `max_concurrent_invocations`, which is captured at
agent bind time and requires kill-and-respawn (CLAUDE.md convention).

## Storage layout

```
~/.librefang/skills/
  pending/
    <agent_uuid>/
      <candidate_uuid>.toml          ← single CandidateSkill, TOML
      <candidate_uuid>.toml.tmp      ← only present mid-write; pruned at boot
  <skill_name>/                       ← active skills (output of approve)
    skill.toml
    prompt_context.md
    versions/
```

`<agent_uuid>` is the agent's UUID; storage entry points (`save`,
`list`, `load`, `reject`, `approve`) all reject anything that does not
parse as a UUID, collapsing every traversal vector (`..`, `..\\`,
homoglyphs, …) into one positive check. `<candidate_uuid>` is generated
by the hook at capture time.

`list_pending_all` (used by the dashboard) defensively skips child dirs
whose name is not UUID-shaped. A stray `pending/__planted__/` cannot
pollute the listing.

### Concurrency

Single-writer-per-agent is **assumed but not enforced**. The hook fires
at most once per turn per agent; the only path to concurrent writes is
the same agent running multiple parallel turns
(`max_concurrent_invocations > 1` plus `session_mode = "new"`), in
which case the cap check between two saves can transiently observe a
stale directory listing and write one extra candidate before evicting.
The breach is bounded by the in-flight invocation count and self-heals
on the next save. If parallel-invocation usage grows, the upgrade path
is per-agent `fs2::FileExt::lock_exclusive`, mirroring
`librefang_skills::evolution::acquire_skill_lock`.

## Security model

Defense in depth. A candidate body crosses the same prompt-injection
scanner twice and at least one human gate before the agent ever sees
it as a prompt artefact.

| Stage | Surface | Scanner | Behaviour on Critical |
|-------|---------|---------|-----------------------|
| Capture | `save_candidate` | `SkillVerifier::scan_prompt_content` over `prompt_context`, `description`, and both provenance excerpts | Abort with `SecurityBlocked`; nothing reaches disk |
| Promotion | `approve_candidate` → `evolution::create_skill` | Same scanner over `prompt_context` again | Abort; pending file kept so reviewer can edit |
| LLM-refined fields | `apply_refinements` (mod.rs) | Charset + length filter, `[a-z0-9_-]{1,64}` for name, ≤200 chars description | Refinement dropped; heuristic-suggested values kept |

The LLM reviewer is treated as **untrusted output**. The candidate body
shipped to the model is partly user-influenced text, so the model's own
reply could contain attacker-shaped JSON fragments. `strip_json_envelope`
takes leftmost `{` to rightmost `}` — when multiple JSON blocks appear
the slice is malformed, `serde_json::from_str` fails, and the verdict
falls to `Indeterminate`, which routes through the same heuristic
verdict the LLM was reviewing. There is no path from "model output"
to "candidate accepted" that bypasses the heuristic gate.

Excerpt bounds (`PROVENANCE_EXCERPT_MAX_CHARS = 800`) are enforced in
characters, not bytes, so multibyte truncation never panics on UTF-8
boundaries.

## Cost model when disabled

Per turn, for an agent with `enabled = false`:

1. `HookContext.event == AgentLoopEnd` — pointer compare.
2. `is_fork` lookup in `ctx.data` — `serde_json::Value::get` over a
   small map.
3. `Weak::upgrade` of the kernel reference.
4. `agent_registry().get(agent_id)` — dashmap O(1), clones an
   `AgentEntry` (full `AgentManifest`).
5. `if !cfg.enabled { return; }` — boolean compare.

No SQLite, no FS, no LLM. The dashmap clone in step 4 is the only
non-trivial cost; if it ever shows up in a flame graph, the fix is to
peek at the `enabled` field via `entry().map(|e| e.manifest.skill_workshop.enabled)`
without cloning the manifest. Currently it is below the noise floor.

At kernel boot, `prune_orphan_temp_files` ran inline before #4741's
defense-in-depth followup; it now hops to `Handle::spawn_blocking`
when a tokio runtime is current, with a sync fallback for the rare
`set_self_handle` callers that lack one. Boot stays off the FS for
agents that never enabled the workshop.

## Auxiliary LLM routing (`AuxTask::SkillWorkshopReview`)

Skill review is a **separate** `AuxTask` slot from
`AuxTask::SkillReview` (which is owned by
`kernel::messaging::background_skill_review`). They share the same
default cheap-tier chain in `aux_client::default_chain` —
`haiku → gpt-4o-mini → openrouter-haiku` — but configuration changes
to one do not silently affect the other.

`AuxClient::resolve` returns `used_primary = true` when no cheap-tier
credentials are configured. The workshop respects this signal and
returns `ReviewDecision::Indeterminate` rather than billing review
calls to the user's primary (paid) provider. A passive subsystem
turning on premium calls would be a financial DoS; the check is a
hard gate, not a soft preference.

## CLI

```
librefang skill pending list [--agent <uuid>]
librefang skill pending show <candidate_uuid>
librefang skill pending approve <candidate_uuid>
librefang skill pending reject <candidate_uuid>
```

Approval is the only path that promotes a candidate. There is no
"shadow" promotion that bypasses the second security scan — the API
route shares the same `storage::approve_candidate` entry point.

## HTTP

| Method | Path | Returns |
|--------|------|---------|
| `GET` | `/api/skills/pending` | List for all agents (`?agent=<uuid>` filters) |
| `GET` | `/api/skills/pending/{id}` | Single candidate |
| `POST` | `/api/skills/pending/{id}/approve` | Promote, return new skill name + version |
| `POST` | `/api/skills/pending/{id}/reject` | Drop without promoting |

All four routes are authenticated (no entry in the `is_public`
allowlist). `WorkshopError::InvalidId` round-trips as 400; not-found
as 404; security-block / promotion conflicts as 409.

## File map

- `crates/librefang-kernel/src/skill_workshop/`
  - `mod.rs`            — hook + `run_capture` pipeline
  - `candidate.rs`      — `CandidateSkill`, `CaptureSource`, `Provenance`
  - `heuristic.rs`      — three regex scanners
  - `llm_review.rs`     — JSON-contract review prompt + parser
  - `storage.rs`        — pending writer + cap eviction + approve
- `crates/librefang-kernel/src/kernel/bindings_and_handle.rs` — hook
  registration in `set_self_handle`, alongside `auto_dream`
- `crates/librefang-types/src/agent.rs` — `SkillWorkshopConfig`
- `crates/librefang-types/src/config/types.rs` — `AuxTask::SkillWorkshopReview`
- `crates/librefang-runtime/src/aux_client.rs` — cheap-tier fallback chain
- `crates/librefang-api/src/routes/skills.rs` — HTTP routes (lines ~500–680)
- `crates/librefang-api/dashboard/src/components/PendingSkillsSection.tsx`
  — dashboard surface
- `crates/librefang-cli/src/main.rs` — `skill pending` subcommands
- `crates/librefang-api/tests/skill_workshop_pending_routes_test.rs`
  — integration tests
