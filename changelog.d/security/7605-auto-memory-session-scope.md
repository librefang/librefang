Automatic memory no longer crosses session boundaries, and it now honours the memory capabilities an agent declares.
A public agent serves every visitor from one per-agent memory store, so a fact auto-memorized while serving one visitor was auto-retrieved into the next visitor's turn even when the two turns were addressed to different `session_id`s and their message histories never touched — the reporter was calling `DELETE /api/memory/agents/{id}` before every turn to work around it.
`auto_memorize` now stamps each memory with the session that produced it and `auto_retrieve` refuses to surface a memory stamped for a different session, so the two conversations stay apart without giving up memory altogether.
Rows written before this shipped carry no stamp and stay recallable everywhere, so upgrading does not blank out an existing store.
Set `[proactive_memory] session_scoped_recall = false` in `config.toml`, or in one agent's `agent.toml`, to go back to a single agent-wide pool — worth doing for a single-user agent whose `session_mode = "new"` runs are meant to build on each other.
Separately, `capabilities.memory_read = []` and `memory_write = []` in `agent.toml` now actually stop automatic recall and automatic capture; previously the agent kept receiving a populated `memories_used` on every turn and kept growing its store.
An absent `memory_read` key still means "unrestricted", as every other capability list does — only a list the manifest actually wrote is enforced.
(#7849) (@houko)
