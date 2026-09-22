An install that grew by spawning agents rather than authoring templates could not go the other way: there was no way to take an agent whose configuration already worked and make it the starting point for the next one.
`POST /api/agents/{id}/save-as-agent-type` now snapshots a live agent's manifest into a reusable agent type, leaving the source agent untouched.
Spawning from the result works too — `POST /api/agents {"template": name}` previously only ever looked in `workspaces/agents/`, so it answered 404 for every real agent type. (#8360) (@DaBlitzStein)
