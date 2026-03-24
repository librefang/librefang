//! Workflow template types — reusable parameterized workflow blueprints.
//!
//! A `WorkflowTemplate` is a blueprint that can be instantiated into a
//! concrete workflow by supplying values for its parameters. Template
//! steps use `prompt_template` strings with `{{param_name}}` placeholders
//! that are substituted at instantiation time.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Localized name + description for a template.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TemplateI18n {
    /// Translated name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Translated description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A reusable workflow blueprint with parameterized steps.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowTemplate {
    /// Unique identifier for this template.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Description of what this template produces.
    pub description: String,
    /// Optional category for grouping (e.g. "data-pipeline", "code-review").
    pub category: Option<String>,
    /// Parameters that must (or may) be supplied when instantiating.
    #[serde(default)]
    pub parameters: Vec<TemplateParameter>,
    /// The steps that make up the workflow blueprint.
    pub steps: Vec<WorkflowTemplateStep>,
    /// Free-form tags for search / filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// ISO-8601 creation timestamp (set by the registry on insert).
    pub created_at: Option<String>,
    /// Per-language overrides for name/description. Key is a BCP-47 tag (e.g. "zh", "ja").
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub i18n: HashMap<String, TemplateI18n>,
}

/// A single parameter declaration inside a workflow template.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemplateParameter {
    /// Parameter name — used as the placeholder key in prompt templates.
    pub name: String,
    /// Human-readable description shown to the user.
    pub description: Option<String>,
    /// The value type expected for this parameter.
    pub param_type: ParameterType,
    /// Optional default value; used when the caller omits this parameter.
    pub default: Option<serde_json::Value>,
    /// Whether the caller must supply this parameter (no default fallback).
    pub required: bool,
}

/// Supported parameter value types.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    /// Free-form text.
    String,
    /// Numeric value (integer or float).
    Number,
    /// True / false flag.
    Boolean,
    /// Reference to an existing agent by ID.
    AgentId,
}

/// A single step inside a workflow template.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowTemplateStep {
    /// Step name for logging and dependency references.
    pub name: String,
    /// Prompt with `{{param}}` placeholders resolved at instantiation.
    pub prompt_template: String,
    /// Optional agent selector (name or id); `None` means "use default".
    pub agent: Option<String>,
    /// Names of other steps that must complete before this one starts.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serde() {
        let tpl = WorkflowTemplate {
            id: "tpl-1".into(),
            name: "Summarise & Translate".into(),
            description: "Summarise text then translate it.".into(),
            category: Some("text".into()),
            parameters: vec![TemplateParameter {
                name: "target_lang".into(),
                description: Some("Language to translate into".into()),
                param_type: ParameterType::String,
                default: Some(serde_json::Value::String("en".into())),
                required: false,
            }],
            steps: vec![
                WorkflowTemplateStep {
                    name: "summarise".into(),
                    prompt_template: "Summarise: {{input}}".into(),
                    agent: None,
                    depends_on: vec![],
                },
                WorkflowTemplateStep {
                    name: "translate".into(),
                    prompt_template: "Translate to {{target_lang}}: {{input}}".into(),
                    agent: Some("translator-agent".into()),
                    depends_on: vec!["summarise".into()],
                },
            ],
            tags: vec!["nlp".into(), "translation".into()],
            created_at: Some("2025-01-01T00:00:00Z".into()),
        };

        let json = serde_json::to_string(&tpl).expect("serialize");
        let back: WorkflowTemplate = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.id, "tpl-1");
        assert_eq!(back.parameters.len(), 1);
        assert_eq!(back.steps.len(), 2);
        assert_eq!(back.tags, vec!["nlp", "translation"]);
    }

    #[test]
    fn deserialize_minimal() {
        let json = r#"{
            "id": "m",
            "name": "Minimal",
            "description": "d",
            "steps": [{"name":"s","prompt_template":"do it"}]
        }"#;
        let tpl: WorkflowTemplate = serde_json::from_str(json).expect("deserialize minimal");
        assert!(tpl.parameters.is_empty());
        assert!(tpl.tags.is_empty());
        assert!(tpl.category.is_none());
        assert!(tpl.created_at.is_none());
    }

    #[test]
    fn parameter_type_serde() {
        let cases = vec![
            (ParameterType::String, "\"string\""),
            (ParameterType::Number, "\"number\""),
            (ParameterType::Boolean, "\"boolean\""),
            (ParameterType::AgentId, "\"agent_id\""),
        ];
        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
        }
    }
}
