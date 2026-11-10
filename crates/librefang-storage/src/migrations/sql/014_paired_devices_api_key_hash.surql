-- Migration v14: add api_key_hash column to paired_devices for per-device
-- bearer tokens (mirrors SQLite V24, upstream PR #3344 / feat(mobile)).
-- The hash is SHA-256 prefixed "$sha256$"; empty string means no token minted yet.

DEFINE FIELD IF NOT EXISTS api_key_hash ON paired_devices TYPE string
    VALUE $value OR '';
