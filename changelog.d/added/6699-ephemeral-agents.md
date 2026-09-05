Add `agent_spawn` ephemeral mode for Claude Code-style disposable sub-agents — spawn a temporary worker that runs a single task and returns the result directly, with no workspace, no DB persistence, and no registry entry.
A worker's advertised tools are derived from the parent's own set, so a restricted parent cannot launder a privilege escalation through one.
`POST /api/agents/spawn-ephemeral` is the HTTP entry point, and the dashboard's Quick Run drives it (#7875, #7903) (@DaBlitzStein)
