A `[[triggers]]` entry with no `pattern` no longer freezes an agent's entire `agent.toml`.
The missing key deserialized to a JSON null, TOML cannot represent null, and the kernel serializes the whole manifest in one call — so one malformed trigger made every other field unpersistable while the API still answered `200 OK`, and the next restart quietly restored the stale file over every edit made since.
`PUT /api/agents/{id}/channels` also stops treating a body it cannot parse as "clear the allowlist": the shape its own API docs advertised silently reopened the agent to every configured channel, and is now refused.
(#7879) (@houko)
