//! Golden list of the `config.fld_*` label keys the dashboard `ConfigPage` can look up (#8425).
//!
//! `ConfigPage.tsx` composes every field label from a template literal — `config.fld_${sKey}_${fieldKey}`, falling back to `config.fld_${fieldKey}` — so the dashboard's static dead-key scan (`en-locale-coverage.test.ts`) cannot tell a live label from one whose field left the schema two refactors ago.
//! The set of names those templates can produce is not static, but it is fully determined by the `GET /api/config/schema` response: the `x-sections` overlay plus the schemars-derived `properties` / `definitions`.
//! This test fetches that response from the real handler, derives the keys the same way `resolveSectionFields` in `ConfigPage.tsx` does, and compares them with a sorted fixture the TypeScript test reads.
//!
//! Regenerate after an intentional schema or section change:
//!     cargo test -p librefang-api --test config_field_label_keys_golden -- --ignored regenerate_config_field_label_keys --nocapture
//!
//! The derivation deliberately covers every `x-sections` entry, including ones no `CATEGORY_SECTIONS` tab lists yet, and does not apply the page's dedicated-editor filter.
//! Both of those are page-side presentation choices; a label for a field the schema exposes is kept live rather than deleted and re-translated the day the section gets a tab.

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use librefang_testing::TestAppState;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;
use tower::ServiceExt;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("dashboard")
        .join("src")
        .join("lib")
        .join("__tests__")
        .join("fixtures")
        .join("config_field_label_keys.golden.json")
}

/// The schema exactly as the dashboard receives it.
async fn fetch_config_schema() -> Value {
    let test = TestAppState::new();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/config/schema")
        .body(Body::empty())
        .expect("request");
    let resp = test.router().oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 << 20)
        .await
        .expect("read schema body");
    serde_json::from_slice(&bytes).expect("schema is JSON")
}

/// Mirror of `resolveRef` in `dashboard/src/api.ts`.
fn resolve_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let tail = reference.strip_prefix("#/")?;
    tail.split('/').try_fold(root, |node, seg| node.get(seg))
}

/// `a.type && a.type !== "null"` from `ConfigPage.tsx`: any present `type` except the string `"null"` (an array of types is truthy and not equal to `"null"`).
fn has_non_null_type(node: &Value) -> bool {
    match node.get("type") {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty() && s != "null",
        Some(_) => true,
    }
}

/// Mirror of the field enumeration in `resolveSectionFields` (`dashboard/src/pages/ConfigPage.tsx`).
/// Only the field keys matter here, so the per-field render resolution and `SECTION_FIELD_ORDER` (which reorders but never drops) are not reproduced.
fn section_field_keys(root: &Value, desc: &Value) -> Vec<String> {
    let key = desc.get("key").and_then(Value::as_str).unwrap_or_default();
    let properties = root.get("properties");

    if desc.get("root_level").and_then(Value::as_bool) == Some(true) {
        if let Some(fields) = desc.get("fields").and_then(Value::as_array) {
            return fields
                .iter()
                .filter_map(Value::as_str)
                .filter(|f| properties.and_then(|p| p.get(*f)).is_some())
                .map(str::to_string)
                .collect();
        }
    }

    let Some(struct_field) = desc.get("struct_field").and_then(Value::as_str) else {
        return Vec::new();
    };
    let mut target = properties.and_then(|p| p.get(struct_field));

    // Peel the schemars wrapper shapes the same way the page does: `allOf` first, then an `anyOf` / `oneOf` non-null branch.
    if let Some(t) = target.filter(|t| t.get("$ref").is_none()) {
        if let Some(first) = t
            .get("allOf")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        {
            target = Some(first);
        } else if let Some(branches) = t.get("anyOf").and_then(Value::as_array) {
            if let Some(b) = branches
                .iter()
                .find(|a| a.get("$ref").is_some() || has_non_null_type(a))
            {
                target = Some(b);
            }
        } else if let Some(branches) = t.get("oneOf").and_then(Value::as_array) {
            if let Some(b) = branches
                .iter()
                .find(|a| a.get("$ref").is_some() || has_non_null_type(a))
            {
                target = Some(b);
            }
        }
    }
    if let Some(reference) = target.and_then(|t| t.get("$ref")).and_then(Value::as_str) {
        target = resolve_ref(root, reference);
    }
    let Some(target) = target else {
        return Vec::new();
    };

    let target_type = target.get("type").and_then(Value::as_str);

    // Collection-typed sections render as one editor whose synthetic field key is the section key.
    if target_type == Some("object")
        && target
            .get("additionalProperties")
            .is_some_and(Value::is_object)
    {
        return vec![key.to_string()];
    }
    if target_type == Some("array") {
        if let Some(items) = target.get("items").filter(|i| !i.is_null()) {
            if items.get("$ref").is_some()
                || items.get("type").and_then(Value::as_str) == Some("object")
            {
                return vec![key.to_string()];
            }
        }
    }

    match target.get("properties").and_then(Value::as_object) {
        Some(props) => props.keys().cloned().collect(),
        None => Vec::new(),
    }
}

