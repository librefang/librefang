//! Readers for the JSON shapes the daemon's HTTP API returns.
//!
//! List endpoints answer in one of three envelopes: the canonical paginated `{items,total,offset,limit}`, a named collection such as `{"triggers":[...]}` or `{"models":[...]}`, or (for a few older routes) a bare array.
//! Calling `body.as_array()` on an envelope yields `None`, and the `unwrap_or_default()` that usually follows turns a successful 200 into an empty list.
//! [`list_items`] accepts all three so a caller cannot get that wrong again.

use serde_json::Value;

/// The rows of a list response: `body[key]` when `key` is given and present, else `body.items`, else `body` itself when it is a bare array.
///
/// Returns an empty slice for any other shape, which callers render as an empty list.
pub(crate) fn list_items<'a>(body: &'a Value, key: Option<&str>) -> &'a [Value] {
    key.and_then(|k| body.get(k))
        .and_then(Value::as_array)
        .or_else(|| body.get("items").and_then(Value::as_array))
        .or_else(|| body.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// `provider/model` label for an agent, from either the detail shape of `GET /api/agents/{id}` (nested `model.provider` / `model.model`) or the flat `model_provider` / `model_name` keys of the list endpoint.
pub(crate) fn agent_model_label(body: &Value) -> String {
    let provider = body["model"]["provider"]
        .as_str()
        .or_else(|| body["model_provider"].as_str())
        .unwrap_or("?");
    let model = body["model"]["model"]
        .as_str()
        .or_else(|| body["model_name"].as_str())
        .unwrap_or("?");
    format!("{provider}/{model}")
}

/// Render a value that is usually a string but may arrive as a structured value (a trigger pattern serialised from an enum, a numeric protocol version).
///
/// A string is returned as-is, `null` / missing as `fallback`, and anything else as compact JSON.
pub(crate) fn display_value(value: &Value, fallback: &str) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => fallback.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_items_reads_paginated_envelope() {
        let body =
            json!({"items": [{"id": "a"}, {"id": "b"}], "total": 2, "offset": 0, "limit": null});
        assert_eq!(list_items(&body, None).len(), 2);
    }

    #[test]
    fn list_items_reads_named_key_before_items() {
        let body = json!({"triggers": [{"id": "t"}], "total": 1});
        assert_eq!(list_items(&body, Some("triggers"))[0]["id"], "t");
    }

    #[test]
    fn list_items_accepts_bare_array() {
        let body = json!([{"id": "x"}]);
        assert_eq!(list_items(&body, Some("models")).len(), 1);
    }

    #[test]
    fn list_items_is_empty_for_other_shapes() {
        assert!(list_items(&json!({"error": "nope"}), None).is_empty());
    }

    #[test]
    fn agent_model_label_reads_nested_detail_shape() {
        let body = json!({"id": "x", "model": {"provider": "openai", "model": "gpt-4o"}});
        assert_eq!(agent_model_label(&body), "openai/gpt-4o");
    }

    #[test]
    fn agent_model_label_reads_flat_list_shape() {
        let body = json!({"model_provider": "groq", "model_name": "llama"});
        assert_eq!(agent_model_label(&body), "groq/llama");
    }

    #[test]
    fn display_value_renders_structured_values() {
        assert_eq!(display_value(&json!("lifecycle"), "?"), "lifecycle");
        assert_eq!(
            display_value(&json!({"agent_spawned": {"name_pattern": "*"}}), "?"),
            r#"{"agent_spawned":{"name_pattern":"*"}}"#
        );
        assert_eq!(display_value(&json!(1), ""), "1");
        assert_eq!(display_value(&Value::Null, "?"), "?");
    }
}
