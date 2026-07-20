//! Knowledge graph backed by SQLite.
//!
//! Stores entities and relations with support for graph pattern queries.

use chrono::Utc;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    Entity, EntityType, GraphMatch, GraphPattern, Relation, RelationType,
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::collections::HashMap;
use tracing::error;
use uuid::Uuid;

/// Knowledge graph store backed by SQLite.
#[derive(Clone)]
pub struct KnowledgeStore {
    pool: Pool<SqliteConnectionManager>,
}

impl KnowledgeStore {
    /// Create a new knowledge store wrapping the given connection.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Add an entity to the knowledge graph.
    ///
    /// `peer_id` scopes the entity to a single user on a multi-user agent
    /// (#6494); `None` writes a shared/unscoped entity, preserving the
    /// pre-migration behaviour. Because the entities table is keyed on the
    /// composite `(id, peer_id)` (v47), two users' same-named entities — which
    /// normalize to the same deterministic `id` — are distinct rows, so an
    /// upsert only ever merges into the calling peer's own row and never
    /// overwrites another user's entity.
    pub fn add_entity(
        &self,
        entity: Entity,
        agent_id: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<String> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let id = if entity.id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            entity.id.clone()
        };
        let entity_type_str =
            serde_json::to_string(&entity.entity_type).map_err(LibreFangError::serialization)?;
        let props_str =
            serde_json::to_string(&entity.properties).map_err(LibreFangError::serialization)?;
        let now = Utc::now().to_rfc3339();
        // Shared/unscoped is stored as the empty-string sentinel, never NULL,
        // so the composite (id, peer_id) key deduplicates shared entities.
        let peer = peer_id.unwrap_or("");
        conn.execute(
            "INSERT INTO entities (id, entity_type, name, properties, created_at, updated_at, agent_id, peer_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)
             ON CONFLICT(id, peer_id) DO UPDATE SET name = ?3, properties = ?4, updated_at = ?5",
            rusqlite::params![id, entity_type_str, entity.name, props_str, now, agent_id, peer],
        )
        .map_err(LibreFangError::memory)?;
        Ok(id)
    }

    /// Add a relation between two entities.
    ///
    /// `peer_id` scopes the relation to a single user on a multi-user agent
    /// (#6494); `None` writes a shared/unscoped relation. The relation is the
    /// load-bearing isolation predicate — [`query_graph_scoped`] filters on
    /// `r.peer_id` — so a per-user relation keeps one user's triples out of
    /// another's graph reads.
    pub fn add_relation(
        &self,
        relation: Relation,
        agent_id: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<String> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let id = Uuid::new_v4().to_string();
        let rel_type_str =
            serde_json::to_string(&relation.relation).map_err(LibreFangError::serialization)?;
        let props_str =
            serde_json::to_string(&relation.properties).map_err(LibreFangError::serialization)?;
        let now = Utc::now().to_rfc3339();
        // Shared/unscoped is the empty-string sentinel, never NULL (see add_entity).
        let peer = peer_id.unwrap_or("");
        conn.execute(
            "INSERT INTO relations (id, source_entity, relation_type, target_entity, properties, confidence, created_at, agent_id, peer_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                id,
                relation.source,
                rel_type_str,
                relation.target,
                props_str,
                relation.confidence as f64,
                now,
                agent_id,
                peer,
            ],
        )
        .map_err(LibreFangError::memory)?;
        Ok(id)
    }

    /// Delete an agent's relations, plus the entities it first wrote that no
    /// surviving relation still references.
    ///
    /// Wrapped in a single transaction so a relations-then-entities failure
    /// can't leave orphan entities (relations referencing entities silently
    /// broke ranking on the next graph query). See #3501.
    ///
    /// Relations are strictly per-agent, so this agent's relations are deleted
    /// wholesale. Entities are NOT per-agent: the table's key is `(id, peer_id)`
    /// (agent_id is only first-writer provenance), so an entity a since-deleted
    /// agent happened to write first can still be referenced — by id or name —
    /// by another agent's live relations. Deleting every `agent_id = A` entity
    /// would silently orphan those relations, quietly vanishing another agent's
    /// data from future reads (#6521). So only entities this agent wrote that
    /// NO surviving relation still references (by id or name, across any agent /
    /// peer) are removed; shared, still-referenced entities are kept in place.
    pub fn delete_by_agent(&self, agent_id: &str) -> LibreFangResult<u64> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let tx = conn
            .unchecked_transaction()
            .map_err(LibreFangError::memory)?;
        let rel_count = tx
            .execute(
                "DELETE FROM relations WHERE agent_id = ?1",
                rusqlite::params![agent_id],
            )
            .map_err(LibreFangError::memory)? as u64;
        // Runs AFTER the relations delete above, so this agent's own relations
        // no longer count as "referencing" — only OTHER agents' surviving
        // relations keep an entity alive. Conservative on `name` as well as
        // `id` because `query_graph_scoped`'s JOIN resolves a relation endpoint
        // by either.
        let ent_count = tx
            .execute(
                "DELETE FROM entities
                 WHERE agent_id = ?1
                   AND id NOT IN (SELECT source_entity FROM relations
                                  UNION SELECT target_entity FROM relations)
                   AND name NOT IN (SELECT source_entity FROM relations
                                    UNION SELECT target_entity FROM relations)",
                rusqlite::params![agent_id],
            )
            .map_err(LibreFangError::memory)? as u64;
        tx.commit().map_err(LibreFangError::memory)?;
        Ok(rel_count + ent_count)
    }

    /// Check if a relation already exists between two entities with a given type.
    ///
    /// `peer_id` scopes the dedup check to one user (#6494): without it, user
    /// B's identical triple would match user A's existing relation and be
    /// silently dropped as a duplicate. Shared/unscoped maps to the `''`
    /// sentinel, so a plain `=` comparison dedups shared relations correctly.
    pub fn has_relation(
        &self,
        source_id: &str,
        relation_type: &RelationType,
        target_id: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<bool> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;
        let rel_str =
            serde_json::to_string(relation_type).map_err(LibreFangError::serialization)?;
        let peer = peer_id.unwrap_or("");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM relations r
                 WHERE (r.source_entity = ?1 OR EXISTS (SELECT 1 FROM entities e WHERE e.id = ?1 AND e.name = r.source_entity))
                 AND r.relation_type = ?2
                 AND (r.target_entity = ?3 OR EXISTS (SELECT 1 FROM entities e WHERE e.id = ?3 AND e.name = r.target_entity))
                 AND r.peer_id = ?4",
                rusqlite::params![source_id, rel_str, target_id, peer],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(count > 0)
    }

    /// Query the knowledge graph with a pattern.
    pub fn query_graph(&self, pattern: GraphPattern) -> LibreFangResult<Vec<GraphMatch>> {
        self.query_graph_scoped(pattern, None, None)
    }

    /// Query the knowledge graph with a pattern, optionally scoped to a
    /// single agent's and/or single user's triples.
    ///
    /// When `agent_id` is `Some`, an `AND r.agent_id = ?` predicate is
    /// appended so the caller only sees that agent's relations.
    /// When `peer_id` is `Some`, an `AND r.peer_id = ?` predicate is appended
    /// so the caller only sees that user's relations (#6494). Because the
    /// entity JOINs tie `s`/`t` to the relation's `agent_id` **and** `peer_id`,
    /// scoping the relation side scopes all three tables — a matched entity can
    /// never come from a different user's row even though the deterministic
    /// `id` is shared across peers.
    /// This is the ACL boundary for the per-agent / per-user relations read
    /// endpoint: the write path keys every row on `agent_id` (+ `peer_id`), so
    /// an unscoped read leaked every agent's — and every user's — graph.
    /// A `None` peer_id is an unscoped read that returns all peers' rows
    /// (shared semantics, matching memories); it does not filter to NULL-only.
    pub fn query_graph_scoped(
        &self,
        pattern: GraphPattern,
        agent_id: Option<&str>,
        peer_id: Option<&str>,
    ) -> LibreFangResult<Vec<GraphMatch>> {
        let conn = self.pool.get().map_err(LibreFangError::memory)?;

        // The name-based JOIN arm ties a matched entity to the relation's
        // agent_id AND peer_id, so a name that collides across users (the
        // deterministic id is shared) still resolves to the entity owned by the
        // same user as the relation — never another user's same-named entity.
        let mut sql = String::from(
            "SELECT
                s.id, s.entity_type, s.name, s.properties, s.created_at, s.updated_at,
                r.id, r.source_entity, r.relation_type, r.target_entity, r.properties, r.confidence, r.created_at,
                t.id, t.entity_type, t.name, t.properties, t.created_at, t.updated_at
             FROM relations r
             JOIN entities s ON ((r.source_entity = s.id OR (r.source_entity = s.name AND s.agent_id = r.agent_id)) AND s.peer_id = r.peer_id)
             JOIN entities t ON ((r.target_entity = t.id OR (r.target_entity = t.name AND t.agent_id = r.agent_id)) AND t.peer_id = r.peer_id)
             WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1;

        if let Some(ref source) = pattern.source {
            sql.push_str(&format!(" AND (s.id = ?{} OR s.name = ?{})", idx, idx + 1));
            params.push(Box::new(source.clone()));
            params.push(Box::new(source.clone()));
            idx += 2;
        }
        if let Some(ref relation) = pattern.relation {
            let rel_str = serde_json::to_string(relation).map_err(LibreFangError::serialization)?;
            sql.push_str(&format!(" AND r.relation_type = ?{idx}"));
            params.push(Box::new(rel_str));
            idx += 1;
        }
        if let Some(ref target) = pattern.target {
            sql.push_str(&format!(" AND (t.id = ?{} OR t.name = ?{})", idx, idx + 1));
            params.push(Box::new(target.clone()));
            params.push(Box::new(target.clone()));
            idx += 2;
        }
        if let Some(agent_id) = agent_id {
            sql.push_str(&format!(" AND r.agent_id = ?{idx}"));
            params.push(Box::new(agent_id.to_string()));
            idx += 1;
        }
        if let Some(peer_id) = peer_id {
            sql.push_str(&format!(" AND r.peer_id = ?{idx}"));
            params.push(Box::new(peer_id.to_string()));
            idx += 1;
        }
        let _ = idx;

        sql.push_str(" LIMIT 100");

        let mut stmt = conn.prepare(&sql).map_err(LibreFangError::memory)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(RawGraphRow {
                    s_id: row.get(0)?,
                    s_type: row.get(1)?,
                    s_name: row.get(2)?,
                    s_props: row.get(3)?,
                    s_created: row.get(4)?,
                    s_updated: row.get(5)?,
                    r_id: row.get(6)?,
                    r_source: row.get(7)?,
                    r_type: row.get(8)?,
                    r_target: row.get(9)?,
                    r_props: row.get(10)?,
                    r_confidence: row.get(11)?,
                    r_created: row.get(12)?,
                    t_id: row.get(13)?,
                    t_type: row.get(14)?,
                    t_name: row.get(15)?,
                    t_props: row.get(16)?,
                    t_created: row.get(17)?,
                    t_updated: row.get(18)?,
                })
            })
            .map_err(LibreFangError::memory)?;

        let mut matches = Vec::new();
        for row_result in rows {
            let r = row_result.map_err(LibreFangError::memory)?;
            matches.push(GraphMatch {
                source: parse_entity(
                    &r.s_id,
                    &r.s_type,
                    &r.s_name,
                    &r.s_props,
                    &r.s_created,
                    &r.s_updated,
                )?,
                relation: parse_relation(
                    &r.r_source,
                    &r.r_type,
                    &r.r_target,
                    &r.r_props,
                    r.r_confidence,
                    &r.r_created,
                )?,
                target: parse_entity(
                    &r.t_id,
                    &r.t_type,
                    &r.t_name,
                    &r.t_props,
                    &r.t_created,
                    &r.t_updated,
                )?,
            });
        }
        Ok(matches)
    }
}

