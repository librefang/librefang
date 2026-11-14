-- Migration v38: webauthn_credentials table for passkey (WebAuthn/FIDO2) login.
-- Mirrors upstream SQLite v44 (migrate_v44, upstream #5981).
--
-- Upstream DDL:
--   CREATE TABLE IF NOT EXISTS webauthn_credentials (
--       credential_id TEXT PRIMARY KEY,
--       user_name     TEXT NOT NULL,
--       cred          TEXT NOT NULL,
--       label         TEXT,
--       created_at    INTEGER NOT NULL,
--       last_used_at  INTEGER
--   );
--   CREATE INDEX IF NOT EXISTS idx_webauthn_credentials_user_name
--       ON webauthn_credentials(user_name);
--
-- `cred` holds the serialized webauthn-rs `Passkey`, which is credential
-- material: the table is SCHEMAFULL so an undeclared field cannot be smuggled
-- alongside it. Timestamps are `int` (unix seconds) because upstream stores
-- them as INTEGER here, unlike the RFC3339 text used by goal_runs — mirroring
-- the column type keeps a row round-tripping through either backend.

DEFINE TABLE IF NOT EXISTS webauthn_credentials SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS credential_id ON webauthn_credentials TYPE string;
DEFINE FIELD IF NOT EXISTS user_name     ON webauthn_credentials TYPE string;
DEFINE FIELD IF NOT EXISTS cred          ON webauthn_credentials TYPE string;
DEFINE FIELD IF NOT EXISTS label         ON webauthn_credentials TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at    ON webauthn_credentials TYPE int;
DEFINE FIELD IF NOT EXISTS last_used_at  ON webauthn_credentials TYPE option<int>;

DEFINE INDEX IF NOT EXISTS idx_webauthn_credentials_credential_id
    ON webauthn_credentials FIELDS credential_id UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_webauthn_credentials_user_name
    ON webauthn_credentials FIELDS user_name;
