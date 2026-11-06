-- Migration v12: Add user_id and channel columns to audit_entries (RBAC M1 attribution).
--
-- Mirrors SQLite migration v22. Pre-existing rows will have NULL values for
-- both columns — the chain hash is preserved because user_id/channel were not
-- part of the original hash input (they are folded in only when present, per
-- the `compute_hash` logic in surreal_audit.rs).
--
-- SurrealDB `option<string>` maps to a nullable string — existing rows that
-- do not carry these fields are read back as `None` in Rust.

DEFINE FIELD IF NOT EXISTS user_id ON audit_entries TYPE option<string>;
DEFINE FIELD IF NOT EXISTS channel ON audit_entries TYPE option<string>;

DEFINE INDEX IF NOT EXISTS audit_user_idx
    ON audit_entries COLUMNS user_id;
