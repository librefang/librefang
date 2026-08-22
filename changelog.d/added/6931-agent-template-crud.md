Add write operations (POST/PUT/DELETE) to the /api/templates surface, allowing dashboard users to create, update, and delete named agent templates stored under `~/.librefang/templates/`. (@DaBlitzStein)

The existing GET /api/templates listing now returns entries from both `~/.librefang/workspaces/agents/` (source = "agent") and `~/.librefang/templates/` (source = "template"). (@DaBlitzStein)

New dashboard page: Agent Types (card grid with create/edit/delete plus Quick Run for ephemeral spawn). (@DaBlitzStein)

Installed marketplace skills now show a link to the original marketplace (ClawHub, Skillhub, FangHub) so users can see comments, ratings, and full details at the source. (@DaBlitzStein)
