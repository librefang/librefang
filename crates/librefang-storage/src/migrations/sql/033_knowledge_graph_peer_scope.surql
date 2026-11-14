-- Migration v33: Scope knowledge-graph rows to a peer.
-- Mirrors upstream SQLite memory migration v47 (#6494).
--
-- The empty string is the shared/unscoped sentinel, matching the SQLite
-- migration. VALUE backfills existing rows while keeping future writes from
-- producing NONE in these SCHEMAFULL tables.

DEFINE FIELD IF NOT EXISTS peer_id ON kg_entities
    TYPE string
    VALUE $value OR '';

-- Logical entity IDs are separate from SurrealDB record IDs so two peers can
-- own the same normalized entity ID without overwriting one another. Existing
-- rows keep the empty sentinel and remain readable through the record-ID
-- fallback in SurrealKnowledgeBackend.
DEFINE FIELD IF NOT EXISTS entity_id ON kg_entities
    TYPE string
    VALUE $value OR '';

DEFINE FIELD IF NOT EXISTS peer_id ON kg_relations
    TYPE string
    VALUE $value OR '';

DEFINE INDEX IF NOT EXISTS kg_entities_agent_peer
    ON kg_entities COLUMNS agent_id, peer_id;

DEFINE INDEX IF NOT EXISTS kg_relations_agent_peer
    ON kg_relations COLUMNS agent_id, peer_id;
