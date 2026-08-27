# Agent manifest field parity

`AgentManifest` (`crates/librefang-types/src/agent.rs`) has 59 top-level fields.
This page records, for each one, whether an operator can change it on an **already-running** agent from the HTTP API and from the TUI, and — where they cannot — whether that is a gap or a deliberate rule.

Refs #7742, which opened as an open-ended epic.
Writing the inventory down is what converts it into a finite, checkable statement.

The field count is guarded by `librefang_types` test `the_populated_fixture_covers_every_serialized_manifest_key`, so a 59th field cannot be added without someone updating this page.

## How to read the two columns

**API** means `PATCH /api/agents/{id}`.
That route has two modes, and only one of them is general: a body carrying `manifest_toml` is parsed into a whole `AgentManifest` and handed to `LibreFangKernel::update_manifest`, which replaces the registry entry, re-grants capabilities, refreshes the scheduler quota, saves to SQLite, and writes `agent.toml`.
Every other key on that route (`name`, `description`, `model`, `system_prompt`, `mcp_servers`, `schedule`, `auto_evolve`) is a shortcut to a dedicated registry method.
So the API column is "yes" for anything the whole-manifest path carries, and the interesting rows are the three it deliberately does not.

**TUI** means editing an existing agent from `librefang tui` → Agents → detail.
The custom-agent builder can set roughly ten fields *at creation*, which is a different thing: it writes a fresh manifest, it cannot read one back, and nothing in that flow is reachable once the agent exists.

Durability is part of "editable".
Boot reconciliation re-reads each agent's `agent.toml` and overwrites the SQLite projection when the two disagree, so an edit that reaches only the database is an edit the next restart discards.
Every "yes" below is asserted through a write-then-`reload_agent_from_disk` round trip in `crates/librefang-api/tests/agent_manifest_persist_test.rs`, or through the serialize/parse round trip in `crates/librefang-types/tests/manifest_field_parity.rs`.

## The table

