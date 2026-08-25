Agent types are now editable from the dashboard: a new **Agent Types** page creates, edits and deletes the reusable agent manifests you spawn from, backed by `POST`/`PUT`/`DELETE` on `/api/templates`.
Saving is a patch, not a rewrite — the editor renders seven of a manifest's fifty-eight fields, so the server merges what you sent over the document already on disk and leaves the rest alone.
An operator's `[[triggers]]`, `[compaction]`, `max_history_messages`, `mcp_servers`, `tool_allowlist`, `session_mode`, `[workspaces]`, `channels`, `[exec_policy]` and `fallback_models` all survive an edit made through the form, and a blank system prompt stays blank instead of being replaced with canned text.
Skills and tools are picked from the installed catalogs rather than typed into a comma-separated box, and rows that come from a live agent's own workspace are marked *managed via Agents* instead of offering an Edit button the API would refuse.
The design and defect analysis are @DaBlitzStein's, from #6931, #7740 and #7731.
(#7859) (@houko)
