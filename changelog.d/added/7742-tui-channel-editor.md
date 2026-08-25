The TUI can assign channels to a running agent, from the agent detail screen with `n`.
`PUT /api/agents/{id}/channels` had shipped with no client anywhere — not in the dashboard, not in the CLI — so restricting which channels an existing agent answers on meant hand-editing `agent.toml` over SSH.
The same screen's skills and MCP rows now show the agent's real allowlists instead of struct defaults, which they had been rendering since the detail pane was added.
(#7879) (@houko)
