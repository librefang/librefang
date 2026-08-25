# Workflow run attribution: who owns a run, and whose budget a step spends

Two questions a workflow run has to be able to answer, and they have different answers:

- **Who asked for this run?** — recorded on the run as `owner_agent_id`.
- **Whose budget line does this LLM call land on?** — recorded on the usage event as `billed_agent_id`.

Both columns arrive in schema **v49** (`crates/librefang-memory/src/migration.rs: migrate_v49`), and both are nullable, because both questions legitimately have no answer for an operator-initiated run made by no agent at all.

## Ownership lives on the run, not on the agent

A step written as `{ type = "researcher" }` resolves through find-or-spawn (`crates/librefang-kernel/src/kernel/step_agent.rs: find_or_spawn_agent_type`) to the *canonical* name-derived instance for that type (#4614).
That is deliberate: a step agent outlives the run that first needed it and is shared by every later run of every workflow that references the type, which is also why it is spawned top-level with `parent = None` rather than being parented to whichever run happened to be first.

The consequence is that the executing agent cannot carry ownership.
If two agents each start a workflow whose first step is `{ type = "researcher" }`, both runs execute on the same `researcher` instance, with the same agent id.
Hanging ownership off that instance would make the second owner's runs report the first owner's, and the report would look perfectly consistent while being wrong.

So the owner is a property of the run.
`WorkflowRun::owner_agent_id` is stamped once at creation by `WorkflowEngine::create_run_owned` and never reassigned.
`WorkflowEngine::create_run` is the ownerless entry point and is what an operator-initiated run uses.

Where the owner comes from, per entry point:

| Entry point | Owner |
| --- | --- |
| `workflow_run` tool | the calling agent, forwarded as `caller_agent_id` through `WorkflowRunner::run_workflow_owned` |
| `workflow_start` tool | the calling agent, via the `caller_agent_id` `start_workflow_async_tracked` already receives |
| Channel `/workflow run` | the agent bound to that channel and chat, resolved by the same binding every other channel command uses |
| `POST /api/workflows/{id}/run` | none — an operator action |
| `POST /api/workflows/runs/{id}/rerun` | copied off the original run |

The re-run row deserves its own sentence, because it is the case that is easy to get wrong.
A re-run is a repeat of the same work on the same owner's behalf, and the operator who pressed the button is not that owner — so the owner is read off the stored run rather than re-derived from the caller.
The resume path inherits the owner for free: it mutates the existing run rather than creating a new one.

`workflow_start` parses `caller_agent_id` for ownership *independently* of the async-task tracker, which registers only when both a caller agent and a caller session are present.
A `workflow_start` with an agent and no session is still owned by that agent.

## Billing rolls up to the spawner, and does not disturb quotas

`UsageRecord::billed_agent_id` is set at every LLM write site to `entry.parent.unwrap_or(agent_id)` (`crates/librefang-kernel/src/kernel/agent_execution.rs: billed_agent_for`).
A worker spawned by another agent spends on its spawner's behalf, so the spawner needs that cost on its own budget line rather than scattered across throwaway children it cannot enumerate.
A top-level agent has no parent and bills to itself, which is the behaviour every agent had before this existed.

`billed_agent_id` is a **separate column from `agent_id`, not a rewrite of it**, and that is the whole design.

`agent_id` remains the quota subject. The pre-call check is `check_quota(agent_id, &entry.manifest.resources)`; the post-call check is `check_all_and_record(&record, &manifest.resources, ..)` keyed on `record.agent_id`.
Both therefore ask about the same agent, against that same agent's ceiling.

Re-pointing `agent_id` at the parent would have broken that pairing in two ways at once: the pre-call check would read the child's accumulated spend while the post-call check read the parent's, and both would compare against the *child's* limits.
Attribution and enforcement are independent dimensions and have to stay independent.

Read the rollup with `UsageStore::query_billed_summary(agent_id)`, which sums `COALESCE(billed_agent_id, agent_id) = agent_id` — the agent's own calls plus every call a worker made on its behalf.
`query_summary` still answers "what did this agent execute", which stays the right question for quota work.

## Why there is no `fresh = true` on a step agent

The original proposal for #7714 included a third form, `{ type = "x", fresh = true }`, spawning a new random-id instance with a per-run name tag.
It is not implemented, and the reason is that both things it was for now have answers.

Its stated motivation was that find-or-spawn "always reuses the canonical instance, which carries cross-run history".
That is a statement about **history**, and per-step `session_mode` (#7862) answers it directly: `session_mode = "new"` on a step mints a fresh `SessionId` for that step's invocation regardless of what the target agent's manifest defaults to, so the step sees none of the agent's prior state.
That is a fresh session per run without a second agent per run.

The other thing an instance-level flag would have bought is a distinct budget line per owner, and that is what `owner_agent_id` and `billed_agent_id` above provide — without registry churn, and correctly for the case that actually motivates it, which is two *different* owners sharing one agent type.

What `fresh = true` would still add is a distinct agent *identity* per run, and the cost of that is concrete: one registry entry, one workspace, one scheduler bootstrap and one set of channel/cron registrations per run, accumulating forever, with no reaper.
Bounded registry growth is the reason find-or-spawn resolves to a canonical id in the first place.
Paying that to isolate history that `session_mode` already isolates, and to separate budgets that ownership already separates, is not a trade worth making.

**If you want a workflow step isolated from an agent's prior context, set `session_mode = "new"` on the step.**
If a future case genuinely needs a distinct workspace or a distinct capability ceiling per run — the two things a session cannot give you — that is a different feature from the one this note declines, and it should be specified against those requirements rather than resurrected from this one.
