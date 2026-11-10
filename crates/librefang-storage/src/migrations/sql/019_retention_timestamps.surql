-- Migration v19: retention timestamps for soft-deleted memories and finished tasks (mirrors SQLite V29, upstream #3466/#3467).
-- Adds deleted_at to memories and finished_at to task_queue for periodic prune sweeps.
-- SurrealDB uses FLEXIBLE fields for schema-compatible ALTER TABLE equivalent.

DEFINE FIELD IF NOT EXISTS deleted_at  ON memories   TYPE option<int>;
DEFINE FIELD IF NOT EXISTS finished_at ON task_queue TYPE option<int>;

DEFINE INDEX IF NOT EXISTS idx_memories_deleted_at   ON memories   FIELDS deleted, deleted_at;
DEFINE INDEX IF NOT EXISTS idx_task_queue_finished_at ON task_queue FIELDS status, finished_at;
