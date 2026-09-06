# What automatic memory is allowed to see (#7605)

The proactive memory store is keyed by **agent**, not by conversation.
One row in `memories` belongs to `agent_id`, and every turn that agent takes is a candidate reader.
That is the right shape for a personal assistant and the wrong shape for anything serving more than one person, which is how #7605 was reported: a pre-sales agent on a public website auto-memorized one visitor's customer code and auto-retrieved it into the next visitor's turn, even though the two turns were addressed to different `session_id`s and their message histories never touched.

Three filters now decide whether a memory reaches a prompt.
They are independent, they are checked in different places, and none of them subsumes another.

## 1. Does the agent have the capability at all

`capabilities.memory_read` and `capabilities.memory_write` in `agent.toml` gate the automatic paths, resolved by `ManifestCapabilities::allows_own_memory_read` / `allows_own_memory_write`.

There are **two** automatic paths on each side and the gate is applied to both.
On the read side `gated_proactive_memory_for_retrieve` (`crates/librefang-runtime/src/agent_loop/end_turn.rs`) closes `auto_retrieve`, and `RecallSetupContext::memory_read_allowed` closes the substrate recall and the context engine — `setup_recalled_memories` checks it before issuing any query, so a denied agent runs none at all.
On the write side `gated_proactive_memory_for_memorize` closes `auto_memorize`, and the same `allows_own_memory_write` check at the `remember_interaction_best_effort` call site closes the per-turn episodic writer.
Gating only the two proactive halves is what the first version of this shipped, and it left the larger half of the store open in both directions: the verbatim exchange written on every turn, and the recall that reads it back.

The two lists are tri-state, which is unusual in a manifest and deliberate:

| In `agent.toml` | Parsed as | Automatic memory |
| --- | --- | --- |
| key absent | `None` | allowed |
| `memory_read = []` | `Some([])` | denied |
| `memory_read = ["kv:*"]` | `Some(["kv:*"])` | denied — the scope does not cover this store |
| `memory_read = ["*"]` / `["self.*"]` / `["proactive"]` | covering | allowed |

Everywhere else in a manifest an empty list reads as "undeclared, therefore unrestricted" — `capabilities.tools = []` grants every tool.
Keeping the distinction here is what lets `memory_read = []` mean what an operator typing it means, without failing closed on the many manifests that have no `[capabilities]` block at all.
The accepted scope strings come from `librefang_types::capability::scope_covers_own_memory`, shared with the `memory_semantic_*` tool gate (#7808) so the two cannot drift.

Sharing the scope vocabulary is not sharing the tri-state, and the difference is worth stating because it bounds what `memory_read = []` buys you.
`manifest_to_capabilities` (`crates/librefang-kernel/src/kernel/manifest_helpers.rs`) flattens both `None` and `Some([])` into zero `Capability::MemoryRead` entries, and the tool gate in `Kernel::available_tools` keeps a semantic-memory tool whenever the scope list it sees is empty — deliberately, so that the many manifests declaring nothing can still reach the feature.
So a denied agent runs no automatic recall at all, and still has `memory_semantic_search` on its tool list: if the model chooses to call it, it reads the store directly, filtered by neither chat nor session.
Automatic memory is what this document governs; closing the explicit tool path is a change to the #7808 gate's own tri-state, which is a separate decision about a separate surface.

`ManifestCapabilities` serializes an undeclared list by omitting the key, so a manifest that round-trips through the session store or the REST layer comes back undeclared rather than declared-empty.

One upgrade note follows from that.
A manifest blob written into SQLite *before* this shipped stored an undeclared list as `[]`, which now decodes as declared-empty.
Boot re-reads each agent's `agent.toml` and lets the disk copy win when it differs (`kernel/boot.rs`), so any agent whose manifest is on disk — which `persist_manifest_to_disk` gives every registered agent — is corrected on the first restart after the upgrade.
An agent whose `agent.toml` has since been deleted reads as declared-empty and loses automatic memory until the file is restored.
That direction is deliberate: the failure is loss of a convenience, logged at `debug` naming the capability, rather than a store quietly re-opening.

## 2. Which chat did it come from

The #5227 filter, keyed on the `chat_scope` metadata stamp: a memory extracted in a WhatsApp group does not surface in a DM with the same peer.
`MemoryLevel::User` rows are exempt — the chats it separates belong to the same person, so stable facts about them are meant to cross.
Composed by `compose_sender_scope(channel, chat_id)` at the kernel inject site, and `None` for every non-channel caller (dashboard, REST, CLI).
Both writers stamp it: `auto_memorize` on the facts it extracts, and `remember_interaction_best_effort` on the verbatim exchange it files every turn.

## 3. Which session did it come from

The #7605 filter, keyed on the `session_scope` metadata stamp.
`auto_memorize` and `remember_interaction_best_effort` both record the session that produced a memory; `auto_retrieve` and the substrate recall in `setup_recalled_memories` drop any memory stamped for a different one.
Stamping the per-turn writer matters more than stamping the extractor: raw dialogue is the dominant class in a mature store — 794 of 999 live rows on the installation measured in #7920 — and each row inlines a whole user turn plus the reply, so for as long as it went out unstamped the filter was separating the smaller and less revealing half of the store.
The dedup candidate set inside `add_with_decision` is filtered the same way, or a later turn in another session could NOOP against a stranger's row — losing the fact — or UPDATE it in place, overwriting one visitor's memory with another's content.

There is no `MemoryLevel::User` exemption here, unlike the chat filter.
Two sessions of a public agent are routinely two different people, and "user-level" is exactly where an extractor files the personal details that must not cross.

### Which session

Whatever session the turn is already reading and writing — `Session::id`, resolved by the ladder in [session-mode-resolution.md](session-mode-resolution.md).
The canonical session for a bare `librefang message`, the caller's `session_id` for a REST turn or `librefang message --session-id` (#7815), `SessionId::for_channel` for a channel message, the parent's session for a fork.
No second notion of a session is derived here.

### Defaults, and the one behaviour that changes

`[proactive_memory] session_scoped_recall` defaults to `true` in `config.toml`, and `[proactive_memory] session_scoped_recall` in `agent.toml` overrides it per agent (`ProactiveMemoryOverrides::resolve_session_scoped_recall`).
Per-agent overrides live in `agent.toml`, never in `config.toml` — see #5476.

For most deployments nothing changes.
An agent driven by a bare CLI or a REST call with no `session_id` runs every turn in one canonical session, so the stamp is constant.
A channel agent gets one session per chat, which is what the chat filter was already separating.

The case that does change is `session_mode = "new"`: each invocation gets a fresh session, so an agent that memorized something on one cron fire no longer auto-recalls it on the next.
That is the honest reading of "new" — an isolated turn — but an agent that was relying on the agent-wide pool for continuity across its runs wants `session_scoped_recall = false` in its `agent.toml`.

Memories written before this shipped carry no `session_scope` stamp and stay recallable from every session, so upgrading never blanks out an existing store.
The isolation applies from the first turn after the upgrade onwards; an operator who wants the old rows separated too has to clear the store.

### Where an operator sets it

`GET /api/memory/config` reports the live value under `proactive_memory.session_scoped_recall`, and `PATCH /api/memory/config` writes it into the `[proactive_memory]` table of `config.toml` and hot-reloads it like every other key on that endpoint.
The dashboard's memory settings drawer exposes it as a switch beside Auto Memorize and Auto Retrieve.
Both surfaces move the deployment-wide default only; the per-agent override stays in `agent.toml`, because `KernelConfig` has no `agents` table and an `[agents.<name>]` block in `config.toml` would parse and then never reach a manifest (#5476).
