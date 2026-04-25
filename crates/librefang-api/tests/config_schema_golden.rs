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
    // Normalize platform-specific values that `schemars` embeds in `default`
    // fields. `KernelConfig::default()` computes `home_dir` / `data_dir` from
    // `dirs::home_dir()` which resolves to `/Users/<me>/…` on macOS,
    // `/home/runner/…` on the Ubuntu CI runner, `C:\Users\runneradmin\…` on
    // Windows. The schema's *structure* is what we want the golden to guard;
    // user-specific absolute paths would make the fixture machine-local.
    let mut value = serde_json::to_value(&schema).expect("schema → Value");
    normalize_volatile_defaults(&mut value);
    // Deterministic output: pretty-print via serde_json (maps are BTreeMap in
    // the schemars output, so ordering is stable across runs).
    serde_json::to_string_pretty(&value).expect("serialize schema")
}

/// Walk the schema tree and replace any `"default"` string value that looks
/// like an absolute path with a placeholder, so home-directory differences
/// between dev machines and CI runners don't fail the fixture.
fn normalize_volatile_defaults(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(def) = map.get_mut("default") {
                normalize_path_like(def);
            }
            for (_, v) in map.iter_mut() {
                normalize_volatile_defaults(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                normalize_volatile_defaults(v);
            }
        }
        _ => {}
    }
}

fn normalize_path_like(value: &mut serde_json::Value) {
    // Heuristic: looks like an absolute path and contains `.librefang`
    // (or a home-directory marker). Replace with a stable placeholder.
    match value {
        serde_json::Value::String(s)
            if (s.starts_with('/') || s.contains(":\\") || s.contains(":/"))
                && (s.contains(".librefang") || s.contains("/home/") || s.contains("/Users/")) =>
        {
            *s = "<home>/.librefang".into();
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                normalize_path_like(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                normalize_path_like(v);
            }
        }
        _ => {}
    }
}

#[test]
fn kernel_config_schema_matches_golden_fixture() {
    let actual = generate_schema_json();
    let expected = std::fs::read_to_string(fixture_path())
        .expect("read fixtures/kernel_config_schema.golden.json — regenerate with `--ignored regenerate_golden`");

    // Normalize line endings (Windows git checkout may produce CRLF even
    // when the file is committed with LF) and trailing whitespace.
    fn canon(s: &str) -> String {
        s.replace("\r\n", "\n").trim().to_string()
    }
    if canon(&actual) != canon(&expected) {
        let actual_lines = actual.lines().count();
        let expected_lines = expected.lines().count();
        // Embed the actual schema as base64 in the panic message so CI logs
        // expose the full drifted schema. Reviewers can decode the marker
        // block to recover the new fixture without re-running the test
        // locally — useful when CI fails on a platform we don't have at hand.
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(actual.as_bytes());
        panic!(
            "KernelConfig schema drifted from golden fixture.\n\
             actual: {actual_lines} lines / {} bytes\n\
             expected: {expected_lines} lines / {} bytes\n\
             \n\
             Review the schema diff. If the change is intentional, regenerate:\n\
             \n\
             \tcargo test -p librefang-api --test config_schema_golden \\\n\
             \t\t-- --ignored regenerate_golden --nocapture\n\
             \n\
             ===BEGIN_ACTUAL_SCHEMA_B64===\n\
             {b64}\n\
             ===END_ACTUAL_SCHEMA_B64===\n",
            actual.len(),
            expected.len(),
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
