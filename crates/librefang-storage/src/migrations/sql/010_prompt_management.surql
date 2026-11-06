-- Phase 14 / migration v10: Prompt versioning and A/B experiment management.
--
-- Mirrors four rusqlite tables: prompt_versions, prompt_experiments,
-- experiment_variants, experiment_metrics.

-- Prompt versions — immutable snapshots of an agent's system prompt + tools.
DEFINE TABLE IF NOT EXISTS prompt_versions SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS agent_id      ON prompt_versions TYPE string;
DEFINE FIELD IF NOT EXISTS version       ON prompt_versions TYPE int;
DEFINE FIELD IF NOT EXISTS content_hash  ON prompt_versions TYPE string;
DEFINE FIELD IF NOT EXISTS system_prompt ON prompt_versions TYPE string;
DEFINE FIELD IF NOT EXISTS tools         ON prompt_versions TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS variables     ON prompt_versions TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS created_at    ON prompt_versions TYPE string;
DEFINE FIELD IF NOT EXISTS created_by    ON prompt_versions TYPE string;
DEFINE FIELD IF NOT EXISTS is_active     ON prompt_versions TYPE bool;
DEFINE FIELD IF NOT EXISTS description   ON prompt_versions TYPE option<string>;

DEFINE INDEX IF NOT EXISTS pv_agent_idx        ON prompt_versions COLUMNS agent_id;
DEFINE INDEX IF NOT EXISTS pv_agent_version_idx ON prompt_versions COLUMNS agent_id, version UNIQUE;
DEFINE INDEX IF NOT EXISTS pv_agent_active_idx ON prompt_versions COLUMNS agent_id, is_active;

-- Prompt experiments — A/B test definitions.
DEFINE TABLE IF NOT EXISTS prompt_experiments SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS name              ON prompt_experiments TYPE string;
DEFINE FIELD IF NOT EXISTS agent_id          ON prompt_experiments TYPE string;
DEFINE FIELD IF NOT EXISTS status            ON prompt_experiments TYPE string;
DEFINE FIELD IF NOT EXISTS traffic_split     ON prompt_experiments TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS success_criteria  ON prompt_experiments TYPE option<object> FLEXIBLE;
DEFINE FIELD IF NOT EXISTS started_at        ON prompt_experiments TYPE option<string>;
DEFINE FIELD IF NOT EXISTS ended_at          ON prompt_experiments TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at        ON prompt_experiments TYPE string;

DEFINE INDEX IF NOT EXISTS pe_agent_idx  ON prompt_experiments COLUMNS agent_id;
DEFINE INDEX IF NOT EXISTS pe_status_idx ON prompt_experiments COLUMNS status;

-- Experiment variants — one row per variant within an experiment.
DEFINE TABLE IF NOT EXISTS experiment_variants SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS experiment_id     ON experiment_variants TYPE string;
DEFINE FIELD IF NOT EXISTS name              ON experiment_variants TYPE string;
DEFINE FIELD IF NOT EXISTS prompt_version_id ON experiment_variants TYPE string;
DEFINE FIELD IF NOT EXISTS description       ON experiment_variants TYPE option<string>;

DEFINE INDEX IF NOT EXISTS ev_experiment_idx ON experiment_variants COLUMNS experiment_id;

-- Experiment metrics — aggregated per-variant metrics.
DEFINE TABLE IF NOT EXISTS experiment_metrics SCHEMAFULL;

DEFINE FIELD IF NOT EXISTS experiment_id       ON experiment_metrics TYPE string;
DEFINE FIELD IF NOT EXISTS variant_id          ON experiment_metrics TYPE string;
DEFINE FIELD IF NOT EXISTS total_requests      ON experiment_metrics TYPE int;
DEFINE FIELD IF NOT EXISTS successful_requests ON experiment_metrics TYPE int;
DEFINE FIELD IF NOT EXISTS failed_requests     ON experiment_metrics TYPE int;
DEFINE FIELD IF NOT EXISTS total_latency_ms    ON experiment_metrics TYPE int;
DEFINE FIELD IF NOT EXISTS total_cost_usd      ON experiment_metrics TYPE float;
DEFINE FIELD IF NOT EXISTS last_updated        ON experiment_metrics TYPE string;

DEFINE INDEX IF NOT EXISTS em_variant_idx ON experiment_metrics COLUMNS variant_id;
