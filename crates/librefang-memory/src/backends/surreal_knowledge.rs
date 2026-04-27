//! SurrealDB-backed [`crate::KnowledgeBackend`] implementation.
//!
//! Stores knowledge-graph entities and relations in the `kg_entities` and
//! `kg_relations` tables (defined in `011_knowledge_graph.surql`).
//!
//! Table names are prefixed `kg_` to avoid collision with the `entities` /
//! `relations` tables managed by the `surreal-memory` library.
//!
//! All SurrealQL queries use parameterised bindings (`.bind()`) — no
//! caller-supplied strings are ever interpolated into query text.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;

use librefang_storage::pool::SurrealSession;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{
    Entity, EntityType, GraphMatch, GraphPattern, Relation, RelationType,
};

use crate::backend::KnowledgeBackend;

// ── SurrealKnowledgeBackend ───────────────────────────────────────────────────

/// SurrealDB implementation of [`KnowledgeBackend`].
pub struct SurrealKnowledgeBackend {
    db: Arc<Surreal<Any>>,
}

impl SurrealKnowledgeBackend {
    /// Open against an existing [`SurrealSession`].
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: Arc::new(session.client().clone()),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn entity_type_to_str(et: &EntityType) -> String {
    serde_json::to_string(et)
        .unwrap_or_else(|_| "\"custom\"".to_string())
        .trim_matches('"')
        .to_string()
}

fn relation_type_to_str(rt: &RelationType) -> String {
    serde_json::to_string(rt)
        .unwrap_or_else(|_| "\"related_to\"".to_string())
        .trim_matches('"')
        .to_string()
}

// ── KnowledgeBackend impl ─────────────────────────────────────────────────────

#[async_trait]
impl KnowledgeBackend for SurrealKnowledgeBackend {
    async fn add_entity(&self, entity: Entity) -> LibreFangResult<String> {
        let id = if entity.id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            entity.id.clone()
        };
        let entity_type_str = entity_type_to_str(&entity.entity_type);
        let props = serde_json::to_value(&entity.properties)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        let now = Utc::now().to_rfc3339();
        let row = serde_json::json!({
            "entity_type": entity_type_str,
            "name": entity.name,
            "properties": props,
            "created_at": entity.created_at.to_rfc3339(),
            "updated_at": now,
            "agent_id": "",
        });
        self.db
            .upsert::<Option<serde_json::Value>>(("kg_entities", id.clone()))
            .content(row)
            .await
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB kg add_entity: {e}")))?;
        Ok(id)
    }

    async fn add_relation(&self, relation: Relation) -> LibreFangResult<String> {
        let id = Uuid::new_v4().to_string();
        let relation_type_str = relation_type_to_str(&relation.relation);
        let props = serde_json::to_value(&relation.properties)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        let now = Utc::now().to_rfc3339();
        let row = serde_json::json!({
            "source_entity": relation.source,
            "relation_type": relation_type_str,
            "target_entity": relation.target,
            "properties": props,
            "confidence": relation.confidence as f64,
            "created_at": now,
            "agent_id": "",
        });
        self.db
            .create::<Option<serde_json::Value>>(("kg_relations", id.clone()))
            .content(row)
            .await
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB kg add_relation: {e}")))?;
        Ok(id)
    }

