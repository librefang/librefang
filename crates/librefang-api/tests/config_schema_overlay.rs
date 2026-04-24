//! Verify that every path key in `x-ui-options` and every `struct_field` /
//! root-level `fields` entry in `x-sections` actually resolves to a real
//! property in the schemars-generated `KernelConfig` schema.
//!
//! Why this exists (round-4 review of #3055): the UI overlay lives in
//! `routes/config.rs` as hand-authored JSON-pointer paths like
//! `/memory/decay_rate`. If a struct field is renamed, the overlay path
//! silently stops matching and the UI loses its min/max/step hint — no
//! error, no warning, just degraded UX. The golden-schema test catches
//! the struct rename but doesn't verify the overlay is still valid.
//! This test closes that gap.
//!
//! Regenerate the golden fixture first if the schema legitimately
//! changed; then run this test to ensure the overlay still matches.

use librefang_api::routes::config as cfg_routes;

fn schema_root() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(librefang_types::config::KernelConfig))
        .expect("schema_for KernelConfig")
}

/// Resolve a `$ref` like `#/definitions/MemoryConfig` against the schema root.
fn resolve_ref<'a>(root: &'a serde_json::Value, r#ref: &str) -> Option<&'a serde_json::Value> {
    let tail = r#ref.strip_prefix("#/")?;
    let mut node = root;
    for seg in tail.split('/') {
        node = node.get(seg)?;
    }
    Some(node)
}

/// Walk one JSON-pointer segment (e.g. `memory`, `decay_rate`) into the
/// current schema node. Unwraps `anyOf`/`oneOf` null unions and `$ref`s
/// so callers don't have to.
fn step_into<'a>(
    root: &'a serde_json::Value,
    node: &'a serde_json::Value,
    segment: &str,
) -> Option<&'a serde_json::Value> {
    // If node is {$ref}, follow it first.
    let mut cur = if let Some(r) = node.get("$ref").and_then(|v| v.as_str()) {
        resolve_ref(root, r)?
    } else {
        node
    };
    // Unwrap the various schemars wrapper shapes so nested properties are
    // reachable:
    //   - allOf: [{$ref}]  — metadata-wrapped required struct
    //   - anyOf|oneOf: [{$ref}, {type:"null"}] — Option<T>
    if let Some(arr) = cur.get("allOf").and_then(|v| v.as_array()) {
        if let Some(first) = arr.first() {
            cur = first;
            if let Some(r) = cur.get("$ref").and_then(|v| v.as_str()) {
                cur = resolve_ref(root, r)?;
            }
        }
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(arr) = cur.get(key).and_then(|v| v.as_array()) {
            if let Some(non_null) = arr.iter().find(|b| {
                b.get("type").and_then(|t| t.as_str()) != Some("null")
                    && (b.get("$ref").is_some() || b.get("properties").is_some())
            }) {
                cur = non_null;
                if let Some(r) = cur.get("$ref").and_then(|v| v.as_str()) {
                    cur = resolve_ref(root, r)?;
                }
                break;
            }
        }
    }
    cur.get("properties").and_then(|p| p.get(segment))
}

/// Walk a full JSON-pointer path (`/memory/decay_rate`) and return the
/// terminal schema node.
fn resolve_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if segments.is_empty() || segments[0].is_empty() {
        return None;
    }
    let mut cur = root.get("properties")?.get(segments[0])?;
    for seg in &segments[1..] {
        cur = step_into(root, cur, seg)?;
    }
    Some(cur)
}

#[test]
fn every_x_ui_options_path_resolves_in_schema() {
    let root = schema_root();
    // Use empty dynamic options — we're only checking path validity.
    let overlay = cfg_routes::ui_options_overlay(Vec::new(), Vec::new());
    let overlay_obj = overlay.as_object().expect("overlay is an object");

    let mut missing: Vec<String> = Vec::new();
    for path in overlay_obj.keys() {
        if resolve_path(&root, path).is_none() {
            missing.push(path.clone());
        }
    }

    assert!(
        missing.is_empty(),
        "x-ui-options paths that do not resolve to a real schema field:\n  {}\n\n\
         Each path is a hand-written JSON pointer in routes/config.rs ui_options_overlay(). \
         When a struct field is renamed or removed the pointer silently stops matching \
         and the UI loses its min/max/step/select hint. Either update the path or remove \
         the overlay entry.",
        missing.join("\n  ")
    );
}

#[test]
fn every_x_sections_struct_field_exists_on_kernel_config() {
    let root = schema_root();
    let top_properties = root
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("KernelConfig properties block");
    let sections = cfg_routes::ui_sections_overlay();
    let sections_arr = sections.as_array().expect("x-sections is an array");

    let mut missing: Vec<String> = Vec::new();
    for desc in sections_arr {
        if let Some(sf) = desc.get("struct_field").and_then(|v| v.as_str()) {
            if !top_properties.contains_key(sf) {
                missing.push(format!(
                    "section {}: struct_field={}",
                    desc.get("key").and_then(|v| v.as_str()).unwrap_or("?"),
                    sf
                ));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "x-sections entries referencing non-existent KernelConfig fields:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn every_root_level_field_exists_on_kernel_config() {
    let root = schema_root();
    let top_properties = root
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("KernelConfig properties block");
    let sections = cfg_routes::ui_sections_overlay();
    let sections_arr = sections.as_array().unwrap();

    let mut missing: Vec<String> = Vec::new();
    for desc in sections_arr {
        if desc.get("root_level").and_then(|v| v.as_bool()) == Some(true) {
            if let Some(fields) = desc.get("fields").and_then(|v| v.as_array()) {
                for f in fields {
                    if let Some(name) = f.as_str() {
                        if !top_properties.contains_key(name) {
                            missing.push(format!(
                                "root_level section {}: field={}",
                                desc.get("key").and_then(|v| v.as_str()).unwrap_or("?"),
                                name
                            ));
                        }
                    }
                }
            }
        }
    }

    assert!(
        missing.is_empty(),
        "root-level section fields that do not exist on KernelConfig:\n  {}",
        missing.join("\n  ")
    );
}
