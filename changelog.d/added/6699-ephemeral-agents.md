Add `agent_spawn` ephemeral mode for Claude Code-style disposable sub-agents — spawn a temporary worker that runs a single task and returns the result directly, with no workspace, no DB persistence, and no registry entry. (@DaBlitzStein)

New API routes: `POST /api/agents/spawn-ephemeral`, plus agent-types CRUD at `GET|POST|PUT|DELETE /api/agent-types` (manage named templates in `~/.librefang/templates/`). (@DaBlitzStein)

New dashboard page: Agent Types (card grid with create/edit/delete + Quick Run). (@DaBlitzStein)