    async fn query_graph(&self, pattern: GraphPattern) -> LibreFangResult<Vec<GraphMatch>> {
        // Build the query with optional source/relation/target filters using bindings.
        // We execute up to three separate bound parameters depending on what is set.
        // The JOIN-equivalent in SurrealDB: fetch relations then resolve entity rows.
        let mut q = self.db.query(
            "SELECT r.source_entity, r.relation_type, r.target_entity, \
                    r.properties AS r_props, r.confidence, r.created_at AS r_created, \
                    s.id AS s_id, s.entity_type AS s_type, s.name AS s_name, \
                    s.properties AS s_props, s.created_at AS s_created, s.updated_at AS s_updated, \
                    t.id AS t_id, t.entity_type AS t_type, t.name AS t_name, \
                    t.properties AS t_props, t.created_at AS t_created, t.updated_at AS t_updated \
             FROM kg_relations AS r \
             LEFT JOIN kg_entities AS s ON (r.source_entity = s.id OR r.source_entity = s.name) \
             LEFT JOIN kg_entities AS t ON (r.target_entity = t.id OR r.target_entity = t.name) \
             WHERE ($source = NONE OR r.source_entity = $source OR s.name = $source) \
               AND ($rel_type = NONE OR r.relation_type = $rel_type) \
               AND ($target = NONE OR r.target_entity = $target OR t.name = $target) \
             LIMIT 100",
        );

        // Bind optional filters as NONE or the actual value.
        if let Some(ref src) = pattern.source {
            q = q.bind(("source", src.clone()));
        } else {
            q = q.bind(("source", Option::<String>::None));
        }
        if let Some(ref rel) = pattern.relation {
            q = q.bind(("rel_type", relation_type_to_str(rel)));
        } else {
            q = q.bind(("rel_type", Option::<String>::None));
        }
        if let Some(ref tgt) = pattern.target {
            q = q.bind(("target", tgt.clone()));
        } else {
            q = q.bind(("target", Option::<String>::None));
        }

        let mut res = q
            .await
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB kg query_graph: {e}")))?;
        let rows: Vec<serde_json::Value> = res
            .take(0)
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB kg query_graph result: {e}")))?;

        let matches = rows
            .into_iter()
            .map(|row| {
                // Build synthetic entity and relation objects from the flat row.
                let src_entity = Entity {
                    id: row
                        .get("s_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .split(':')
                        .next_back()
                        .unwrap_or("")
                        .to_string(),
                    entity_type: {
                        let s = row
                            .get("s_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("custom");
                        serde_json::from_str(&format!("\"{s}\""))
                            .unwrap_or(EntityType::Custom(s.to_string()))
                    },
                    name: row
                        .get("s_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    properties: row
                        .get("s_props")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default(),
                    created_at: row
                        .get("s_created")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(Utc::now),
                    updated_at: row
                        .get("s_updated")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(Utc::now),
                };
                let tgt_entity = Entity {
                    id: row
                        .get("t_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .split(':')
                        .next_back()
                        .unwrap_or("")
                        .to_string(),
                    entity_type: {
                        let s = row
                            .get("t_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("custom");
                        serde_json::from_str(&format!("\"{s}\""))
                            .unwrap_or(EntityType::Custom(s.to_string()))
                    },
                    name: row
                        .get("t_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    properties: row
                        .get("t_props")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default(),
                    created_at: row
                        .get("t_created")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(Utc::now),
                    updated_at: row
                        .get("t_updated")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(Utc::now),
                };
                let rtype_str = row
                    .get("relation_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("related_to");
                let relation_type: RelationType = serde_json::from_str(&format!("\"{rtype_str}\""))
                    .unwrap_or(RelationType::RelatedTo);
                let rel = Relation {
                    source: row
                        .get("source_entity")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    relation: relation_type,
                    target: row
                        .get("target_entity")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    properties: row
                        .get("r_props")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default(),
                    confidence: row
                        .get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0) as f32,
                    created_at: row
                        .get("r_created")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(Utc::now),
                };
                GraphMatch {
                    source: src_entity,
                    relation: rel,
                    target: tgt_entity,
                }
            })
            .collect();
        Ok(matches)
    }

    async fn delete_by_agent(&self, agent_id: &str) -> LibreFangResult<u64> {
        let agent = agent_id.to_string();
        let mut res_r = self
            .db
            .query("DELETE kg_relations WHERE agent_id = $agent_id RETURN BEFORE")
            .bind(("agent_id", agent.clone()))
            .await
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB kg delete_relations: {e}")))?;
        let rel_deleted: Vec<serde_json::Value> = res_r
            .take(0)
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB kg delete_relations: {e}")))?;

        let mut res_e = self
            .db
            .query("DELETE kg_entities WHERE agent_id = $agent_id RETURN BEFORE")
            .bind(("agent_id", agent))
            .await
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB kg delete_entities: {e}")))?;
        let ent_deleted: Vec<serde_json::Value> = res_e
            .take(0)
            .map_err(|e| LibreFangError::Memory(format!("SurrealDB kg delete_entities: {e}")))?;

        Ok((rel_deleted.len() + ent_deleted.len()) as u64)
    }
}
