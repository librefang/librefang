`GET /api/memory/config` now reports `proactive_memory.session_scoped_recall` and `PATCH` can change it, with a matching switch in the dashboard's memory settings drawer.
The setting decides whether a memory an agent auto-memorized is recallable only from the conversation that produced it, so on a shared or public agent it is what stands between one visitor's turn and the next visitor's context — and until now the only way to read or move it was to open `config.toml` on the daemon host, which is precisely what an operator driving LibreFang through its API cannot do.
The per-agent override stays in `agent.toml`, where every other per-agent key lives.
(#7870) (@houko)
