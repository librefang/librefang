# Agent types: storage, dual sourcing, and why `PUT` is a patch

An *agent type* is a reusable agent manifest an operator authors once and spawns from.
It is a plain `AgentManifest` document — the same shape as an agent's own `agent.toml` — and it is served, created, edited and deleted through `/api/templates`.

## Where they live

| Source | Path | Written by this API |
| --- | --- | --- |
| `agent-type` | `~/.librefang/agent-types/{name}.toml` | yes |
| `agent` | `~/.librefang/workspaces/agents/{name}/agent.toml` | no |

The catalog is dual-source on purpose: every live agent's manifest is also spawnable-from, and `GET /api/templates` has listed those since before operator-authored types existed.
Only the first source is a file this API owns.
The second belongs to a running agent and is edited through `/api/agents/{id}`.

Every row `GET` returns therefore carries `source` and `editable`, and `PUT`/`DELETE` refuse a name that resolves to the second source with `409` and the code `template_not_editable` — not a bare `404`, which would tell an operator nothing about why a row they can see will not save.
Clients render those rows as managed elsewhere rather than offering an Edit control that cannot succeed.

Name collisions resolve to the `agent-type` copy, because that is the copy the write verbs act on: if `Edit` loaded a live agent's manifest and `Save` wrote the agent-type file, the operator would be editing one document and saving another.
`POST /api/templates` refuses a name that already belongs to a live agent (`template_name_taken`) so the collision cannot be created deliberately; one that arises afterwards is logged at `WARN`.

## The flat shape, and why it is a patch

Editors do not render fifty-eight manifest fields.
They render seven — `name`, `description`, `system_prompt`, `provider`, `model`, `tools`, `skills` — and that projection is what `librefang_types::agent_type::AgentTypeSpec` describes.

The projection is lossy, so **`PUT /api/templates/{name}` is a read-modify-write, not a constructor**.
The handler parses the stored manifest, applies the request body over it field by field, and writes the merged result.
A key the client did not send leaves its field exactly as it was found.

This is the difference between a working editor and silent data loss.
Rebuilding the document from a seven-key body resets the other fifty-one fields to their defaults and answers `200` — `[[triggers]]`, `[compaction]`, `max_history_messages`, `mcp_servers`, `tool_allowlist`, `session_mode`, `[workspaces]`, `channels`, `[exec_policy]` and `fallback_models` all disappear the first time anyone opens the editor and saves (#7740).

Three consequences worth stating explicitly:

- **Absent and empty are different instructions.** `"system_prompt": ""` clears the prompt; omitting the key keeps whatever is on disk. Every field of `AgentTypeSpec` is `Option`, which is what makes the distinction expressible — `unwrap_or("You are a helpful AI agent.")` cannot express it, and writes canned text over an operator's deliberate blank.
- **Unknown keys are rejected, not ignored.** `AgentTypeSpec` is `deny_unknown_fields`. Under patch semantics a typo'd key would otherwise deserialize to "field absent", read as "keep the old value", drop the edit, and still answer `200`.
- **Identity comes from the URL.** A `name` in the body is ignored by `PUT`; it would move the document out from under the path that addressed it.

## Creating one

`POST /api/templates` is the only path that constructs a manifest, because there is nothing on disk to preserve.
`AgentTypeSpec::into_new_manifest` writes an **exhaustive** struct literal — no `..Default::default()` rest pattern.

That is deliberate and load-bearing.
`AgentManifest` gains fields regularly; a rest pattern keeps compiling while quietly deciding, on behalf of whoever added the field, what a newly-created agent type should do with it.
Spelling all fifty-eight out turns that into a compile error at the one place where the decision has to be made.
If you are here because the compiler just told you a field is missing, add it to the literal with a value and a reason — do not reach for `..Default::default()`.

## Writes are atomic

`persist_agent_type` renders the manifest, then lands it through `crate::atomic_write` — a sibling temp file, `sync_all`, `rename`.
`std::fs::write` truncates in place, so a failure partway through would leave a truncated `agent.toml` where a valid one used to be, which is the worst possible failure mode for a file the daemon parses at spawn.

A manifest that cannot be re-rendered as TOML is refused with a `500` and nothing is written, rather than partially serialized.

## Surfaces

| Surface | Reads | Writes |
| --- | --- | --- |
| HTTP | `GET /api/templates`, `/api/templates/{name}`, `/api/templates/{name}/toml` | `POST /api/templates`, `PUT`/`DELETE /api/templates/{name}` |
| Dashboard | `src/lib/queries/agentTypes.ts` | `src/lib/mutations/agentTypes.ts`, `src/pages/AgentTypesPage.tsx` |
| TUI | templates screen (`crates/librefang-cli/src/tui/event.rs`) — daemon backend via HTTP, in-process backend reads the same two directories | — |

The TUI spawns from the manifest `GET /api/templates/{name}/toml` serves verbatim; it does not reconstruct one.

All three go through `AgentTypeSpec`, so a future agent-facing authoring tool (#7722) inherits the same merge rules by using the same type rather than re-deriving them.
