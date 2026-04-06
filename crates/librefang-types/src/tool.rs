//! Tool definition and result types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Definition of a tool that an agent can use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Unique tool identifier.
    pub name: String,
    /// Human-readable description for the LLM.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: serde_json::Value,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this tool use instance.
    pub id: String,
    /// Which tool to call.
    pub name: String,
    /// The input parameters.
    pub input: serde_json::Value,
}

/// Execution status of a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    #[default]
    Completed,
    Error,
    WaitingApproval,
    Denied,
    Expired,
    ModifyAndRetry,
    Skipped,
}

impl ToolExecutionStatus {
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Self::Error | Self::Denied | Self::Expired | Self::ModifyAndRetry
        )
    }

    /// Errors that should NOT abort the remaining tool calls —
    /// the LLM can recover by retrying a valid path.
    pub fn is_soft_error(&self) -> bool {
        matches!(self, Self::Denied | Self::ModifyAndRetry | Self::Skipped)
    }
}

/// Result of a tool execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolResult {
    /// The tool_use ID this result corresponds to.
    pub tool_use_id: String,
    /// The output content.
    pub content: String,
    /// Whether the tool execution resulted in an error.
    pub is_error: bool,
    /// Detailed execution status.
    #[serde(default)]
    pub status: ToolExecutionStatus,
    /// Approval request ID, set when status is WaitingApproval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_request_id: Option<String>,
    /// Tool name, set when status is WaitingApproval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ToolResult {
    pub fn ok(tool_use_id: String, content: String) -> Self {
        Self {
            tool_use_id,
            content,
            is_error: false,
            status: ToolExecutionStatus::Completed,
            ..Default::default()
        }
    }

    pub fn error(tool_use_id: String, content: String) -> Self {
        Self {
            tool_use_id,
            content,
            is_error: true,
            status: ToolExecutionStatus::Error,
            ..Default::default()
        }
    }

    pub fn waiting_approval(tool_use_id: String, request_id: String, tool_name: String) -> Self {
        Self {
            tool_use_id,
            content: format!(
                "Tool '{}' requires human approval. Request submitted (ID: {}). Continue with other tasks — you will be notified when resolved.",
                tool_name, request_id
            ),
            is_error: false,
            status: ToolExecutionStatus::WaitingApproval,
            approval_request_id: Some(request_id),
            tool_name: Some(tool_name),
        }
    }

    pub fn with_status(tool_use_id: String, content: String, status: ToolExecutionStatus) -> Self {
        Self {
            tool_use_id,
            content,
            is_error: status.is_error(),
            status,
            ..Default::default()
        }
    }
}

/// Captures everything needed to execute (or build a terminal result for) a tool
/// after an approval decision. Stored in the approval manager while awaiting human decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferredToolExecution {
    pub agent_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub allowed_tools: Option<Vec<String>>,
    pub sender_id: Option<String>,
    pub channel: Option<String>,
    pub workspace_root: Option<std::path::PathBuf>,
}

/// Outcome of submitting a deferred tool for approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalSubmission {
    Pending { request_id: uuid::Uuid },
    AutoApproved,
}

/// Mid-turn signal sent into a running agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentLoopSignal {
    Message {
        content: String,
    },
    ApprovalResolved {
        tool_use_id: String,
        tool_name: String,
        decision: String,
        result_content: String,
        result_is_error: bool,
        result_status: ToolExecutionStatus,
    },
}

/// A structured trace of a tool selection decision during agent execution.
///
/// Captures why a tool was selected (the LLM's reasoning), what input was provided,
/// how long execution took, and whether it succeeded. Useful for debugging,
/// auditing, and optimizing agent behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTrace {
    /// Unique ID of the tool call (matches the LLM's tool_use ID).
    pub tool_use_id: String,
    /// Name of the tool that was selected.
    pub tool_name: String,
    /// The input parameters the LLM provided for the tool call.
    pub input: serde_json::Value,
    /// The LLM's reasoning text from the same response that triggered this tool call.
    /// This is the assistant text that accompanied the tool_use blocks, providing
    /// insight into why the tool was selected.
    pub rationale: Option<String>,
    /// Whether this tool was recovered from text output (non-native tool calling).
    pub recovered_from_text: bool,
    /// Wall-clock execution duration in milliseconds.
    pub execution_ms: u64,
    /// Whether the tool execution resulted in an error.
    pub is_error: bool,
    /// Truncated summary of the tool output (first 200 chars).
    pub output_summary: String,
    /// The loop iteration in which this tool was called.
    pub iteration: u32,
    /// Timestamp when the tool execution started.
    pub timestamp: DateTime<Utc>,
}

