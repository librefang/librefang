A template that names a skill nobody installed, or an MCP server that never connected, used to behave exactly like a template that named nothing: the tools were simply absent and the operator had nothing to read.
Spawned agents now surface those declarations — a WARN at spawn, `pending_skills` / `pending_mcp_servers` on the agents API, a `pending` field on the per-agent skills and MCP routes, and badges on the dashboard.
The MCP half is derived from the live connection pool rather than the configured server list, so a server that is configured here and unreachable is reported rather than hidden behind its own config entry.
Nothing is dropped and nothing needs a re-spawn: installing the skill and reloading the registry, or connecting the server, clears the entry on the next read.
Analysis and the original approach are @DaBlitzStein's in #7716. (#7853) (@houko)
