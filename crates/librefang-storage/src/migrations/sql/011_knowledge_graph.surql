-- Knowledge graph tables for librefang agents.
--
-- Uses `kg_entities` and `kg_relations` as table names to avoid collision
-- with the `entities` / `relations` tables managed by the surreal-memory
-- library.  Schema mirrors the SQLite KnowledgeStore (knowledge.rs) so that
-- the sqlite_to_surreal migration tool can copy data without transformation.

-- ── kg_entities ──────────────────────────────────────────────────────────────

DEFINE TABLE kg_entities SCHEMAFULL;

DEFINE FIELD entity_type ON kg_entities TYPE string;
DEFINE FIELD name        ON kg_entities TYPE string;
DEFINE FIELD properties  ON kg_entities TYPE option<object> FLEXIBLE;
DEFINE FIELD created_at  ON kg_entities TYPE string;
DEFINE FIELD updated_at  ON kg_entities TYPE string;
DEFINE FIELD agent_id    ON kg_entities TYPE string;

DEFINE INDEX kg_entities_agent_id ON kg_entities FIELDS agent_id;
DEFINE INDEX kg_entities_name     ON kg_entities FIELDS name;

-- ── kg_relations ─────────────────────────────────────────────────────────────

DEFINE TABLE kg_relations SCHEMAFULL;

DEFINE FIELD source_entity  ON kg_relations TYPE string;
DEFINE FIELD relation_type  ON kg_relations TYPE string;
DEFINE FIELD target_entity  ON kg_relations TYPE string;
DEFINE FIELD properties     ON kg_relations TYPE option<object> FLEXIBLE;
DEFINE FIELD confidence     ON kg_relations TYPE float;
DEFINE FIELD created_at     ON kg_relations TYPE string;
DEFINE FIELD agent_id       ON kg_relations TYPE string;

DEFINE INDEX kg_relations_source   ON kg_relations FIELDS source_entity;
DEFINE INDEX kg_relations_target   ON kg_relations FIELDS target_entity;
DEFINE INDEX kg_relations_agent_id ON kg_relations FIELDS agent_id;
