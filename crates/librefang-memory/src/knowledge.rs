//! Knowledge graph backed by SQLite.
//!
//! Stores entities and relations with support for graph pattern queries.

use chrono::Utc;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    Entity, EntityType, GraphMatch, GraphPattern, Relation, RelationType,
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Knowledge graph store backed by SQLite.
#[derive(Clone)]
pub struct KnowledgeStore {
    conn: Arc<Mutex<Connection>>,
}

impl KnowledgeStore {
    /// Create a new knowledge store wrapping the given connection.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Add an entity to the knowledge graph.
    pub fn add_entity(&self, entity: Entity, agent_id: &str) -> LibreFangResult<String> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
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
        conn.execute(
            "INSERT INTO entities (id, entity_type, name, properties, created_at, updated_at, agent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name = ?3, properties = ?4, updated_at = ?5",
            rusqlite::params![id, entity_type_str, entity.name, props_str, now, agent_id],
        )
        .map_err(LibreFangError::memory)?;
        Ok(id)
    }

    /// Add a relation between two entities.
    pub fn add_relation(&self, relation: Relation, agent_id: &str) -> LibreFangResult<String> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let id = Uuid::new_v4().to_string();
        let rel_type_str =
            serde_json::to_string(&relation.relation).map_err(LibreFangError::serialization)?;
        let props_str =
            serde_json::to_string(&relation.properties).map_err(LibreFangError::serialization)?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO relations (id, source_entity, relation_type, target_entity, properties, confidence, created_at, agent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                relation.source,
                rel_type_str,
                relation.target,
                props_str,
                relation.confidence as f64,
                now,
                agent_id,
            ],
        )
        .map_err(LibreFangError::memory)?;
        Ok(id)
    }

    /// Delete all entities and relations belonging to a specific agent.
    ///
    /// Wrapped in a single transaction so a relations-then-entities
    /// failure can't leave orphan entities (relations referencing entities
    /// silently broke ranking on the next graph query). See #3501.
    pub fn delete_by_agent(&self, agent_id: &str) -> LibreFangResult<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(LibreFangError::memory)?;
        let rel_count = tx
            .execute(
                "DELETE FROM relations WHERE agent_id = ?1",
                rusqlite::params![agent_id],
            )
            .map_err(LibreFangError::memory)? as u64;
        let ent_count = tx
            .execute(
                "DELETE FROM entities WHERE agent_id = ?1",
                rusqlite::params![agent_id],
            )
            .map_err(LibreFangError::memory)? as u64;
        tx.commit().map_err(LibreFangError::memory)?;
        Ok(rel_count + ent_count)
    }

    /// Check if a relation already exists between two entities with a given type.
    pub fn has_relation(
        &self,
        source_id: &str,
        relation_type: &RelationType,
        target_id: &str,
    ) -> LibreFangResult<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;
        let rel_str =
            serde_json::to_string(relation_type).map_err(LibreFangError::serialization)?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM relations r
                 WHERE (r.source_entity = ?1 OR EXISTS (SELECT 1 FROM entities e WHERE e.id = ?1 AND e.name = r.source_entity))
                 AND r.relation_type = ?2
                 AND (r.target_entity = ?3 OR EXISTS (SELECT 1 FROM entities e WHERE e.id = ?3 AND e.name = r.target_entity))",
                rusqlite::params![source_id, rel_str, target_id],
                |row| row.get(0),
            )
            .map_err(LibreFangError::memory)?;
        Ok(count > 0)
    }

    /// Query the knowledge graph with a pattern.
    pub fn query_graph(&self, pattern: GraphPattern) -> LibreFangResult<Vec<GraphMatch>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| LibreFangError::Internal(e.to_string()))?;

        let mut sql = String::from(
            "SELECT
                s.id, s.entity_type, s.name, s.properties, s.created_at, s.updated_at,
                r.id, r.source_entity, r.relation_type, r.target_entity, r.properties, r.confidence, r.created_at,
                t.id, t.entity_type, t.name, t.properties, t.created_at, t.updated_at
             FROM relations r
             JOIN entities s ON (r.source_entity = s.id OR (r.source_entity = s.name AND s.agent_id = r.agent_id))
             JOIN entities t ON (r.target_entity = t.id OR (r.target_entity = t.name AND t.agent_id = r.agent_id))
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
                ),
                relation: parse_relation(
                    &r.r_source,
                    &r.r_type,
                    &r.r_target,
                    &r.r_props,
                    r.r_confidence,
                    &r.r_created,
                ),
                target: parse_entity(
                    &r.t_id,
                    &r.t_type,
                    &r.t_name,
                    &r.t_props,
                    &r.t_created,
                    &r.t_updated,
                ),
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
) -> Entity {
    let entity_type: EntityType =
        serde_json::from_str(etype).unwrap_or(EntityType::Custom("unknown".to_string()));
    let properties: HashMap<String, serde_json::Value> =
        serde_json::from_str(props).unwrap_or_default();
    let created_at = chrono::DateTime::parse_from_rfc3339(created)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = chrono::DateTime::parse_from_rfc3339(updated)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Entity {
        id: id.to_string(),
        entity_type,
        name: name.to_string(),
        properties,
        created_at,
        updated_at,
    }
}

fn parse_relation(
    source: &str,
    rtype: &str,
    target: &str,
    props: &str,
    confidence: f64,
    created: &str,
) -> Relation {
    let relation: RelationType = serde_json::from_str(rtype).unwrap_or(RelationType::RelatedTo);
    let properties: HashMap<String, serde_json::Value> =
        serde_json::from_str(props).unwrap_or_default();
    let created_at = chrono::DateTime::parse_from_rfc3339(created)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Relation {
        source: source.to_string(),
        relation,
        target: target.to_string(),
        properties,
        confidence: confidence as f32,
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn setup() -> KnowledgeStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        KnowledgeStore::new(Arc::new(Mutex::new(conn)))
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
}
