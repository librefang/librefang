//! Golden-file regression guard for the schemars-derived `KernelConfig`
//! JSON Schema. Any change to the generated schema (new field, renamed
//! field, changed type, different default) fails this test and forces a
//! reviewer to regenerate the fixture and eyeball the diff.
//!
//! Regenerate:
//!     cargo test -p librefang-api --test config_schema_golden \
//!         -- --ignored regenerate_golden --nocapture
//!
//! Rationale (issue #3055 review P2): before this PR the hand-written
//! `config_schema()` gave implicit protection — struct-field renames broke
//! the UI visibly. Now the schema is auto-derived, so a rename can
//! silently reshape the wire format. The golden fixture makes the
//! reshape land in a reviewable diff.
//!
//! Only the draft-07 `properties` / `definitions` block is compared. The
//! runtime `x-sections` / `x-ui-options` overlays live in
//! `routes/config.rs::config_schema()` and are tested separately.

use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("kernel_config_schema.golden.json")
}

fn generate_schema_json() -> String {
    let schema = schemars::schema_for!(librefang_types::config::KernelConfig);
    // Deterministic output: pretty-print via serde_json (maps are BTreeMap in
    // the schemars output, so ordering is stable across runs).
    serde_json::to_string_pretty(&schema).expect("serialize schema")
}

#[test]
fn kernel_config_schema_matches_golden_fixture() {
    let actual = generate_schema_json();
    let expected = std::fs::read_to_string(fixture_path())
        .expect("read fixtures/kernel_config_schema.golden.json — regenerate with `--ignored regenerate_golden`");

    // Normalize trailing newline differences so `cat >` and editor-written
    // files compare equal.
    if actual.trim() != expected.trim() {
        let actual_lines = actual.lines().count();
        let expected_lines = expected.lines().count();
        panic!(
            "KernelConfig schema drifted from golden fixture.\n\
             actual: {actual_lines} lines / {} bytes\n\
             expected: {expected_lines} lines / {} bytes\n\
             \n\
             Review the schema diff. If the change is intentional, regenerate:\n\
             \n\
             \tcargo test -p librefang-api --test config_schema_golden \\\n\
             \t\t-- --ignored regenerate_golden --nocapture\n",
            actual.len(),
            expected.len()
        );
    }
}

/// Regenerate the fixture. Gated behind `--ignored` so it doesn't run by
/// default and isn't a silent self-healing test.
#[test]
#[ignore = "run manually with --ignored to regenerate the golden fixture"]
fn regenerate_golden() {
    let schema = generate_schema_json();
    let path = fixture_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixtures dir");
    }
    // Ensure a single trailing newline.
    let content = if schema.ends_with('\n') {
        schema
    } else {
        format!("{schema}\n")
    };
    std::fs::write(&path, &content).expect("write golden fixture");
    println!(
        "wrote golden fixture: {} ({} bytes)",
        path.display(),
        content.len()
    );
}
