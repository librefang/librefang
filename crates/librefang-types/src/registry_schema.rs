//! Machine-parseable registry schema types.
//!
//! Loaded from `~/.librefang/schema.toml` (synced from the registry) and
//! served via `GET /api/registry/schema` so the dashboard can auto-generate
//! forms for creating/editing registry content (agents, hands, integrations, etc.).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level registry schema — maps content type name to its schema definition.
///
/// Example TOML:
/// ```toml
/// [provider]
/// description = "LLM provider configuration"
/// file_pattern = "providers/*.toml"
///
/// [provider.fields.id]
/// type = "string"
/// required = true
/// description = "Unique provider identifier"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistrySchema {
    #[serde(flatten)]
    pub content_types: HashMap<String, ContentTypeSchema>,
}

/// Schema for a single content type (e.g. "provider", "agent", "hand").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentTypeSchema {
    /// Human-readable description of this content type.
    #[serde(default)]
    pub description: Option<String>,
    /// Glob pattern for files of this type (e.g. "providers/*.toml").
    #[serde(default)]
    pub file_pattern: Option<String>,
    /// Top-level fields for this content type.
    #[serde(default)]
    pub fields: HashMap<String, FieldSchema>,
    /// Nested sections (e.g. `[model]` inside agent, `[transport]` inside integration).
    #[serde(default)]
    pub sections: HashMap<String, SectionSchema>,
}

/// Schema for a nested section within a content type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SectionSchema {
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this section can appear multiple times (e.g. `[[models]]`, `[[requires]]`).
    #[serde(default)]
    pub repeatable: bool,
    /// Fields within this section.
    #[serde(default)]
    pub fields: HashMap<String, FieldSchema>,
    /// Sub-sections within this section.
    #[serde(default)]
    pub sections: HashMap<String, SectionSchema>,
}

/// Schema for a single field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FieldSchema {
    /// Field type: "string", "bool", "number", "array", "object".
    #[serde(rename = "type")]
    pub field_type: String,
    /// Whether this field is required.
    #[serde(default)]
    pub required: bool,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Example value.
    #[serde(default)]
    pub example: Option<toml::Value>,
    /// Default value when omitted.
    #[serde(default, rename = "default")]
    pub default_value: Option<toml::Value>,
    /// Valid options for enum/select fields.
    #[serde(default)]
    pub options: Vec<String>,
    /// For array fields: the type of each item.
    #[serde(default)]
    pub item_type: Option<String>,
}

/// Load and parse the registry schema from disk.
///
/// Tries `<home_dir>/schema.toml` first (synced copy), then falls back to
/// `<home_dir>/registry/schema.toml` (raw clone).
pub fn load_registry_schema(home_dir: &std::path::Path) -> Option<RegistrySchema> {
    let paths = [
        home_dir.join("schema.toml"),
        home_dir.join("registry").join("schema.toml"),
    ];
    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            match toml::from_str::<RegistrySchema>(&content) {
                Ok(schema) if !schema.content_types.is_empty() => return Some(schema),
                Ok(_) => continue, // parsed but empty — probably old comment-only format
                Err(_) => continue,
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_schema() {
        let toml_str = r#"
[provider]
description = "LLM provider configuration"
file_pattern = "providers/*.toml"

[provider.fields.id]
type = "string"
required = true
description = "Unique provider identifier"
example = "anthropic"

[provider.fields.key_required]
type = "bool"
required = true
description = "Whether an API key is needed"
default = true
"#;
        let schema: RegistrySchema = toml::from_str(toml_str).unwrap();
        assert!(schema.content_types.contains_key("provider"));
        let provider = &schema.content_types["provider"];
        assert_eq!(
            provider.description.as_deref(),
            Some("LLM provider configuration")
        );
        assert_eq!(provider.fields.len(), 2);
        assert!(provider.fields["id"].required);
        assert_eq!(provider.fields["id"].field_type, "string");
    }

    #[test]
    fn test_parse_nested_sections() {
        let toml_str = r#"
[agent]
description = "Agent definition"

[agent.fields.name]
type = "string"
required = true

[agent.sections.model]
description = "LLM configuration"

[agent.sections.model.fields.provider]
type = "string"
description = "Provider ID"
example = "anthropic"

[agent.sections.model.fields.temperature]
type = "number"
description = "Sampling temperature"
default = 0.7
"#;
        let schema: RegistrySchema = toml::from_str(toml_str).unwrap();
        let agent = &schema.content_types["agent"];
        assert!(agent.sections.contains_key("model"));
        let model = &agent.sections["model"];
        assert_eq!(model.fields.len(), 2);
        assert_eq!(model.fields["provider"].field_type, "string");
    }

    #[test]
    fn test_parse_repeatable_section() {
        let toml_str = r#"
[provider]
description = "Provider with models"

[provider.sections.models]
description = "Model entries"
repeatable = true

[provider.sections.models.fields.id]
type = "string"
required = true
"#;
        let schema: RegistrySchema = toml::from_str(toml_str).unwrap();
        assert!(schema.content_types["provider"].sections["models"].repeatable);
    }

    #[test]
    fn test_parse_field_with_options() {
        let toml_str = r#"
[agent]

[agent.fields.tier]
type = "string"
required = true
options = ["frontier", "smart", "balanced", "fast", "local"]
"#;
        let schema: RegistrySchema = toml::from_str(toml_str).unwrap();
        assert_eq!(
            schema.content_types["agent"].fields["tier"].options.len(),
            5
        );
    }

    #[test]
    fn test_empty_schema_returns_none() {
        assert!(load_registry_schema(std::path::Path::new("/nonexistent")).is_none());
    }
}