| # | Field | API | TUI | Note |
|---|---|---|---|---|
| 1 | `name` | via `PATCH {"name"}` only | no | Deliberately pinned inside `update_manifest` — see *Deliberately not writable* |
| 2 | `version` | yes | no | |
| 3 | `description` | yes | no | Also `PATCH {"description"}` and `PATCH /config` |
| 4 | `author` | yes | no | |
| 5 | `owner` | yes | no | The principal turns with no authenticated caller act for (#7744); an operator spec string, `user:<name>` / `group:<name>` |
| 6 | `module` | yes | no | Path-escape validated on every write (`validate_manifest_module_path`, #3533) |
| 7 | `schedule` | yes | no | Also `PATCH {"schedule"}`, which additionally restarts the background loop (#4984) |
| 8 | `session_mode` | yes | no | Takes effect on respawn |
| 9 | `model` | yes | no | Also `PATCH {"model"}` / `PATCH /config` |
| 10 | `fallback_models` | yes | no | Also `PATCH /config` |
| 11 | `resources` | yes | no | Quota cache refreshed in place |
| 12 | `priority` | yes | no | |
| 13 | `capabilities` | yes | no | Re-granted in place |
| 14 | `profile` | yes | no | |
| 15 | `tools` (per-tool params) | yes | no | |
| 16 | `skills` | yes | **yes** | `PUT /skills`; TUI detail, `s` |
| 17 | `skills_disabled` | yes | no | `PUT /skills` clears it as a side effect |
| 18 | `mcp_servers` | yes | **yes** | `PUT /mcp_servers`; TUI detail, `m` |
| 19 | `channels` | yes | **yes — added here** | `PUT /channels`; TUI detail, `n` |
| 20 | `mcp_disabled` | yes | no | |
| 21 | `metadata` | yes | no | |
| 22 | `tags` | **no** | no | The one API gap. In flight in #7866 — see *Known gaps* |
| 23 | `routing` | yes | no | |
| 24 | `autonomous` | yes | no | |
| 25 | `pinned_model` | yes | no | |
| 26 | `workspace` | yes, with a guard | no | Preserved when the incoming manifest omits it — see *Deliberately not writable* |
| 27 | `generate_identity_files` | yes | no | Read at spawn only |
| 28 | `workspaces` | yes | no | |
| 29 | `exec_policy` | yes | no | |
| 30 | `tool_allowlist` | yes | no | Also `PUT /tools` |
| 31 | `tool_blocklist` | yes | no | Also `PUT /tools` |
| 32 | `tools_disabled` | yes | no | `PUT /tools` cannot set it, and clears it — in flight in #7866 |
| 33 | `response_format` | yes | no | |
| 34 | `enabled` | yes | no | |
| 35 | `allowed_plugins` | yes | no | |
| 36 | `inherit_parent_context` | yes | no | |
| 37 | `thinking` | yes | no | |
| 38 | `context_injection` | yes | no | |
| 39 | `is_hand` | yes — arguably should not be | no | Provenance, and it is read. See *Known gaps* |
| 40 | `web_search_augmentation` | yes | no | Also `PATCH /config` |
| 41 | `auto_dream_enabled` | yes | no | |
| 42 | `auto_dream_min_hours` | yes | no | |
| 43 | `auto_dream_min_sessions` | yes | no | |
| 44 | `show_progress` | yes | no | |
| 45 | `auto_evolve` | yes | no | Also `PATCH {"auto_evolve"}` |
| 46 | `channel_overrides` | yes | no | |
| 47 | `max_history_messages` | yes | no | |
| 48 | `max_concurrent_invocations` | yes | no | Cap is sized once per spawn; needs kill + respawn |
| 49 | `assignee_wake` | yes | no | |
| 50 | `cache_context` | yes | no | |
| 51 | `tool_exec_backend` | yes | no | Picked up on respawn |
| 52 | `skill_workshop` | yes | no | |
| 53 | `proactive_memory` | yes | no | |
| 54 | `compaction` | yes | no | |
| 55 | `context_engine` | yes | no | Picked up on respawn |
| 56 | `rl_export` | yes | no | |
| 57 | `triggers` | yes | no | An entry with no `pattern` used to make the whole file unwritable — see below |
| 58 | `reconcile_orphans` | yes | no | |
| 59 | `async_tasks` | yes | no | |

## Deliberately not writable

These three are not gaps, and making them freely writable would be a defect rather than parity.

**`name`** is pinned to the current value inside both `update_manifest` and `reload_agent_from_disk`.
`AgentRegistry` keeps `AgentEntry::name` and a `name_index` beside the manifest, and a manifest swap updates neither, so a rename through the whole-manifest path would leave `find_by_name` resolving the old string while the manifest claimed the new one.
`PATCH {"name": …}` routes through `AgentRegistry::update_name`, which maintains all three.
The rule is therefore "renaming has one door", not "renaming is unsupported".
Asserted by `manifest_toml_cannot_rename_an_agent_out_from_under_the_registry`.

**`workspace`** is preserved when the incoming manifest leaves it unset, and honoured when it is set.
The asymmetry is the point: the workspace path is populated at spawn with a real directory, and a client that assembles a partial manifest without it would otherwise orphan the agent from its own home, its identity files, and its named workspace mounts.
An operator who genuinely wants to relocate an agent still can, by sending the path.

**The `hand:` / `hand_role:` / `hand_instance:` prefixes of `tags`** are system-owned, not decoration.
`workspace_setup.rs` routes a hand agent's workspace to `hands/<hand>/<role>` off them, `approval_gate.rs` decides whether a tool call needs an approval gate off them, `messaging.rs` marks the agent autonomous for idle-wake off them, and `librefang-memory` scopes structured memory off them.
Letting a manifest edit invent a `hand:` tag would move a workspace and re-decide an approval boundary through a field that reads like free-form metadata.
The operator-owned half of `tags` is a real gap; the system-owned half is a rule.
#7866 implements exactly that split.

## Known gaps

**`tags` and `tools_disabled`** — in flight in #7866, not duplicated here.
`update_manifest` overwrites `new_manifest.tags` with the spawn-time snapshot, so no route into it can change a tag.
`AgentRegistry::update_tool_config` clears `tools_disabled` on every successful write, so saving an unrelated tool filter silently re-enables a disabled agent's tools.

**`is_hand` is writable and probably should not be.**
It is provenance — `hands_lifecycle.rs` sets it when a Hand spawns an agent — and it is not merely stored.
`librefang-memory/src/structured.rs` reads `manifest.is_hand` when rebuilding an `AgentEntry` out of SQLite, which makes it the persisted seed for `AgentEntry::is_hand`, and that field has around fifty readers across workspace routing, the approval gate, idle-wake and memory scoping.
`manifest_privacy.rs` also reports it when classifying what a manifest may disclose.
`update_manifest` does not pin it, so a whole-manifest write can flip it in either direction, and the change takes effect on the next daemon start.
This is not fixed here because the pin belongs in the same lines of `update_manifest` that #7866 is currently rewriting for `tags`, and that PR's reasoning about `AgentEntry::is_hand` versus `manifest.is_hand` is the right place to settle it.

**The TUI can edit three fields of 59 on a running agent.**
Closing the rest one field at a time is not the shape of the answer — a TUI equivalent of the dashboard's `AgentManifestForm` is, and that needs a manifest read endpoint (`GET /api/agents/{id}/manifest`, in flight in #7749) before the daemon-backed TUI can do a read-modify-write at all.
Today the whole-manifest path is write-only over HTTP: a client can `PATCH` 59 fields but cannot fetch the 59 it is about to overwrite.

**There is no concurrency control on the whole-manifest route.**
Two clients that both read, edit one field, and write will silently lose one edit.
Rare today because whole-manifest writes are rare; it becomes routine the moment an editor is mounted on that route.

## Why a bad trigger froze every other field

`ManifestTrigger` carries `#[serde(default)]` at the struct level, so a `[[triggers]]` block that omits `pattern` parses fine and leaves `pattern` as `serde_json::Value::Null`.
TOML has no null.
`persist_full_manifest_at` serializes the entire `AgentManifest` with `toml::to_string_pretty`, so one such entry failed the write for all 59 fields at once — and the failure was a swallowed log, so `update_manifest` still answered `200 OK`.
The agent's `agent.toml` froze at its previous contents while the in-memory copy kept accepting edits, and the next restart restored the frozen file over every one of them.

`pattern` now carries `skip_serializing_if = "Value::is_null"`.
The kernel already treats a null pattern as inert — `reconcile_manifest_triggers` skips it with a `warn!` and continues — so emitting nothing and reading it back as null is the honest round trip.
The serialization failure path also logs at `error!` now, naming the stale path and the consequence, because "your next restart will discard this" is not a warning.

Regression: `a_trigger_with_no_pattern_does_not_block_manifest_serialization` and `a_pattern_less_trigger_does_not_freeze_the_rest_of_the_manifest`.
