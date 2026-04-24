//! PoC: dump schemars-generated draft-07 schema for three representative
//! config types so we can eyeball edge-case behavior without writing a
//! dashboard-facing endpoint.
//!
//! Run: `cargo test -p librefang-types --test schemars_poc -- --nocapture`

use librefang_types::config::{BudgetConfig, ResponseFormat, VaultConfig};

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
fn dump_response_format_schema() {
    // Tagged enum with a variant carrying serde_json::Value — major risk point.
    let schema = schemars::schema_for!(ResponseFormat);
    let json = serde_json::to_string_pretty(&schema).unwrap();
    println!("\n=== ResponseFormat ({} bytes) ===\n{json}", json.len());
}
