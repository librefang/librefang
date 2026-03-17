//! Validates the auto-generated OpenAPI spec and writes it to `openapi.json`.

use librefang_api::openapi::ApiDoc;
use utoipa::OpenApi;

#[test]
fn generate_openapi_json() {
    let doc = ApiDoc::openapi();
    let json = doc
        .to_pretty_json()
        .expect("Failed to serialize OpenAPI spec");

    // Basic sanity checks
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(
        parsed["openapi"].as_str().is_some(),
        "missing openapi version"
    );
    assert!(parsed["info"]["title"].as_str().is_some(), "missing title");

    let paths = parsed["paths"].as_object().expect("missing paths");
    assert!(
        paths.len() > 100,
        "expected 100+ paths, got {}",
        paths.len()
    );

    // Write to repo root for SDK codegen / CI consumption
    // Write to repo root (two levels up from crates/librefang-api/)
    let out_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openapi.json");
    std::fs::write(&out_path, &json).expect("Failed to write openapi.json");
    eprintln!("Wrote {} paths to {}", paths.len(), out_path.display());
}