/// Raw row from a graph query.
struct RawGraphRow {
    s_id: String,
    s_type: String,
    s_name: String,
    s_props: String,
    s_created: String,
    s_updated: String,
    r_id: String,
    r_source: String,
    r_type: String,
    r_target: String,
    r_props: String,
    r_confidence: f64,
    r_created: String,
    t_id: String,
    t_type: String,
    t_name: String,
    t_props: String,
    t_created: String,
    t_updated: String,
}

// Suppress the unused field warning — r_id is part of the schema
impl RawGraphRow {
    #[allow(dead_code)]
    fn relation_id(&self) -> &str {
        &self.r_id
    }
}

fn parse_entity(
    id: &str,
    etype: &str,
    name: &str,
    props: &str,
    created: &str,
    updated: &str,
) -> LibreFangResult<Entity> {
    let entity_type: EntityType =
        serde_json::from_str(etype).unwrap_or(EntityType::Custom("unknown".to_string()));
    // Refuse to silently substitute `HashMap::default()` for a corrupt
    // `properties` blob — that disguises corruption as "this entity has
    // no properties", which the operator cannot tell apart from a row
    // that legitimately has none (audit: json-text-silent-parse-fallback).
    let properties: HashMap<String, serde_json::Value> = match serde_json::from_str(props) {
        Ok(m) => m,
        Err(e) => {
            error!(
                row_id = %id,
                table = "entities",
                column = "properties",
                error = %e,
                "corrupt JSON in TEXT column"
            );
            return Err(LibreFangError::serialization(e));
        }
    };
    let created_at = chrono::DateTime::parse_from_rfc3339(created)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = chrono::DateTime::parse_from_rfc3339(updated)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(Entity {
        id: id.to_string(),
        entity_type,
        name: name.to_string(),
        properties,
        created_at,
        updated_at,
    })
}

