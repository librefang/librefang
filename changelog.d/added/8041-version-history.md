Agent manifest version history: config changes are now recorded in SQLite so operators can see what changed and when.
New endpoint `GET /api/agents/{id}/manifest-history` returns timestamped TOML snapshots, scoped like every other agent-scoped read — a snapshot is the agent's whole `agent.toml`, so a role that cannot inspect an agent cannot read its history either.
Recording covers the writers that patch `agent.toml` in place as well as full manifest persists, so a suspend, a resume, or an MCP allowlist change no longer leaves the newest recorded snapshot disagreeing with what is on disk.
Dashboard gains a "History" tab on the agent detail panel showing the version timeline as raw TOML per snapshot — viewing only, there is no diff and no restore of a prior version.
Skills already had version history via the evolution system; this closes the gap for agents. (#8041) (@DaBlitzStein)