/// Normalize a JSON Schema for cross-provider compatibility.
///
/// Some providers (Gemini, Groq) reject `anyOf` in tool schemas.
/// This function:
/// - Converts `anyOf` arrays of simple types to flat `enum` arrays
/// - Strips `$schema` keys (not accepted by most providers)
/// - Recursively walks `properties` and `items`
pub fn normalize_schema_for_provider(
    schema: &serde_json::Value,
    provider: &str,
) -> serde_json::Value {
    // Anthropic handles anyOf natively — no normalization needed
    if provider == "anthropic" {
        return schema.clone();
    }
    normalize_schema_recursive(schema)
}

fn normalize_schema_recursive(schema: &serde_json::Value) -> serde_json::Value {
    let obj = match schema.as_object() {
        Some(o) => o,
        None => {
            // If the schema is a JSON string, try to parse it as a JSON object.
            // Some MCP servers / skill definitions serialize schemas as strings.
            if let Some(s) = schema.as_str() {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
                    if parsed.is_object() {
                        return normalize_schema_recursive(&parsed);
                    }
                }
            }
            // Non-object schema (null, number, bool, unparseable string, array) —
            // return a valid empty object schema so providers don't reject it.
            return serde_json::json!({"type": "object", "properties": {}});
        }
    };

    // Resolve $ref references before processing.
    // If the schema has $defs and $ref, inline the referenced definition.
    let resolved = resolve_refs(obj);
    let obj = resolved.as_object().unwrap_or(obj);

    let mut result = serde_json::Map::new();

    for (key, value) in obj {
        // Strip fields unsupported by Gemini and most non-Anthropic providers
        if matches!(
            key.as_str(),
            "$schema"
                | "$defs"
                | "$ref"
                | "additionalProperties"
                | "default"
                | "$id"
                | "$comment"
                | "examples"
                | "title"
                | "const"
                | "format"
        ) {
            continue;
        }

        // Convert anyOf/oneOf to flat type + enum when possible
        if key == "anyOf" || key == "oneOf" {
            if let Some(converted) = try_flatten_any_of(value) {
                for (k, v) in converted {
                    result.insert(k, v);
                }
                continue;
            }
            // Can't flatten — strip entirely rather than leave unsupported keyword
            continue;
        }

        // Flatten type arrays like ["string", "null"] to single type + nullable
        if key == "type" {
            if let Some(arr) = value.as_array() {
                let types: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                let has_null = types.contains(&"null");
                let non_null: Vec<&&str> = types.iter().filter(|&&t| t != "null").collect();
                if has_null && non_null.len() == 1 {
                    // ["string", "null"] → type: "string", nullable: true
                    result.insert(
                        "type".to_string(),
                        serde_json::Value::String(non_null[0].to_string()),
                    );
                    result.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    continue;
                } else if non_null.len() == 1 {
                    // ["string"] → type: "string"
                    result.insert(
                        "type".to_string(),
                        serde_json::Value::String(non_null[0].to_string()),
                    );
                    continue;
                } else if !non_null.is_empty() {
                    // Multiple non-null types — pick first (best effort)
                    result.insert(
                        "type".to_string(),
                        serde_json::Value::String(non_null[0].to_string()),
                    );
                    if has_null {
                        result.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    }
                    continue;
                }
            }
            // Scalar type string — pass through
            result.insert(key.clone(), value.clone());
            continue;
        }

        // Recurse into properties
        if key == "properties" {
            if let Some(props) = value.as_object() {
                let mut new_props = serde_json::Map::new();
                for (prop_name, prop_schema) in props {
                    new_props.insert(prop_name.clone(), normalize_schema_recursive(prop_schema));
                }
                result.insert(key.clone(), serde_json::Value::Object(new_props));
                continue;
            }
        }

        // Recurse into items
        if key == "items" {
            result.insert(key.clone(), normalize_schema_recursive(value));
            continue;
        }

        result.insert(key.clone(), value.clone());
    }

    serde_json::Value::Object(result)
}

