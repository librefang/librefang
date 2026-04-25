//! Map a tool name (and optional definition) to a [`ToolApprovalClass`].
//!
//! This is a passive helper — it only computes a class, it does not gate
//! execution. The approval ladder will consume it in a follow-up PR.
//!
//! Resolution order:
//! 1. If the supplied [`ToolDefinition::input_schema`] carries an
//!    `x-tool-class` extension key (a JSON-Schema vendor extension) whose
//!    value is a known snake_case identifier, that wins. We also accept the
//!    same key nested under a top-level `metadata` object so future tool
//!    schemas can group bookkeeping fields without breaking the classifier.
//! 2. Otherwise we pattern-match the tool name against a hand-curated list.
//! 3. Anything else falls through to [`ToolApprovalClass::Unknown`].

use librefang_types::tool::ToolDefinition;
use librefang_types::tool_class::ToolApprovalClass;

/// Classify a tool by name, honoring an explicit `x-tool-class` annotation
/// inside the definition's `input_schema` when present.
pub fn classify_tool(name: &str, definition: Option<&ToolDefinition>) -> ToolApprovalClass {
    if let Some(def) = definition {
        if let Some(explicit) = explicit_class_from_schema(&def.input_schema) {
            return explicit;
        }
    }
    classify_by_name(name)
}

fn classify_by_name(name: &str) -> ToolApprovalClass {
    match name {
        "file_read" | "glob" | "grep" | "ls" | "cat" => ToolApprovalClass::ReadonlyScoped,
        "web_search" | "web_fetch" => ToolApprovalClass::ReadonlySearch,
        "file_write" | "file_edit" | "apply_patch" => ToolApprovalClass::Mutating,
        "shell_exec" | "python_exec" | "exec" => ToolApprovalClass::ExecCapable,
        "config_set" | "agent_spawn" | "agent_kill" | "kernel_reload" => {
            ToolApprovalClass::ControlPlane
        }
        "approval_request" | "totp_request" => ToolApprovalClass::Interactive,
        _ => ToolApprovalClass::Unknown,
    }
}

/// Look for an explicit class annotation in a tool's input schema.
///
/// Accepts either:
/// - top-level `"x-tool-class": "<snake_case>"`, or
/// - `"metadata": { "tool_class": "<snake_case>" }`
fn explicit_class_from_schema(schema: &serde_json::Value) -> Option<ToolApprovalClass> {
    let obj = schema.as_object()?;

    if let Some(s) = obj.get("x-tool-class").and_then(|v| v.as_str()) {
        if let Some(c) = ToolApprovalClass::from_snake_case(s) {
            return Some(c);
        }
    }

    if let Some(meta) = obj.get("metadata").and_then(|v| v.as_object()) {
        if let Some(s) = meta.get("tool_class").and_then(|v| v.as_str()) {
            if let Some(c) = ToolApprovalClass::from_snake_case(s) {
                return Some(c);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_with_schema(schema: serde_json::Value) -> ToolDefinition {
        ToolDefinition {
            name: "file_read".to_string(),
            description: "test".to_string(),
            input_schema: schema,
        }
    }

    #[test]
    fn readonly_scoped_names() {
        for n in ["file_read", "glob", "grep", "ls", "cat"] {
            assert_eq!(
                classify_tool(n, None),
                ToolApprovalClass::ReadonlyScoped,
                "{n} should be ReadonlyScoped"
            );
        }
    }

    #[test]
    fn readonly_search_names() {
        for n in ["web_search", "web_fetch"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::ReadonlySearch);
        }
    }

    #[test]
    fn mutating_names() {
        for n in ["file_write", "file_edit", "apply_patch"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::Mutating);
        }
    }

    #[test]
    fn exec_capable_names() {
        for n in ["shell_exec", "python_exec", "exec"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::ExecCapable);
        }
    }

    #[test]
    fn control_plane_names() {
        for n in ["config_set", "agent_spawn", "agent_kill", "kernel_reload"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::ControlPlane);
        }
    }

    #[test]
    fn interactive_names() {
        for n in ["approval_request", "totp_request"] {
            assert_eq!(classify_tool(n, None), ToolApprovalClass::Interactive);
        }
    }

    #[test]
    fn unknown_falls_through() {
        assert_eq!(
            classify_tool("brand_new_tool", None),
            ToolApprovalClass::Unknown
        );
    }

    #[test]
    fn classify_file_read_without_definition() {
        assert_eq!(
            classify_tool("file_read", None),
            ToolApprovalClass::ReadonlyScoped
        );
    }

    #[test]
    fn explicit_metadata_overrides_name_heuristic() {
        // file_read would normally be ReadonlyScoped, but the explicit
        // annotation must win.
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "metadata": { "tool_class": "exec_capable" }
        }));
        assert_eq!(
            classify_tool("file_read", Some(&def)),
            ToolApprovalClass::ExecCapable
        );
    }

    #[test]
    fn explicit_x_tool_class_overrides_name_heuristic() {
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "x-tool-class": "control_plane"
        }));
        assert_eq!(
            classify_tool("file_read", Some(&def)),
            ToolApprovalClass::ControlPlane
        );
    }

    #[test]
    fn unknown_explicit_value_falls_back_to_name() {
        // Unrecognized annotation must not poison the result — fall back
        // to the name-based heuristic.
        let def = def_with_schema(serde_json::json!({
            "type": "object",
            "x-tool-class": "totally_made_up"
        }));
        assert_eq!(
            classify_tool("file_read", Some(&def)),
            ToolApprovalClass::ReadonlyScoped
        );
    }

    #[test]
    fn definition_without_annotation_uses_name() {
        let def = def_with_schema(serde_json::json!({"type": "object"}));
        assert_eq!(
            classify_tool("shell_exec", Some(&def)),
            ToolApprovalClass::ExecCapable
        );
    }

    #[test]
    fn severity_rank_ordering_spot_check() {
        // Mirrors the spec: ReadonlyScoped < ExecCapable < Interactive < Unknown.
        let scoped = classify_tool("file_read", None).severity_rank();
        let exec = classify_tool("shell_exec", None).severity_rank();
        let interactive = classify_tool("totp_request", None).severity_rank();
        let unknown = classify_tool("???", None).severity_rank();
        assert!(scoped < exec);
        assert!(exec < interactive);
        assert!(interactive < unknown);
    }

    #[test]
    fn serde_round_trip_readonly_scoped() {
        let json = serde_json::to_string(&ToolApprovalClass::ReadonlyScoped).unwrap();
        assert_eq!(json, "\"readonly_scoped\"");
        let back: ToolApprovalClass = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ToolApprovalClass::ReadonlyScoped);
    }
}
