-- Phase 6 / migration v1: append-only audit chain.
--
-- Each row is one [`librefang_runtime::audit::AuditEntry`] linked to its
-- predecessor by `prev_hash` so the chain is verifiable end-to-end.

DEFINE TABLE IF NOT EXISTS audit_entries SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS seq        ON audit_entries TYPE int;
-- Stored as ISO-8601 string (NOT datetime) — SurrealDB 3.0 coerces
-- datetime fields and refuses to round-trip them as JSON.
DEFINE FIELD IF NOT EXISTS timestamp  ON audit_entries TYPE string;
DEFINE FIELD IF NOT EXISTS agent_id   ON audit_entries TYPE string;
DEFINE FIELD IF NOT EXISTS action     ON audit_entries TYPE string;
DEFINE FIELD IF NOT EXISTS detail     ON audit_entries TYPE string;
DEFINE FIELD IF NOT EXISTS outcome    ON audit_entries TYPE string;
DEFINE FIELD IF NOT EXISTS prev_hash  ON audit_entries TYPE string;
DEFINE FIELD IF NOT EXISTS hash       ON audit_entries TYPE string;

DEFINE INDEX IF NOT EXISTS audit_entries_seq_idx
    ON audit_entries COLUMNS seq UNIQUE;
DEFINE INDEX IF NOT EXISTS audit_entries_agent_idx
    ON audit_entries COLUMNS agent_id;