/// Every name the section-qualified (`config.fld_<section>_<field>`) and bare (`config.fld_<field>`) label lookups can resolve to, sorted and deduplicated.
fn derive_label_keys(schema: &Value) -> BTreeSet<String> {
    let sections = schema
        .get("x-sections")
        .and_then(Value::as_array)
        .expect("schema carries an x-sections array");
    let mut keys = BTreeSet::new();
    for desc in sections {
        let section = desc
            .get("key")
            .and_then(Value::as_str)
            .expect("section key");
        for field in section_field_keys(schema, desc) {
            keys.insert(format!("config.fld_{section}_{field}"));
            keys.insert(format!("config.fld_{field}"));
        }
    }
    keys
}

fn render_fixture(keys: &BTreeSet<String>) -> String {
    let mut out = serde_json::to_string_pretty(keys).expect("serialize keys");
    out.push('\n');
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn config_field_label_keys_match_golden_fixture() {
    let schema = fetch_config_schema().await;
    let actual = derive_label_keys(&schema);

    // A derivation that silently returned nothing would make every `config.fld_*` locale key look dead, or — if the fixture were regenerated from it — make the guard vacuous.
    assert!(
        actual.contains("config.fld_default_model_provider")
            && actual.contains("config.fld_api_listen"),
        "derivation lost known ConfigPage fields; does it still mirror resolveSectionFields?"
    );

    let expected_raw = std::fs::read_to_string(fixture_path()).expect(
        "read config_field_label_keys.golden.json — regenerate with `--ignored regenerate_config_field_label_keys`",
    );
    let expected: BTreeSet<String> =
        serde_json::from_str(&expected_raw).expect("fixture is a JSON string array");

    let added: Vec<&String> = actual.difference(&expected).collect();
    let removed: Vec<&String> = expected.difference(&actual).collect();
    assert!(
        added.is_empty() && removed.is_empty(),
        "ConfigPage label keys drifted from the golden fixture.\n\
         newly renderable: {added:?}\n\
         no longer renderable: {removed:?}\n\
         \n\
         If the schema or x-sections change is intentional, regenerate:\n\
         \tcargo test -p librefang-api --test config_field_label_keys_golden -- --ignored regenerate_config_field_label_keys --nocapture\n\
         then delete any `config.fld_*` locale key the dashboard dead-key test reports."
    );
    assert_eq!(
        expected_raw.replace("\r\n", "\n"),
        render_fixture(&expected),
        "fixture is not in canonical sorted form; regenerate it"
    );
}

/// Rewrite the fixture. Gated behind `--ignored` so it never self-heals in CI.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "run manually with --ignored to regenerate the golden fixture"]
async fn regenerate_config_field_label_keys() {
    let schema = fetch_config_schema().await;
    let content = render_fixture(&derive_label_keys(&schema));
    let path = fixture_path();
    std::fs::create_dir_all(path.parent().expect("fixture dir")).expect("create fixtures dir");
    std::fs::write(&path, &content).expect("write golden fixture");
    println!("wrote {} ({} bytes)", path.display(), content.len());
}
