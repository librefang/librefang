Agent manifest version history: config changes are now recorded in SQLite so operators can see what changed and when.
New endpoint `GET /api/agents/{id}/manifest-history` returns timestamped TOML snapshots.
Dashboard gains a "History" tab on the agent detail panel showing the version timeline as raw TOML per snapshot — viewing only, there is no diff and no restore of a prior version.
Skills already had version history via the evolution system; this closes the gap for agents. (#8041) (@DaBlitzStein)