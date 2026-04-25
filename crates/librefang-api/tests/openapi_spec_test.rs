//! Validates the auto-generated OpenAPI spec and writes it to `openapi.json`.

use librefang_api::openapi::ApiDoc;
use std::collections::HashSet;
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

#[test]
fn openapi_paths_are_mapped_to_integration_coverage() {
    let doc = ApiDoc::openapi();
    let json = doc
        .to_pretty_json()
        .expect("Failed to serialize OpenAPI spec");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let openapi_paths = parsed["paths"].as_object().expect("missing paths");

    let matrix: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/integration_matrix.json"))
            .expect("integration coverage matrix must be valid JSON");
    let entries = matrix["paths"]
        .as_array()
        .expect("integration coverage matrix must contain paths array");

    let mut matrix_paths = HashSet::new();
    for entry in entries {
        let path = entry["path"]
            .as_str()
            .expect("matrix entry must contain path");
        assert!(matrix_paths.insert(path), "duplicate matrix path: {path}");

        let status = entry["status"]
            .as_str()
            .expect("matrix entry must contain status");
        match status {
            "covered" => {
                assert!(
                    entry["scenario"].as_str().is_some(),
                    "covered path {path} must name a scenario"
                );
                assert!(
                    entry["proof"].as_str().is_some(),
                    "covered path {path} must name proof"
                );
            }
            "exempt" => {
                assert!(
                    entry["reason"].as_str().is_some(),
                    "exempt path {path} must document reason"
                );
                assert!(
                    entry["owner"].as_str().is_some(),
                    "exempt path {path} must document owner"
                );
            }
            other => panic!("path {path} has invalid matrix status {other}"),
        }
    }

    let openapi_set: HashSet<&str> = openapi_paths.keys().map(String::as_str).collect();
    assert_eq!(
        openapi_set, matrix_paths,
        "OpenAPI paths and integration coverage matrix paths must match exactly"
    );
}
