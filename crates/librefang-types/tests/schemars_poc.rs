//! PoC: dump schemars-generated draft-07 schema for three representative
//! config types so we can eyeball edge-case behavior without writing a
//! dashboard-facing endpoint.
//!
//! Run: `cargo test -p librefang-types --test schemars_poc -- --nocapture`

use librefang_types::config::{BudgetConfig, KernelConfig, ResponseFormat, VaultConfig};

#[test]
fn dump_budget_config_schema() {
    let schema = schemars::schema_for!(BudgetConfig);
    let json = serde_json::to_string_pretty(&schema).unwrap();
    println!("\n=== BudgetConfig ({} bytes) ===\n{json}", json.len());
}

#[test]
fn dump_vault_config_schema() {
    // Contains Option<PathBuf> — tests how schemars renders filesystem paths.
    let schema = schemars::schema_for!(VaultConfig);
    let json = serde_json::to_string_pretty(&schema).unwrap();
    println!("\n=== VaultConfig ({} bytes) ===\n{json}", json.len());
}

#[test]
fn full_kernel_config_schema_generates() {
    // End-to-end sanity: the full KernelConfig schema must generate and
    // produce well-formed JSON. Size / field count give us a sense of the
    // overlay surface we still need to add to match the hand-written schema.
    let schema = schemars::schema_for!(KernelConfig);
    let val = serde_json::to_value(&schema).unwrap();
    let top_props = val
        .pointer("/properties")
        .and_then(|v| v.as_object())
        .map(|m| m.len())
        .unwrap_or(0);
    let definitions = val
        .pointer("/definitions")
        .and_then(|v| v.as_object())
        .map(|m| m.len())
        .unwrap_or(0);
    let json = serde_json::to_string(&schema).unwrap();
    println!(
        "\n=== KernelConfig generated OK: top_props={top_props} definitions={definitions} bytes={}",
        json.len()
    );
    assert!(
        top_props > 50,
        "expected KernelConfig to have >50 top-level fields"
    );
    assert!(definitions > 50, "expected many nested definitions");
}

#[test]
fn dump_response_format_schema() {
    // Tagged enum with a variant carrying serde_json::Value — major risk point.
    let schema = schemars::schema_for!(ResponseFormat);
    let json = serde_json::to_string_pretty(&schema).unwrap();
    println!("\n=== ResponseFormat ({} bytes) ===\n{json}", json.len());
}
