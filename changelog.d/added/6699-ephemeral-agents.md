Add `agent_spawn` ephemeral mode for Claude Code-style disposable sub-agents — spawn a temporary worker that runs a single task and returns the result directly, with no workspace, no DB persistence, and no registry entry. (@DaBlitzStein)

New API route: `POST /api/agents/spawn-ephemeral`. (@DaBlitzStein)

Agent template CRUD and dashboard Agent Types page are in a separate PR (#6931). (@DaBlitzStein)
