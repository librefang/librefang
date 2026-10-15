Agent manifest version history: every `agent.toml` write the daemon makes — dashboard and API edits, suspend/resume, provisioning and restores — is recorded in SQLite with its change source, so operators can see what changed, when, and from where.
History starts at an agent's first such change: spawn records no initial version, and hand edits picked up from disk are applied without a snapshot.
New endpoints `GET /api/agents/{id}/manifest-history` and `POST /api/agents/{id}/manifest-history/{version_id}/restore` list timestamped TOML snapshots newest-first and roll an agent back to a prior one.
The dashboard gains a History tab on the agent detail panel with expandable snapshots and a confirm-gated restore.
Retention is capped at 50 snapshots per agent and history rows are purged with the agent through the shared agent-scoped cascade (#8504) (@DaBlitzStein)
