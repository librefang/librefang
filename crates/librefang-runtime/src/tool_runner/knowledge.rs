//! Knowledge-graph tools — add_entity / add_relation / query.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576) — fourth slice after `tool_runner::{cron, schedule, task}`. These
//! tools take no caller agent id (no per-caller authorization), so the
//! migration is purely the error-channel type. The `parse_entity_type` /
//! `parse_relation_type` helpers are infallible and unchanged.

use super::error::{ToolError, ToolResult};
use super::require_kernel_typed;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

fn parse_entity_type(s: &str) -> librefang_types::memory::EntityType {
    use librefang_types::memory::EntityType;
    match s.to_lowercase().as_str() {
        "person" => EntityType::Person,
        "organization" | "org" => EntityType::Organization,
        "project" => EntityType::Project,
        "concept" => EntityType::Concept,
        "event" => EntityType::Event,
        "location" => EntityType::Location,
        "document" | "doc" => EntityType::Document,
        "tool" => EntityType::Tool,
        other => EntityType::Custom(other.to_string()),
    }
}

fn parse_relation_type(s: &str) -> librefang_types::memory::RelationType {
    use librefang_types::memory::RelationType;
    match s.to_lowercase().as_str() {
        "works_at" | "worksat" => RelationType::WorksAt,
        "knows_about" | "knowsabout" | "knows" => RelationType::KnowsAbout,
        "related_to" | "relatedto" | "related" => RelationType::RelatedTo,
        "depends_on" | "dependson" | "depends" => RelationType::DependsOn,
        "owned_by" | "ownedby" => RelationType::OwnedBy,
        "created_by" | "createdby" => RelationType::CreatedBy,
        "located_in" | "locatedin" => RelationType::LocatedIn,
        "part_of" | "partof" => RelationType::PartOf,
        "uses" => RelationType::Uses,
        "produces" => RelationType::Produces,
        other => RelationType::Custom(other.to_string()),
    }
}

pub(super) async fn tool_knowledge_add_entity(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let name = input["name"]
        .as_str()
        .ok_or(ToolError::MissingParameter("name"))?;
    let entity_type_str = input["entity_type"]
        .as_str()
        .ok_or(ToolError::MissingParameter("entity_type"))?;
    let properties = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let entity = librefang_types::memory::Entity {
        id: String::new(), // kernel/store assigns a real ID
        entity_type: parse_entity_type(entity_type_str),
        name: name.to_string(),
        properties,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let id = kh
        .knowledge_add_entity(&entity)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!("Entity '{name}' added with ID: {id}"))
}

pub(super) async fn tool_knowledge_add_relation(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let source = input["source"]
        .as_str()
        .ok_or(ToolError::MissingParameter("source"))?;
    let relation_str = input["relation"]
        .as_str()
        .ok_or(ToolError::MissingParameter("relation"))?;
    let target = input["target"]
        .as_str()
        .ok_or(ToolError::MissingParameter("target"))?;
    let confidence = input["confidence"].as_f64().unwrap_or(1.0) as f32;
    let properties = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let relation = librefang_types::memory::Relation {
        source: source.to_string(),
        relation: parse_relation_type(relation_str),
        target: target.to_string(),
        properties,
        confidence,
        created_at: chrono::Utc::now(),
    };

    let id = kh
        .knowledge_add_relation(&relation)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!(
        "Relation '{source}' --[{relation_str}]--> '{target}' added with ID: {id}"
    ))
}

pub(super) async fn tool_knowledge_query(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let source = input["source"].as_str().map(|s| s.to_string());
    let target = input["target"].as_str().map(|s| s.to_string());
    let relation = input["relation"].as_str().map(parse_relation_type);
    // Cap depth to prevent LLM-triggered DoS via exponential graph
    // traversal. Knowledge graphs rarely benefit from depth > 5 and
    // the backend traversal is O(branching_factor^depth).
    const MAX_KNOWLEDGE_DEPTH: u64 = 10;
    let max_depth = input["max_depth"]
        .as_u64()
        .unwrap_or(1)
        .min(MAX_KNOWLEDGE_DEPTH) as u32;

    let pattern = librefang_types::memory::GraphPattern {
        source,
        relation,
        target,
        max_depth,
    };

    let matches = kh
        .knowledge_query(pattern)
        .await
        .map_err(ToolError::upstream)?;
    if matches.is_empty() {
        return Ok("No matching knowledge graph entries found.".to_string());
    }

    let mut output = format!("Found {} match(es):\n", matches.len());
    for m in &matches {
        output.push_str(&format!(
            "\n  {} ({:?}) --[{:?} ({:.0}%)]--> {} ({:?})",
            m.source.name,
            m.source.entity_type,
            m.relation.relation,
            m.relation.confidence * 100.0,
            m.target.name,
            m.target.entity_type,
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn knowledge_add_entity_without_kernel_returns_unavailable() {
        let r = tool_knowledge_add_entity(&json!({}), None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn knowledge_add_relation_without_kernel_returns_unavailable() {
        let r = tool_knowledge_add_relation(&json!({}), None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn knowledge_query_without_kernel_returns_unavailable() {
        let r = tool_knowledge_query(&json!({}), None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[test]
    fn parse_entity_type_maps_known_and_custom() {
        use librefang_types::memory::EntityType;
        assert!(matches!(parse_entity_type("person"), EntityType::Person));
        assert!(matches!(parse_entity_type("org"), EntityType::Organization));
        match parse_entity_type("alien") {
            EntityType::Custom(s) => assert_eq!(s, "alien"),
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn parse_relation_type_maps_known_and_custom() {
        use librefang_types::memory::RelationType;
        assert!(matches!(
            parse_relation_type("works_at"),
            RelationType::WorksAt
        ));
        assert!(matches!(
            parse_relation_type("knows"),
            RelationType::KnowsAbout
        ));
        match parse_relation_type("orbits") {
            RelationType::Custom(s) => assert_eq!(s, "orbits"),
            other => panic!("expected Custom, got {other:?}"),
        }
    }
}
