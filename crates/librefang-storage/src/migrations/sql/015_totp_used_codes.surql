-- Migration v15: TOTP replay prevention table (mirrors SQLite V25, upstream #3952).
-- Stores SHA-256 hashes of recently-used codes to block reuse within the
-- same 30-second window. Entries older than 120s are pruned on verification.

DEFINE TABLE IF NOT EXISTS totp_used_codes SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS code_hash ON totp_used_codes TYPE string;
DEFINE FIELD IF NOT EXISTS used_at   ON totp_used_codes TYPE int;

DEFINE INDEX IF NOT EXISTS idx_totp_used_codes_hash    ON totp_used_codes FIELDS code_hash UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_totp_used_codes_used_at ON totp_used_codes FIELDS used_at;
