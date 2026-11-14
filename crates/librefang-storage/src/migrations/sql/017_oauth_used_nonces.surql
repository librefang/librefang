-- Migration v17: oauth_used_nonces table for OIDC nonce single-use enforcement (mirrors SQLite V27, upstream #3944/#4049).
-- SHA-256 hashes of recently-redeemed OIDC state nonces; entries older than 1 hour are pruned.

DEFINE TABLE IF NOT EXISTS oauth_used_nonces SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS nonce_hash ON oauth_used_nonces TYPE string;
DEFINE FIELD IF NOT EXISTS used_at    ON oauth_used_nonces TYPE int;

DEFINE INDEX IF NOT EXISTS idx_oauth_used_nonces_hash    ON oauth_used_nonces FIELDS nonce_hash UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_oauth_used_nonces_used_at ON oauth_used_nonces FIELDS used_at;
