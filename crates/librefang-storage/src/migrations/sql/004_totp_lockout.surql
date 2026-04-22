-- Phase 6 / migration v4: per-sender TOTP lockout window.
--
-- Mirrors the rusqlite `totp_lockout` table. `locked_at` is stored as a
-- Unix-seconds integer (NOT datetime) so we round-trip cleanly through
-- the JSON layer.

DEFINE TABLE IF NOT EXISTS totp_lockout SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS sender_id ON totp_lockout TYPE string;
DEFINE FIELD IF NOT EXISTS failures  ON totp_lockout TYPE int;
DEFINE FIELD IF NOT EXISTS locked_at ON totp_lockout TYPE option<int>;

DEFINE INDEX IF NOT EXISTS totp_lockout_sender_id_idx
    ON totp_lockout COLUMNS sender_id UNIQUE;
