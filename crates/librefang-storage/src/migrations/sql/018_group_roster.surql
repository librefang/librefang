-- Migration v18: group_roster table for cross-channel group membership tracking (mirrors SQLite V28, upstream #4079).
-- Tracks which users have been seen in each group chat; persists across daemon restarts.

DEFINE TABLE IF NOT EXISTS group_roster SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS channel_type ON group_roster TYPE string;
DEFINE FIELD IF NOT EXISTS chat_id      ON group_roster TYPE string;
DEFINE FIELD IF NOT EXISTS user_id      ON group_roster TYPE string;
DEFINE FIELD IF NOT EXISTS display_name ON group_roster TYPE string;
DEFINE FIELD IF NOT EXISTS username     ON group_roster TYPE option<string>;
DEFINE FIELD IF NOT EXISTS first_seen   ON group_roster TYPE int VALUE $value OR time::unix();
DEFINE FIELD IF NOT EXISTS last_seen    ON group_roster TYPE int VALUE $value OR time::unix();

DEFINE INDEX IF NOT EXISTS idx_group_roster_pk ON group_roster FIELDS channel_type, chat_id, user_id UNIQUE;
