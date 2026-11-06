-- Phase 10 / migration v7: per-agent task queue.
--
-- Mirrors the rusqlite `task_queue` table.  `payload` is FLEXIBLE to allow
-- arbitrary task data without requiring schema changes for new task types.
-- Status values: 'pending' | 'running' | 'completed' | 'failed' | 'stuck'.

DEFINE TABLE IF NOT EXISTS task_queue SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS agent_id      ON task_queue TYPE string;
DEFINE FIELD IF NOT EXISTS task_type     ON task_queue TYPE string;
DEFINE FIELD IF NOT EXISTS payload       ON task_queue TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS status        ON task_queue TYPE string;
DEFINE FIELD IF NOT EXISTS priority      ON task_queue TYPE int;
DEFINE FIELD IF NOT EXISTS scheduled_at  ON task_queue TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at    ON task_queue TYPE string;
DEFINE FIELD IF NOT EXISTS completed_at  ON task_queue TYPE option<string>;
DEFINE FIELD IF NOT EXISTS delegated_by  ON task_queue TYPE option<string>;
DEFINE FIELD IF NOT EXISTS assigned_to   ON task_queue TYPE option<string>;

DEFINE INDEX IF NOT EXISTS task_queue_status_priority_idx
    ON task_queue COLUMNS status, priority;
DEFINE INDEX IF NOT EXISTS task_queue_agent_idx
    ON task_queue COLUMNS agent_id;