fn parse_relation(
    source: &str,
    rtype: &str,
    target: &str,
    props: &str,
    confidence: f64,
    created: &str,
) -> LibreFangResult<Relation> {
    let relation: RelationType = serde_json::from_str(rtype).unwrap_or(RelationType::RelatedTo);
    // Same rationale as `parse_entity`: a corrupt `properties` blob must
    // surface as an error, not as a silent empty map (audit:
    // json-text-silent-parse-fallback).
    let properties: HashMap<String, serde_json::Value> = match serde_json::from_str(props) {
        Ok(m) => m,
        Err(e) => {
            error!(
                source = %source,
                target = %target,
                table = "relations",
                column = "properties",
                error = %e,
                "corrupt JSON in TEXT column"
            );
            return Err(LibreFangError::serialization(e));
        }
    };
    let created_at = chrono::DateTime::parse_from_rfc3339(created)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(Relation {
        source: source.to_string(),
        relation,
        target: target.to_string(),
        properties,
        confidence: confidence as f32,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> KnowledgeStore {
        let manager = r2d2_sqlite::SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder().max_size(1).build(manager).unwrap();
        run_migrations(&pool.get().unwrap()).unwrap();
        KnowledgeStore::new(pool)
    }

    #[test]
    fn test_add_and_query_entity() {
        let store = setup();
        let id = store
            .add_entity(
                Entity {
                    id: String::new(),
                    entity_type: EntityType::Person,
                    name: "Alice".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn test_add_relation_and_query() {
        let store = setup();
        let alice_id = store
            .add_entity(
                Entity {
                    id: "alice".to_string(),
                    entity_type: EntityType::Person,
                    name: "Alice".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();
        let company_id = store
            .add_entity(
                Entity {
                    id: "acme".to_string(),
                    entity_type: EntityType::Organization,
                    name: "Acme Corp".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();
        store
            .add_relation(
                Relation {
                    source: alice_id.clone(),
                    relation: RelationType::WorksAt,
                    target: company_id,
                    properties: HashMap::new(),
                    confidence: 0.95,
                    created_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();

        let matches = store
            .query_graph(GraphPattern {
                source: Some(alice_id),
                relation: Some(RelationType::WorksAt),
                target: None,
                max_depth: 1,
            })
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].target.name, "Acme Corp");
    }

    /// Regression test for #1022: when relations reference entities by name
    /// (as the MCP tool does) instead of by ID, the JOIN must still match.
    #[test]
    fn test_query_graph_relation_references_by_name() {
        let store = setup();
        // Simulate MCP tool: entities get UUID ids, relations reference by name
        let _alice_id = store
            .add_entity(
                Entity {
                    id: String::new(), // will be assigned a UUID
                    entity_type: EntityType::Person,
                    name: "Alice".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "",
                None,
            )
            .unwrap();
        let _corp_id = store
            .add_entity(
                Entity {
                    id: String::new(),
                    entity_type: EntityType::Organization,
                    name: "Acme Corp".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "",
                None,
            )
            .unwrap();
        // Relation references entities by name (as MCP knowledge_add_relation does)
        store
            .add_relation(
                Relation {
                    source: "Alice".to_string(),
                    relation: RelationType::WorksAt,
                    target: "Acme Corp".to_string(),
                    properties: HashMap::new(),
                    confidence: 0.9,
                    created_at: Utc::now(),
                },
                "",
                None,
            )
            .unwrap();

        let matches = store
            .query_graph(GraphPattern {
                source: Some("Alice".to_string()),
                relation: None,
                target: None,
                max_depth: 1,
            })
            .unwrap();
        assert_eq!(
            matches.len(),
            1,
            "Should find match when relation references entity by name"
        );
        assert_eq!(matches[0].source.name, "Alice");
        assert_eq!(matches[0].target.name, "Acme Corp");
    }

    /// Regression for the audit item `json-text-silent-parse-fallback`.
    ///
    /// Pre-fix, `parse_entity` / `parse_relation` silently substituted
    /// `HashMap::default()` when the `properties` TEXT column failed to
    /// parse — so a corrupt row was indistinguishable from one that
    /// legitimately had no properties. After the fix, a corrupt
    /// `properties` blob causes `query_graph` to fail loudly with a
    /// `Serialization` error instead of returning a fabricated empty map.
    #[test]
    fn query_graph_surfaces_corrupt_entity_properties_instead_of_defaulting() {
        let store = setup();
        let alice_id = store
            .add_entity(
                Entity {
                    id: "alice".to_string(),
                    entity_type: EntityType::Person,
                    name: "Alice".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();
        let company_id = store
            .add_entity(
                Entity {
                    id: "acme".to_string(),
                    entity_type: EntityType::Organization,
                    name: "Acme Corp".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();
        store
            .add_relation(
                Relation {
                    source: alice_id.clone(),
                    relation: RelationType::WorksAt,
                    target: company_id,
                    properties: HashMap::new(),
                    confidence: 0.9,
                    created_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();

        // Corrupt Alice's `properties` blob directly — simulates a manual
        // SQL edit, upstream serde drift, or partial-write recovery.
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "UPDATE entities SET properties = ?1 WHERE id = ?2",
                rusqlite::params!["this is not json", &alice_id],
            )
            .unwrap();
        }

        let res = store.query_graph(GraphPattern {
            source: Some(alice_id),
            relation: Some(RelationType::WorksAt),
            target: None,
            max_depth: 1,
        });
        assert!(
            matches!(res, Err(LibreFangError::Serialization { .. })),
            "corrupt entity properties must surface as Serialization, not be silently defaulted; \
             got: {res:?}"
        );
    }

    /// Same audit item, but the corruption is on the relation row's
    /// `properties` column instead of the entity's.
    #[test]
    fn query_graph_surfaces_corrupt_relation_properties_instead_of_defaulting() {
        let store = setup();
        let alice_id = store
            .add_entity(
                Entity {
                    id: "alice".to_string(),
                    entity_type: EntityType::Person,
                    name: "Alice".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();
        let company_id = store
            .add_entity(
                Entity {
                    id: "acme".to_string(),
                    entity_type: EntityType::Organization,
                    name: "Acme Corp".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();
        let rel_id = store
            .add_relation(
                Relation {
                    source: alice_id.clone(),
                    relation: RelationType::WorksAt,
                    target: company_id,
                    properties: HashMap::new(),
                    confidence: 0.9,
                    created_at: Utc::now(),
                },
                "test-agent",
                None,
            )
            .unwrap();

        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "UPDATE relations SET properties = ?1 WHERE id = ?2",
                rusqlite::params!["{not-valid-json", &rel_id],
            )
            .unwrap();
        }

        let res = store.query_graph(GraphPattern {
            source: Some(alice_id),
            relation: Some(RelationType::WorksAt),
            target: None,
            max_depth: 1,
        });
        assert!(
            matches!(res, Err(LibreFangError::Serialization { .. })),
            "corrupt relation properties must surface as Serialization, not be silently defaulted; \
             got: {res:?}"
        );
    }

    /// Security regression: `query_graph_scoped` must confine results to a
    /// single agent so the per-agent relations HTTP endpoint cannot leak
    /// another agent's triples. The write path keys every entity/relation on
    /// `agent_id`, but the unscoped `query_graph` returned all agents' rows.
    #[test]
    fn query_graph_scoped_isolates_relations_by_agent() {
        let store = setup();

        // Agent A: a private triple (Alice works at Acme).
        store
            .add_entity(
                Entity {
                    id: "alice_a".to_string(),
                    entity_type: EntityType::Person,
                    name: "Alice".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "agent-a",
                None,
            )
            .unwrap();
        store
            .add_entity(
                Entity {
                    id: "acme_a".to_string(),
                    entity_type: EntityType::Organization,
                    name: "Acme Corp".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "agent-a",
                None,
            )
            .unwrap();
        store
            .add_relation(
                Relation {
                    source: "alice_a".to_string(),
                    relation: RelationType::WorksAt,
                    target: "acme_a".to_string(),
                    properties: HashMap::new(),
                    confidence: 0.95,
                    created_at: Utc::now(),
                },
                "agent-a",
                None,
            )
            .unwrap();

        // Agent B: a different private triple (Bob works at Globex).
        store
            .add_entity(
                Entity {
                    id: "bob_b".to_string(),
                    entity_type: EntityType::Person,
                    name: "Bob".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "agent-b",
                None,
            )
            .unwrap();
        store
            .add_entity(
                Entity {
                    id: "globex_b".to_string(),
                    entity_type: EntityType::Organization,
                    name: "Globex".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                "agent-b",
                None,
            )
            .unwrap();
        store
            .add_relation(
                Relation {
                    source: "bob_b".to_string(),
                    relation: RelationType::WorksAt,
                    target: "globex_b".to_string(),
                    properties: HashMap::new(),
                    confidence: 0.95,
                    created_at: Utc::now(),
                },
                "agent-b",
                None,
            )
            .unwrap();

        let pattern = || GraphPattern {
            source: None,
            relation: None,
            target: None,
            max_depth: 1,
        };

        // Scoped to B: only Bob→Globex, never Alice→Acme.
        let b_matches = store
            .query_graph_scoped(pattern(), Some("agent-b"), None)
            .unwrap();
        assert_eq!(
            b_matches.len(),
            1,
            "agent B must see exactly its own triple"
        );
        assert_eq!(b_matches[0].source.name, "Bob");
        assert_eq!(b_matches[0].target.name, "Globex");
        assert!(
            !b_matches.iter().any(|m| m.source.name == "Alice"),
            "agent B must never receive agent A's relations"
        );

        // Scoped to A: only Alice→Acme.
        let a_matches = store
            .query_graph_scoped(pattern(), Some("agent-a"), None)
            .unwrap();
        assert_eq!(a_matches.len(), 1);
        assert_eq!(a_matches[0].source.name, "Alice");

        // Unscoped still returns both — proves the scoping predicate, not a
        // storage artifact, is what isolates the two agents.
        let all = store.query_graph(pattern()).unwrap();
        assert_eq!(all.len(), 2, "unscoped query returns every agent's triples");
    }

    /// #6494: on a single multi-user agent, one user's triples must not appear
    /// in another user's peer-scoped read — and a name that collides across
    /// users (same deterministic entity id) must still resolve to the querying
    /// user's own entity, never the other user's.
    #[test]
    fn query_graph_scoped_isolates_relations_by_peer() {
        let store = setup();
        let agent = "shared-agent";

        // Both users mention a person named "Manager" (same normalized id
        // "manager"), but each with their own employer. Under the composite
        // (id, peer_id) key these coexist as two distinct entity rows.
        let mk_person = || Entity {
            id: "manager".to_string(),
            entity_type: EntityType::Person,
            name: "Manager".to_string(),
            properties: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        // User A: Manager works at Acme.
        store
            .add_entity(mk_person(), agent, Some("user-A"))
            .unwrap();
        store
            .add_entity(
                Entity {
                    id: "acme".to_string(),
                    entity_type: EntityType::Organization,
                    name: "Acme".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                agent,
                Some("user-A"),
            )
            .unwrap();
        store
            .add_relation(
                Relation {
                    source: "manager".to_string(),
                    relation: RelationType::WorksAt,
                    target: "acme".to_string(),
                    properties: HashMap::new(),
                    confidence: 0.9,
                    created_at: Utc::now(),
                },
                agent,
                Some("user-A"),
            )
            .unwrap();

        // User B: Manager works at Globex.
        store
            .add_entity(mk_person(), agent, Some("user-B"))
            .unwrap();
        store
            .add_entity(
                Entity {
                    id: "globex".to_string(),
                    entity_type: EntityType::Organization,
                    name: "Globex".to_string(),
                    properties: HashMap::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                agent,
                Some("user-B"),
            )
            .unwrap();
        store
            .add_relation(
                Relation {
                    source: "manager".to_string(),
                    relation: RelationType::WorksAt,
                    target: "globex".to_string(),
                    properties: HashMap::new(),
                    confidence: 0.9,
                    created_at: Utc::now(),
                },
                agent,
                Some("user-B"),
            )
            .unwrap();

        let pattern = || GraphPattern {
            source: None,
            relation: None,
            target: None,
            max_depth: 1,
        };

        // User B's scoped read: exactly Manager→Globex, never Manager→Acme,
        // even though both share the "manager" source id.
        let b = store
            .query_graph_scoped(pattern(), Some(agent), Some("user-B"))
            .unwrap();
        assert_eq!(b.len(), 1, "user B sees only their own triple");
        assert_eq!(b[0].target.name, "Globex");
        assert!(
            !b.iter().any(|m| m.target.name == "Acme"),
            "user B must never receive user A's relation (#6494)"
        );

        // User A's scoped read: exactly Manager→Acme.
        let a = store
            .query_graph_scoped(pattern(), Some(agent), Some("user-A"))
            .unwrap();
        assert_eq!(a.len(), 1, "user A sees only their own triple");
        assert_eq!(a[0].target.name, "Acme");

        // Unscoped agent read returns both users' triples (shared semantics),
        // proving the peer predicate — not storage — is what isolates them.
        let both = store
            .query_graph_scoped(pattern(), Some(agent), None)
            .unwrap();
        assert_eq!(
            both.len(),
            2,
            "an unscoped read returns every peer's triples"
        );
    }

    /// Regression (#6521): `delete_by_agent` must not remove a shared entity
    /// that a surviving agent's relation still references, and must still clean
    /// up genuinely-orphaned entities the deleted agent wrote.
    #[test]
    fn delete_by_agent_keeps_shared_entities_but_prunes_orphans() {
        let store = setup();
        let ent = |id: &str, name: &str, t: EntityType| Entity {
            id: id.to_string(),
            entity_type: t,
            name: name.to_string(),
            properties: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        // Agent A first-writes two entities and a relation between them.
        store
            .add_entity(
                ent("acme", "Acme", EntityType::Organization),
                "agent-a",
                None,
            )
            .unwrap();
        store
            .add_entity(ent("alice", "Alice", EntityType::Person), "agent-a", None)
            .unwrap();
        store
            .add_relation(
                Relation {
                    source: "alice".to_string(),
                    relation: RelationType::WorksAt,
                    target: "acme".to_string(),
                    properties: HashMap::new(),
                    confidence: 1.0,
                    created_at: Utc::now(),
                },
                "agent-a",
                None,
            )
            .unwrap();
        // Agent B references the SHARED "acme" entity via its own relation
        // (bob -> acme). B's "bob" upserts under agent B; "acme" stays agent A.
        store
            .add_entity(ent("bob", "Bob", EntityType::Person), "agent-b", None)
            .unwrap();
        store
            .add_relation(
                Relation {
                    source: "bob".to_string(),
                    relation: RelationType::WorksAt,
                    target: "acme".to_string(),
                    properties: HashMap::new(),
                    confidence: 1.0,
                    created_at: Utc::now(),
                },
                "agent-b",
                None,
            )
            .unwrap();

        // Delete agent A. Its relation goes; "acme" survives (B still references
        // it); "alice" is pruned (only A's now-deleted relation referenced it).
        store.delete_by_agent("agent-a").unwrap();

        // Agent B's relation still resolves — its target entity is intact.
        let b = store
            .query_graph_scoped(
                GraphPattern {
                    source: Some("bob".to_string()),
                    relation: Some(RelationType::WorksAt),
                    target: None,
                    max_depth: 1,
                },
                Some("agent-b"),
                None,
            )
            .unwrap();
        assert_eq!(b.len(), 1, "agent B's relation must not be orphaned");
        assert_eq!(b[0].target.name, "Acme");

        // The genuinely-orphaned "alice" entity is gone; the shared "acme" stays.
        let conn = store.pool.get().unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM entities ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(names, vec!["Acme".to_string(), "Bob".to_string()]);
    }
}
