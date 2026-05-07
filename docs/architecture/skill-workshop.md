# Skill workshop

Passive after-turn capture of reusable workflows (#3328). Detects when a
user is teaching the agent a rule (`from now on always run cargo fmt`,
`no, do it like X`, repeated tool sequences) and stores draft candidate
skills under `~/.librefang/skills/pending/` for human review. Approved
candidates are promoted into the active skill registry through the same
`evolution::create_skill` path that gates marketplace skills, so every
artefact visible to the agent's prompt has crossed the same security
boundary.

The whole subsystem is **on by default with the conservative knob set**:
heuristic-only review (no LLM call), pending policy (every candidate
waits for human approve / reject), 20-candidate cap. An agent that
omits the `[skill_workshop]` block in `agent.toml` gets exactly that.
Operators that want LLM refinement set
`[skill_workshop] review_mode = "threshold_llm"`; operators that want
to disable the feature set `enabled = false`.

The cost regression vs pre-#3328 is bounded: per turn, three regex
scanners on the most recent message and a small toml file when a
candidate lands. No LLM call unless the operator opts in.

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
│    Returns inline when `enabled=false` (only the gates above    │
│    run). Otherwise dashmap get + Arc clone, then step 2.        │
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

Agents that omit the `[skill_workshop]` block in `agent.toml` get the
defaults shown below — heuristic-only capture into pending/. Override
any subset; serde fills the rest from `Default`.

```toml
# agent.toml — explicit form, equivalent to omitting the block:
[skill_workshop]
enabled              = true        # default true
auto_capture         = true        # default true
approval_policy      = "pending"   # "pending" | "auto"
review_mode          = "heuristic" # "heuristic" | "threshold_llm" | "none"
                                   #   ("both" is a serde alias for
                                   #   threshold_llm, kept for openclaw
                                   #   vocabulary compat)
max_pending          = 20          # 0 disables writes (pipeline still runs)
# max_pending_age_days = 30        # Optional TTL — omit (None) to keep
                                   #   the historical "cap-LRU is the only
                                   #   aging mechanism" behaviour.

# To turn the feature off entirely:
# enabled = false

# To get LLM refinement (cheap-tier provider chain):
# review_mode = "threshold_llm"
```

| Field | Default | Effect |
|-------|---------|--------|
| `enabled` | `true` | Master switch. With `false`, the hook returns before scanners run. |
| `auto_capture` | `true` | Lets an enabled agent skip capture without disabling the whole hook (useful for live debugging of an agent that you don't want to disturb). |
| `approval_policy` | `"pending"` | `"pending"` parks candidates in `~/.librefang/skills/pending/<agent>/`. `"auto"` writes the pending file (audit trail) and then promotes via `evolution::create_skill` + reloads the registry so the new skill is visible the next turn — the same security scan applies in both modes. |
| `review_mode` | `"heuristic"` | `"heuristic"` is regex-only (no LLM cost). `"threshold_llm"` ALSO consults the cheap-tier LLM after the heuristic accepts. `"none"` runs the regex scan but discards every hit (testing path). `"both"` is a serde alias for `"threshold_llm"`. |
| `max_pending` | `20` | Per-agent cap. `0` is honoured as "do not store" — the pipeline still runs but `save_candidate` returns `Ok(false)`. |
| `max_pending_age_days` | `None` | Optional TTL. When set, candidates older than `n` days are reaped at the next save (before the cap check). `None` keeps the historical "cap-LRU only" behaviour, so an operator who never reviews their pending tree never silently loses an old candidate. |

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

### Dedup

`save_candidate` skips the write when a pending candidate with the
same `(source kind, name, prompt_context)` already exists for this
agent. Critical for default-on with `RepeatedToolPattern`: the same
recent-window pattern matches every turn until a new tool sequence
pushes it out of the window, so without dedup the operator would
accumulate one duplicate candidate per turn against the cap. The
check is `O(N)` over at most `max_pending` parsed TOML files —
microseconds in practice.

`prompt_context` is part of the key (rather than just `(source kind,
name)`) so two genuinely-distinct teaching signals that happen to
hit `synth_name`'s degenerate fallback path — `captured_rule` /
`captured_correction` / `captured_repeat`, emitted when the head is
empty after sanitisation (e.g. an emoji-only sentence) — do not
false-dedup against each other. Two captures of the *same* teaching
signal produce identical `(source kind, name, prompt_context)` so
this still catches the duplication case.

### Orphan-pending retry (Auto policy)

`approval_policy = "auto"` calls `save_candidate` then
`approve_candidate`. If `evolution::create_skill` fails inside
`approve_candidate` (rare on the happy path; possible after a name
collision with an active skill, a disk-full, etc.), the pending file
stays on disk. Without further action, the next turn's capture
produces the same `(source kind, name, prompt_context)` tuple, gets
short-circuited by the dedup check, and the orphan never gets
retried — the auto agent sees no progress until the operator runs
`librefang skill pending approve <id>` manually.

The Auto branch detects this case: when `save_candidate` returns
`Ok(false)` (dedup OR `max_pending = 0`), it calls
`storage::find_duplicate_pending` and, if a matching entry exists,
runs `approve_candidate` against that entry's id. Success clears the
orphan and triggers the per-turn registry reload as if the new
capture had landed; failure leaves the orphan in place for human
review (logged at WARN with the orphan id). `max_pending = 0` is
naturally a no-op here because the dedup lookup has nothing to find.

### Per-turn registry reload

`run_capture` aggregates the auto-promotion outcome of every hit it
produced and calls `kernel.reload_skills()` at most once per turn.
A turn that triggers all three scanners (explicit instruction +
correction + repeated tool pattern) under `approval_policy = "auto"`
would otherwise pay three `RwLock + read_dir` reloads back-to-back;
the agent loop only consults the registry once at the next turn's
prompt build, so collapsing to one reload is correct and cheap. The
reload runs through `Handle::spawn_blocking` so the supervised
capture task suspends on disk IO rather than blocking the tokio
worker; a `JoinError` (panic / cancel inside the reload) is logged
at WARN with the agent id but does not propagate.

### Aging (optional TTL)

`max_pending_age_days = Some(n)` with `n > 0` reaps any pending
candidate whose `captured_at` is older than `n` days at the next
save, before the cap check. Default is `None` so an operator who
never reviews their pending tree never silently loses an old
candidate — the cap LRU is the only aging mechanism unless the
operator opts in. Combined with the cap, opting in gives a hard
upper bound on both queue length AND queue age.

`max_pending_age_days = Some(0)` is treated as **disabled** rather
than "expire everything immediately". The natural reading of `Some(0)`
for a TTL is "no TTL", and the alternative ("delete every pending
candidate including the one we are about to write") would be a
footgun. To purge the queue, set `max_pending = 0` instead — that
short-circuits `save_candidate` before any write or eviction runs.

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

## Cost model

Three cases, increasing in cost:

**Default config (`enabled=true`, `review_mode="heuristic"`)**

Per turn:
1. Hook gating (`AgentLoopEnd` event compare, `is_fork` flag check, kernel `Weak::upgrade`).
2. `agent_registry().get(agent_id)` — dashmap O(1) + clone of `AgentEntry`.
3. Three regex scanners run over the latest user message + last assistant turn — microsecond-scale.
4. On a hit (rare), one `SkillVerifier::scan_prompt_content` pass over the candidate body / description / provenance excerpts (regex over a few KB of text), then a TOML serialise + atomic file write of a few KB.

No SQLite, no LLM. A turn that does not produce a hit pays only the regex scan; a turn that hits pays the security scan + a small file write.

**LLM-augmented (`review_mode="threshold_llm"` or `"both"`)**

Same as above, plus on each heuristic hit a single auxiliary LLM call (cheap-tier chain: haiku → gpt-4o-mini → openrouter-haiku). With `threshold_llm` the call only runs when heuristics already accepted; with `both` it runs even when heuristics rejected. If no cheap-tier provider is configured, the workshop returns `Indeterminate` rather than billing the call to the operator's primary provider — see [`AuxTask` routing](#auxiliary-llm-routing-auxtaskskillworkshopreview).

**Disabled (`enabled=false`)**

Hook gating runs (steps 1–2 above), then short-circuits on `!cfg.enabled`. No regex scan, no FS, no LLM. The dashmap clone in step 2 is the only non-trivial cost; if it ever shows up in a flame graph, the fix is to peek at `entry().map(|e| e.manifest.skill_workshop.enabled)` without cloning the manifest. Currently below the noise floor.

At kernel boot, `prune_orphan_temp_files` runs in `Handle::spawn_blocking` when a tokio runtime is current, with a sync fallback for `set_self_handle` callers that lack one. Boot does not block on a `read_dir` walk of the pending tree.

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