/// Resolve `$ref` references by inlining definitions from `$defs`.
///
/// If the schema has `$defs` and any property uses `$ref: "#/$defs/Foo"`,
/// replace the `$ref` with the actual definition. This is needed because
/// Gemini and most providers don't support `$ref`/`$defs`.
fn resolve_refs(obj: &serde_json::Map<String, serde_json::Value>) -> serde_json::Value {
    let defs = match obj.get("$defs").and_then(|d| d.as_object()) {
        Some(d) => d.clone(),
        None => return serde_json::Value::Object(obj.clone()),
    };

    let mut result = obj.clone();
    result.remove("$defs");

    // Recursively replace $ref in the schema
    fn inline_refs(val: &mut serde_json::Value, defs: &serde_json::Map<String, serde_json::Value>) {
        match val {
            serde_json::Value::Object(map) => {
                // If this object is a $ref, replace it with the definition
                if let Some(ref_val) = map.get("$ref").and_then(|r| r.as_str()) {
                    let ref_name = ref_val
                        .strip_prefix("#/$defs/")
                        .or_else(|| ref_val.strip_prefix("#/definitions/"));
                    if let Some(name) = ref_name {
                        if let Some(def) = defs.get(name) {
                            *val = def.clone();
                            // Recurse into the inlined definition
                            inline_refs(val, defs);
                            return;
                        }
                    }
                }
                // Recurse into all values
                for v in map.values_mut() {
                    inline_refs(v, defs);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr.iter_mut() {
                    inline_refs(item, defs);
                }
            }
            _ => {}
        }
    }

    let mut resolved = serde_json::Value::Object(result);
    inline_refs(&mut resolved, &defs);
    resolved
}

/// Try to flatten an `anyOf` array into a simple type + enum.
///
/// Works when all variants are simple types (string, number, etc.) or
/// when it's a nullable pattern like `anyOf: [{type: "string"}, {type: "null"}]`.
fn try_flatten_any_of(any_of: &serde_json::Value) -> Option<Vec<(String, serde_json::Value)>> {
    let items = any_of.as_array()?;
    if items.is_empty() {
        return None;
    }

    // Check if this is a simple type union (all items have just "type")
    let mut types = Vec::new();
    let mut has_null = false;
    let mut non_null_type = None;

    for item in items {
        let obj = item.as_object()?;
        let type_val = obj.get("type")?.as_str()?;

        if type_val == "null" {
            has_null = true;
        } else {
            types.push(type_val.to_string());
            non_null_type = Some(type_val.to_string());
        }
    }

    // If it's a nullable pattern (type + null), emit the non-null type
    if has_null && types.len() == 1 {
        let mut result = vec![(
            "type".to_string(),
            serde_json::Value::String(non_null_type.unwrap()),
        )];
        // Mark as nullable via description hint (since JSON Schema nullable isn't universal)
        result.push(("nullable".to_string(), serde_json::Value::Bool(true)));
        return Some(result);
    }

    // If all items are simple types, pick the first non-null type (best effort).
    // Gemini rejects type arrays, so we can't emit ["string", "number"].
    if types.len() == items.len() && types.len() > 1 {
        let mut result = vec![(
            "type".to_string(),
            serde_json::Value::String(types[0].clone()),
        )];
        if has_null {
            result.push(("nullable".to_string(), serde_json::Value::Bool(true)));
        }
        return Some(result);
    }

    // Can't flatten — caller will strip the key entirely
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_trace_serialization_roundtrip() {
        let trace = DecisionTrace {
            tool_use_id: "toolu_abc123".to_string(),
            tool_name: "web_search".to_string(),
            input: serde_json::json!({"query": "rust async"}),
            rationale: Some("I need to search for information about Rust async".to_string()),
            recovered_from_text: false,
            execution_ms: 150,
            is_error: false,
            output_summary: "Found 10 results about Rust async programming".to_string(),
            iteration: 0,
            timestamp: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&trace).unwrap();
        let deserialized: DecisionTrace = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.tool_name, "web_search");
        assert_eq!(deserialized.tool_use_id, "toolu_abc123");
        assert_eq!(deserialized.execution_ms, 150);
        assert!(!deserialized.is_error);
        assert!(!deserialized.recovered_from_text);
        assert!(deserialized.rationale.is_some());
    }

    #[test]
    fn test_decision_trace_without_rationale() {
        let trace = DecisionTrace {
            tool_use_id: "toolu_xyz".to_string(),
            tool_name: "file_read".to_string(),
            input: serde_json::json!({"path": "/tmp/test.txt"}),
            rationale: None,
            recovered_from_text: true,
            execution_ms: 5,
            is_error: true,
            output_summary: "File not found".to_string(),
            iteration: 2,
            timestamp: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&trace).unwrap();
        assert!(json.contains("\"rationale\":null"));
        assert!(json.contains("\"recovered_from_text\":true"));
        assert!(json.contains("\"is_error\":true"));
    }

    #[test]
    fn test_tool_definition_serialization() {
        let tool = ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["query"]
            }),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("web_search"));
    }

    #[test]
    fn test_normalize_schema_strips_dollar_schema() {
        let schema = serde_json::json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        assert!(result.get("$schema").is_none());
        assert_eq!(result["type"], "object");
    }

    #[test]
    fn test_normalize_schema_flattens_anyof_nullable() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "value": {
                    "anyOf": [
                        { "type": "string" },
                        { "type": "null" }
                    ]
                }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        let value_prop = &result["properties"]["value"];
        assert_eq!(value_prop["type"], "string");
        assert_eq!(value_prop["nullable"], true);
        assert!(value_prop.get("anyOf").is_none());
    }

    #[test]
    fn test_normalize_schema_flattens_anyof_multi_type() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "value": {
                    "anyOf": [
                        { "type": "string" },
                        { "type": "number" }
                    ]
                }
            }
        });
        let result = normalize_schema_for_provider(&schema, "groq");
        let value_prop = &result["properties"]["value"];
        // Gemini rejects type arrays — should flatten to first type
        assert_eq!(value_prop["type"], "string");
        assert!(value_prop.get("anyOf").is_none());
    }

    #[test]
    fn test_normalize_schema_anthropic_passthrough() {
        let schema = serde_json::json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "anyOf": [{"type": "string"}]
        });
        let result = normalize_schema_for_provider(&schema, "anthropic");
        // Anthropic should get the original schema unchanged
        assert!(result.get("$schema").is_some());
    }

    #[test]
    fn test_normalize_schema_nested_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "outer": {
                    "type": "object",
                    "properties": {
                        "inner": {
                            "$schema": "strip_me",
                            "type": "string"
                        }
                    }
                }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        assert!(result["properties"]["outer"]["properties"]["inner"]
            .get("$schema")
            .is_none());
    }

    #[test]
    fn test_normalize_schema_string_parsed_to_object() {
        // MCP servers may return inputSchema as a JSON string
        let schema = serde_json::Value::String(
            r#"{"type":"object","properties":{"query":{"type":"string"}}}"#.to_string(),
        );
        let result = normalize_schema_for_provider(&schema, "openai");
        assert!(result.is_object());
        assert_eq!(result["type"], "object");
        assert!(result["properties"]["query"].is_object());
    }

    #[test]
    fn test_normalize_schema_null_becomes_empty_object() {
        let schema = serde_json::Value::Null;
        let result = normalize_schema_for_provider(&schema, "openai");
        assert!(result.is_object());
        assert_eq!(result["type"], "object");
    }

    #[test]
    fn test_normalize_schema_unparseable_string_becomes_empty_object() {
        let schema = serde_json::Value::String("not valid json".to_string());
        let result = normalize_schema_for_provider(&schema, "openai");
        assert!(result.is_object());
        assert_eq!(result["type"], "object");
    }

    #[test]
    fn test_normalize_schema_number_becomes_empty_object() {
        let schema = serde_json::json!(42);
        let result = normalize_schema_for_provider(&schema, "openai");
        assert!(result.is_object());
        assert_eq!(result["type"], "object");
    }

    #[test]
    fn test_normalize_schema_string_with_dollar_schema_stripped() {
        // String schema that contains $schema — should be parsed AND normalized
        let schema = serde_json::Value::String(
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{}}"#.to_string(),
        );
        let result = normalize_schema_for_provider(&schema, "openai");
        assert!(result.is_object());
        assert_eq!(result["type"], "object");
        assert!(result.get("$schema").is_none());
    }

    #[test]
    fn test_normalize_strips_additional_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "default": "hello", "title": "Name" }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        assert!(result.get("additionalProperties").is_none());
        assert!(result["properties"]["name"].get("default").is_none());
        assert!(result["properties"]["name"].get("title").is_none());
        assert_eq!(result["properties"]["name"]["type"], "string");
    }

    #[test]
    fn test_normalize_resolves_refs() {
        let schema = serde_json::json!({
            "type": "object",
            "$defs": {
                "Color": {
                    "type": "string",
                    "enum": ["red", "green", "blue"]
                }
            },
            "properties": {
                "color": { "$ref": "#/$defs/Color" }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        assert!(result.get("$defs").is_none());
        assert_eq!(result["properties"]["color"]["type"], "string");
        assert!(result["properties"]["color"]["enum"].is_array());
    }

    #[test]
    fn test_normalize_strips_defs_without_refs() {
        let schema = serde_json::json!({
            "type": "object",
            "$defs": { "Unused": { "type": "number" } },
            "properties": {
                "x": { "type": "string" }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        assert!(result.get("$defs").is_none());
        assert_eq!(result["properties"]["x"]["type"], "string");
    }

    // --- Issue #488 tests ---

    #[test]
    fn test_normalize_strips_const() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "version": { "type": "string", "const": "v1" }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        assert!(result["properties"]["version"].get("const").is_none());
        assert_eq!(result["properties"]["version"]["type"], "string");
    }

    #[test]
    fn test_normalize_strips_format() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "created_at": { "type": "string", "format": "date-time" },
                "email": { "type": "string", "format": "email" }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        assert!(result["properties"]["created_at"].get("format").is_none());
        assert!(result["properties"]["email"].get("format").is_none());
        assert_eq!(result["properties"]["created_at"]["type"], "string");
        assert_eq!(result["properties"]["email"]["type"], "string");
    }

    #[test]
    fn test_normalize_flattens_oneof_nullable() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "value": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "null" }
                    ]
                }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        let value_prop = &result["properties"]["value"];
        assert_eq!(value_prop["type"], "string");
        assert_eq!(value_prop["nullable"], true);
        assert!(value_prop.get("oneOf").is_none());
    }

    #[test]
    fn test_normalize_strips_oneof_complex() {
        // Complex oneOf that can't be flattened — should be stripped entirely
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "data": {
                    "oneOf": [
                        { "type": "object", "properties": { "a": { "type": "string" } } },
                        { "type": "object", "properties": { "b": { "type": "number" } } }
                    ]
                }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        let data_prop = &result["properties"]["data"];
        assert!(data_prop.get("oneOf").is_none());
    }

    #[test]
    fn test_normalize_flattens_type_array_nullable() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": ["string", "null"] }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        let name_prop = &result["properties"]["name"];
        assert_eq!(name_prop["type"], "string");
        assert_eq!(name_prop["nullable"], true);
    }

    #[test]
    fn test_normalize_flattens_type_array_multi() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": ["string", "number", "null"] }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        let value_prop = &result["properties"]["value"];
        // Should pick first non-null type
        assert_eq!(value_prop["type"], "string");
        assert_eq!(value_prop["nullable"], true);
    }

    #[test]
    fn test_normalize_flattens_type_array_single() {
        // Single-element type array
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": ["integer"] }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        assert_eq!(result["properties"]["x"]["type"], "integer");
        assert!(result["properties"]["x"].get("nullable").is_none());
    }

    #[test]
    fn test_normalize_strips_anyof_complex() {
        // Complex anyOf that can't be flattened — should be stripped entirely
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "payload": {
                    "anyOf": [
                        { "type": "object", "properties": { "url": { "type": "string" } } },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        let payload_prop = &result["properties"]["payload"];
        assert!(payload_prop.get("anyOf").is_none());
    }

    #[test]
    fn test_normalize_combined_issue_488() {
        // Real-world schema combining multiple #488 issues
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "api_version": { "type": "string", "const": "v2", "format": "semver" },
                "timestamp": { "type": "string", "format": "date-time" },
                "label": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "null" }
                    ]
                },
                "tags": { "type": ["string", "null"] }
            }
        });
        let result = normalize_schema_for_provider(&schema, "gemini");
        // const and format stripped
        assert!(result["properties"]["api_version"].get("const").is_none());
        assert!(result["properties"]["api_version"].get("format").is_none());
        assert!(result["properties"]["timestamp"].get("format").is_none());
        // oneOf flattened
        assert_eq!(result["properties"]["label"]["type"], "string");
        assert_eq!(result["properties"]["label"]["nullable"], true);
        assert!(result["properties"]["label"].get("oneOf").is_none());
        // type array flattened
        assert_eq!(result["properties"]["tags"]["type"], "string");
        assert_eq!(result["properties"]["tags"]["nullable"], true);
    }

    // -----------------------------------------------------------------------
    // ToolResult tests (Step 1 from plan)
    // -----------------------------------------------------------------------

    #[test]
    fn test_tool_result_ok_constructor() {
        let result = ToolResult::ok("toolu_abc".to_string(), "Success".to_string());
        assert_eq!(result.tool_use_id, "toolu_abc");
        assert_eq!(result.content, "Success");
        assert!(!result.is_error);
        assert_eq!(result.status, ToolExecutionStatus::Completed);
    }

    #[test]
    fn test_tool_result_error_constructor() {
        let result = ToolResult::error("toolu_abc".to_string(), "Failed".to_string());
        assert_eq!(result.tool_use_id, "toolu_abc");
        assert_eq!(result.content, "Failed");
        assert!(result.is_error);
        assert_eq!(result.status, ToolExecutionStatus::Error);
    }

    #[test]
    fn test_tool_result_waiting_approval_constructor() {
        let result = ToolResult::waiting_approval(
            "toolu_abc".to_string(),
            "req-123".to_string(),
            "shell_exec".to_string(),
        );
        assert_eq!(result.tool_use_id, "toolu_abc");
        assert!(!result.is_error);
        assert_eq!(result.status, ToolExecutionStatus::WaitingApproval);
        assert_eq!(result.approval_request_id, Some("req-123".to_string()));
        assert_eq!(result.tool_name, Some("shell_exec".to_string()));
    }

    #[test]
    fn test_tool_result_serde_roundtrip() {
        let original = ToolResult::waiting_approval(
            "toolu_abc".to_string(),
            "req-123".to_string(),
            "shell_exec".to_string(),
        );
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: ToolResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.tool_use_id, original.tool_use_id);
        assert_eq!(deserialized.status, original.status);
        assert_eq!(
            deserialized.approval_request_id,
            original.approval_request_id
        );
        assert_eq!(deserialized.tool_name, original.tool_name);
    }

    #[test]
    fn test_tool_result_deserialization_old_format() {
        // Old format without status field
        let json = r#"{
            "tool_use_id": "toolu_old",
            "content": "Old result",
            "is_error": false
        }"#;
        let result: ToolResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.tool_use_id, "toolu_old");
        assert_eq!(result.content, "Old result");
        assert!(!result.is_error);
        assert_eq!(result.status, ToolExecutionStatus::Completed); // Default
    }

    #[test]
    fn test_tool_result_default() {
        let result = ToolResult::default();
        assert_eq!(result.status, ToolExecutionStatus::Completed);
        assert!(!result.is_error);
    }

    #[test]
    fn test_tool_execution_status_is_error() {
        assert!(!ToolExecutionStatus::Completed.is_error());
        assert!(ToolExecutionStatus::Error.is_error());
        assert!(!ToolExecutionStatus::WaitingApproval.is_error());
        assert!(ToolExecutionStatus::Denied.is_error());
        assert!(ToolExecutionStatus::Expired.is_error());
        assert!(ToolExecutionStatus::ModifyAndRetry.is_error());
        assert!(!ToolExecutionStatus::Skipped.is_error());
    }

    #[test]
    fn test_tool_execution_status_is_soft_error() {
        assert!(!ToolExecutionStatus::Completed.is_soft_error());
        assert!(!ToolExecutionStatus::Error.is_soft_error());
        assert!(!ToolExecutionStatus::WaitingApproval.is_soft_error());
        assert!(ToolExecutionStatus::Denied.is_soft_error());
        assert!(!ToolExecutionStatus::Expired.is_soft_error());
        assert!(ToolExecutionStatus::ModifyAndRetry.is_soft_error());
        assert!(ToolExecutionStatus::Skipped.is_soft_error());
    }

    #[test]
    fn test_deferred_tool_execution_serialization() {
        let deferred = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "toolu_abc".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls -la"}),
            allowed_tools: Some(vec!["shell_exec".to_string()]),
            sender_id: Some("user-123".to_string()),
            channel: Some("telegram".to_string()),
            workspace_root: Some(std::path::PathBuf::from("/tmp")),
        };
        let json = serde_json::to_string(&deferred).unwrap();
        let deserialized: DeferredToolExecution = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.agent_id, "agent-1");
        assert_eq!(deserialized.tool_use_id, "toolu_abc");
        assert_eq!(deserialized.tool_name, "shell_exec");
        assert_eq!(deserialized.sender_id, Some("user-123".to_string()));
    }
}
