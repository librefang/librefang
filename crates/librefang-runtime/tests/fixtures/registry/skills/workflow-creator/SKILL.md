---
name: workflow-creator
description: Compose durable multi-step workflows with the workflow_create tool — step shape, agent binding, required skills, and the validation errors worth avoiding
version: 0.1.0
author: librefang
tags: [productivity, automation, workflow]
---
# Composing Workflows with `workflow_create`

A workflow is a named, persisted sequence of steps that dispatches each step to an agent and threads the outputs together.
`workflow_create` registers one; `workflow_run` / `workflow_start` execute it afterwards, and it outlives the conversation that created it.

## Decide whether a workflow is the right shape

Reach for `workflow_create` when the same multi-agent sequence is worth repeating, when it needs to be startable later by a cron job or another agent, or when the steps should run as a DAG rather than one long turn.
For something you will do once, message the agents directly with `agent_send` — a workflow you never run again is a name permanently taken on the daemon.

Call `workflow_list` before creating.
Names are unique across the daemon and the comparison is case-insensitive, so `Deploy` collides with an existing `deploy`.
A collision is rejected outright and nothing is overwritten; the error names the workflow that already holds the name.

## The step shape

Every step requires `name`, `agent`, and `prompt_template`.

- `name` is unique within the workflow and is how `depends_on` addresses the step.
- `prompt_template` is the text sent to the agent. `{{input}}` interpolates the previous step's output; `{{whatever}}` interpolates a declared input parameter or an earlier step's `output_var`.
- `output_var` stores this step's output under a name later steps reference as `{{that_name}}`. Without it, only the immediately following step can read the output, via `{{input}}`.
- `depends_on` lists step names that must finish first. A step may name one declared later in the array — execution is topological, not positional.
- `timeout_secs` is the wall-clock budget for the step: default 120, ceiling 3600.
- `error_mode` is `"fail"` (default, abort the run), `"skip"` (continue without this step's output), or `{"retry": {"max_retries": 3}}`. The retry form also accepts `backoff_ms` and `jitter_pct`.
- `mode` is `"sequential"` (default), `"fan_out"` to run alongside the following `fan_out` steps, or `"collect"` to gather them. Richer nodes take a tagged object, such as `{"conditional": {"condition": "APPROVED"}}`.

A workflow holds at most 50 steps, and `total_timeout_secs` caps the whole run at 86400 seconds.
The workflow `name` itself is 1–64 characters of letters, digits, `_` and `-`.

## Agent binding

The `agent` field accepts four shapes, and exactly one routing key in the object forms:

| Written as | Meaning |
|---|---|
| `"researcher"` | By name — shorthand for the object below. The first registered agent answering to that name runs the step. |
| `{"name": "researcher"}` | By name, spelled out. |
| `{"id": "<uuid>"}` | By UUID — binds to one specific agent instance and survives a rename. |
| `{"type": "researcher"}` | By agent *type* — find-or-spawn. A registered agent of that name is reused; otherwise the template of that name is loaded and spawned. |

Supplying none of `id` / `name` / `type`, or more than one, is a deserialization error rather than a silently-preferred key.

Prefer `{"type": ...}` when the workflow should stand on its own on a fresh install, because it does not require the operator to have pre-registered anything.
Prefer `{"id": ...}` when the step must reach one particular long-running instance.
Bare-string / `{"name": ...}` binding fails at run time if no agent answers to that name, so check `agent_list` first.

Two optional per-step fields shape how the agent is invoked:

- `session_mode` is `"persistent"` (reuse the target agent's long-running session, threading this step into its context) or `"new"` (a fresh session, isolated from prior state). Omit it to defer to the agent's own `session_mode`.
- `inherit_context` set to `false` suppresses parent-workflow context injection for this step regardless of the agent's setting.

## `required_skills`

`required_skills` lists skills the step's agent must actually be able to use.
The check runs right after agent resolution and before the prompt is built, so an unmet requirement fails with a named error and bills no LLM call.

Each name is resolved against the loaded skill registry independently of the agent's allowlist, which means an unrestricted agent cannot mask a requirement for a skill that is not installed.
The error distinguishes three cases, because each has a different fix: the skill is loaded but the agent's `skills` allowlist does not admit it; the agent declares it but nothing on the instance provides it; or nothing provides it and the agent never named it either, which is usually a typo.
An agent with `skills_disabled = true` fails every requirement regardless of the registry.

Use it for the one or two skills a step genuinely cannot work without.
Listing every skill an agent happens to have turns a workflow into something that only runs on the machine it was written on.
An empty or whitespace-only entry is rejected at creation.

## Declaring inputs

`input_schema` declares what callers pass to `workflow_run`, and each entry becomes a `{{name}}` placeholder available to every step's `prompt_template`.

The type key is `param_type`, not `type` — this is the spelling `workflow_describe` reports back.
Values are `string` (the default), `number`, `boolean`, `file`, `image`, or `agent_id`; `file` and `image` document that the caller may pass an artifact reference.
`required` defaults to true.

Declaring it is worth the few extra lines.
When it is absent, `workflow_describe` falls back to scanning step prompts for `{{var}}` placeholders and reports every one it finds as a required string with no description — so the caller learns the parameter names but nothing about what they mean.

## Worked example

```json
{
  "name": "release-notes",
  "description": "Draft release notes from a version tag, then fact-check them against the changelog",
  "steps": [
    {
      "name": "collect",
      "agent": {"type": "researcher"},
      "prompt_template": "List every merged pull request in {{repo}} since tag {{since_tag}}. One line each: number, title, author.",
      "output_var": "merged",
      "timeout_secs": 600
    },
    {
      "name": "draft",
      "agent": {"type": "technical-writer"},
      "prompt_template": "Write release notes for {{repo}} covering these changes. Group by theme and explain why each matters:\n{{merged}}",
      "depends_on": ["collect"],
      "output_var": "draft"
    },
    {
      "name": "fact-check",
      "agent": {"type": "code-reviewer"},
      "prompt_template": "Check this draft against the merged list. Flag any claim the list does not support.\n\nDraft:\n{{draft}}\n\nMerged:\n{{merged}}",
      "depends_on": ["collect", "draft"],
      "required_skills": ["git-expert"],
      "error_mode": "skip"
    }
  ],
  "input_schema": [
    {"name": "repo", "param_type": "string", "required": true, "description": "owner/name of the repository"},
    {"name": "since_tag", "param_type": "string", "required": true, "description": "Tag to diff from, e.g. v0.4.0"}
  ],
  "total_timeout_secs": 3600
}
```

`fact-check` reads both `{{draft}}` and `{{merged}}`, which is why `collect` sets an `output_var` even though `draft` could have read it as `{{input}}`.
It is `error_mode: "skip"` because a missing fact-check is worse than no release notes at all, but not worth failing the run over.

## Failure modes worth avoiding

**Depending on a step that does not exist.** `depends_on` entries are checked against the step names in the same workflow and a miss is rejected at creation, with the offending pair named. A forward reference is fine; a typo is not.

**Reusing a step name.** Step names address dependencies, so duplicates are rejected rather than disambiguated.

**Operator nodes inside a DAG.** `wait`, `gate`, `approval`, `transform`, `branch` and `operator` steps execute only on the sequential path. Combining one with `depends_on` is rejected at creation, because the DAG executor would try to dispatch it to an agent instead.

**A `prompt_template` on an operator node.** `wait`, `gate`, `approval`, `branch` and `operator` ignore the field at run time, so a non-empty template there is rejected rather than silently discarded. Leave it empty or use `{{input}}`.

**Reaching for `{{input}}` across a fan-out.** `{{input}}` is the *previous* step's output. Once steps run in parallel, "previous" is not well defined — give each parallel step an `output_var` and read those by name.

**Treating the workflow as private.** Workflows have no ownership model: the moment one is registered, any agent on the daemon can run it and any operator can read its step prompts. Keep credentials and personal data out of `prompt_template` and pass them as inputs at run time.

**Assuming it disappeared with the conversation.** A created workflow is written to `~/.librefang/workflows/<id>.workflow.json` and reloaded at daemon start. It is durable and it holds its name until someone removes it.

## After creating

`workflow_create` returns `{id, name, description, step_count, has_input_schema}`.
The workflow is runnable immediately: `workflow_run` executes it and waits, `workflow_start` returns a run id straight away, `workflow_status` polls that run, and `workflow_describe` reports the parameters back to whoever calls it next.
