# Agent types: storage, dual sourcing, and why `PUT` is a patch

An *agent type* is a reusable agent manifest an operator authors once and spawns from.
It is a plain `AgentManifest` document — the same shape as an agent's own `agent.toml` — and it is served, created, edited and deleted through `/api/templates`.

## One API path, one human word

"Agent type" is the term every human- and agent-facing surface uses: the dashboard page at `/agent-types`, the TUI screen, the store directory, the `agent_type_create` tool, and this document.
The **API path stays `/api/templates`**, and there is deliberately no `/api/agent-types` alias (#7722).

An alias would be permanent — route tables, the OpenAPI document, the generated dashboard client and every operator script would carry two names for one resource forever, and every reader of the route table would have to work out whether the two paths differ in any way.
They would not, which makes the second name pure cost.
The word an operator reads and the path a client posts to are allowed to differ; what is not allowed is two paths that mean the same thing.

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

Creating is the only path that constructs a manifest, because there is nothing on disk to preserve.
There are two callers — `POST /api/templates` and the `agent_type_create` tool — and they share one implementation, `librefang_types::agent_type_store::create_agent_type`.
It validates the name, refuses one that belongs to a live agent, builds the manifest through `AgentTypeSpec::into_new_manifest`, claims the path with `File::create_new`, and fills the claim with an atomic rename.
Each surface keeps only its own vocabulary for the refusals: HTTP status codes and translated operator strings on one side, `ToolError` variants and model-readable prose on the other.

Sharing the write rather than the validation alone is the point.
A rule that lives on one of two writing paths is a rule the other silently does not have, and the name-shadowing check, the `deny_unknown_fields` refusal and the race-free claim are each worth exactly as much as the weaker of the two paths.

`AgentTypeSpec::into_new_manifest` writes an **exhaustive** struct literal — no `..Default::default()` rest pattern.

That is deliberate and load-bearing.
`AgentManifest` gains fields regularly; a rest pattern keeps compiling while quietly deciding, on behalf of whoever added the field, what a newly-created agent type should do with it.
Spelling all fifty-eight out turns that into a compile error at the one place where the decision has to be made.
If you are here because the compiler just told you a field is missing, add it to the literal with a value and a reason — do not reach for `..Default::default()`.

## Writes are atomic

`librefang_types::agent_type_store` renders the manifest, then lands it through a sibling temp file, `sync_all`, `rename`, and a parent-directory sync on Unix.
`std::fs::write` truncates in place, so a failure partway through would leave a truncated `agent.toml` where a valid one used to be, which is the worst possible failure mode for a file the daemon parses at spawn.

A manifest that cannot be re-rendered as TOML is refused with a `500` and nothing is written, rather than partially serialized.

## Surfaces

| Surface | Reads | Writes |
| --- | --- | --- |
| HTTP | `GET /api/templates`, `/api/templates/{name}`, `/api/templates/{name}/toml` | `POST /api/templates`, `PUT`/`DELETE /api/templates/{name}` |
| Dashboard | `src/lib/queries/agentTypes.ts` | `src/lib/mutations/agentTypes.ts`, `src/pages/AgentTypesPage.tsx` |
| TUI | agent types screen (`crates/librefang-cli/src/tui/event.rs`) — daemon backend via HTTP, in-process backend reads the same two directories | — |
| Agent | — | `agent_type_create` tool (`crates/librefang-runtime/src/tool_runner/agent.rs`) via `AgentControl::create_agent_type` |

The TUI spawns from the manifest `GET /api/templates/{name}/toml` serves verbatim; it does not reconstruct one.

All four go through `AgentTypeSpec`, so the agent-facing tool (#7722) inherits the same merge rules by using the same type rather than re-deriving them.
The tool's own schema is the seven flat keys and nothing else, written in sorted key order because a tool definition is stringified into the LLM prompt on every turn that ships it (#3298).
