An agent-created skill's manifest now names the agent that produced it, instead of the literal `agent-evolved`.
`create_skill` was handed the author and passed it to the evolution history, then wrote a hardcoded string into `skill.toml` — so the provenance existed only in `.evolution.json`, which is not what the marketplace or the `librefang skill` surfaces read.
Every skill approved through the skill workshop was affected, because that path passes the candidate's real agent id.
(#7929) (@houko)
