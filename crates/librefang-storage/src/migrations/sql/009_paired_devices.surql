-- Phase 13 / migration v9: Device pairing persistence.
--
-- Mirrors the rusqlite `paired_devices` table.

DEFINE TABLE IF NOT EXISTS paired_devices SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS device_id    ON paired_devices TYPE string;
DEFINE FIELD IF NOT EXISTS display_name ON paired_devices TYPE string;
DEFINE FIELD IF NOT EXISTS platform     ON paired_devices TYPE string;
DEFINE FIELD IF NOT EXISTS paired_at    ON paired_devices TYPE string;
DEFINE FIELD IF NOT EXISTS last_seen    ON paired_devices TYPE string;
DEFINE FIELD IF NOT EXISTS push_token   ON paired_devices TYPE option<string>;

DEFINE INDEX IF NOT EXISTS paired_devices_device_id_idx
    ON paired_devices COLUMNS device_id UNIQUE;
