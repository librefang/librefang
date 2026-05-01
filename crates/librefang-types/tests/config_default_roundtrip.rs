//! Regression test for issue #3404.
//!
//! When a new field is added to `KernelConfig` (or a nested config struct)
//! with `#[serde(default)]` but the developer forgets to populate it in the
//! manual `Default` impl, deserialization succeeds with `T::default()` while
//! the in-process `KernelConfig::default()` returns whatever the manual impl
//! produces. The two diverge silently — empty TOML round-trips, but the
//! in-memory default carries different values. The schemars-based golden
//! schema test does not catch this because schemars reads the
//! `#[serde(default)]` attribute, not the `Default` impl body.
//!
//! This test asserts:
//!   1. `T::default()` equals what serde produces from an empty TOML document.
//!   2. `T::default()` round-trips losslessly through TOML serialization.
//!
//! Equality is checked by comparing the TOML serialization of both sides
//! rather than deriving `PartialEq` on `KernelConfig` (which would force
//! `PartialEq` onto every nested config type — see issue #3404 caveat 1).

use librefang_types::config::{BudgetConfig, KernelConfig, QueueConfig, SessionConfig};
use serde::Serialize;

fn assert_default_roundtrip<T>(label: &str)
where
    T: Default + Serialize + for<'de> serde::Deserialize<'de>,
{
    let from_default = T::default();
    let default_toml = toml::to_string(&from_default)
        .unwrap_or_else(|e| panic!("{label}: serialize default failed: {e}"));

    // Empty TOML must deserialize to exactly the same value as Default::default().
    // This is what catches a `#[serde(default)]` field whose corresponding line
    // is missing from the manual `Default` impl: serde fills it with
    // `Field::default()` while our manual impl produces something else, and the
    // two TOML strings will differ.
    let from_empty: T = toml::from_str("")
        .unwrap_or_else(|e| panic!("{label}: deserialize empty TOML failed: {e}"));
    let empty_toml = toml::to_string(&from_empty)
        .unwrap_or_else(|e| panic!("{label}: serialize from-empty failed: {e}"));
    assert_eq!(
        default_toml, empty_toml,
        "{label}::default() must equal what serde produces from an empty TOML \
         document. A field is likely declared with `#[serde(default)]` but \
         missing from the manual `Default` impl (or vice versa)."
    );

    // Round-trip the serialized default and assert idempotency.
    let from_roundtrip: T = toml::from_str(&default_toml)
        .unwrap_or_else(|e| panic!("{label}: deserialize roundtrip failed: {e}"));
    let roundtrip_toml = toml::to_string(&from_roundtrip)
        .unwrap_or_else(|e| panic!("{label}: serialize roundtrip failed: {e}"));
    assert_eq!(
        default_toml, roundtrip_toml,
        "{label}::default() must round-trip through TOML serialization."
    );
}

#[test]
fn kernel_config_default_roundtrips_through_toml() {
    // KernelConfig::default() pulls in machine-specific paths via
    // `librefang_home_dir()`, but those paths are deterministic within a
    // single process run, and empty-TOML deserialization re-invokes the same
    // `KernelConfig::default()` (because the struct is annotated with
    // `#[serde(default)]`). Both sides therefore observe identical paths —
    // no normalization needed for paths.
    //
    // `config_version` is the one field where the two sources legitimately
    // diverge and must be normalized before comparison:
    //   - `KernelConfig::default()` returns the current `CONFIG_VERSION`
    //     (currently `2`) — see `crates/librefang-types/src/config/types.rs`
    //     where the manual `Default` impl sets `config_version: CONFIG_VERSION`.
    //     Fresh in-memory configs need no migration, so they are stamped with
    //     the latest version.
    //   - Serde-empty deserialization fills the field via
    //     `default_config_version()` which returns `1` — see
    //     `crates/librefang-types/src/config/version.rs`. The `1` is an
    //     intentional migration tripwire: a legacy on-disk TOML that omits
    //     `config_version` is by definition pre-versioning (v1), and
    //     `run_migrations` will lift it forward to `CONFIG_VERSION`.
    //
    // We normalize `config_version` on the from-empty / from-roundtrip sides
    // so the test still asserts that EVERY OTHER field round-trips exactly.
    // That is the property that catches the bug class issue #3404 describes
    // (a new `#[serde(default)]` field forgotten in the manual `Default`
    // impl) — the deliberate v1 sentinel is orthogonal to that bug class
    // and would otherwise mask all other field comparisons behind a single
    // expected mismatch.
    let from_default = KernelConfig::default();
    let mut from_empty: KernelConfig =
        toml::from_str("").expect("KernelConfig: deserialize empty TOML failed");
    from_empty.config_version = from_default.config_version;

    let default_toml =
        toml::to_string(&from_default).expect("KernelConfig: serialize default failed");
    let empty_toml =
        toml::to_string(&from_empty).expect("KernelConfig: serialize from-empty failed");
    assert_eq!(
        default_toml, empty_toml,
        "KernelConfig::default() must equal what serde produces from an \
         empty TOML document (after normalizing the intentional \
         config_version divergence). A field is likely declared with \
         `#[serde(default)]` but missing from the manual `Default` impl \
         (or vice versa)."
    );

    let mut from_roundtrip: KernelConfig =
        toml::from_str(&default_toml).expect("KernelConfig: deserialize roundtrip failed");
    from_roundtrip.config_version = from_default.config_version;
    let roundtrip_toml =
        toml::to_string(&from_roundtrip).expect("KernelConfig: serialize roundtrip failed");
    assert_eq!(
        default_toml, roundtrip_toml,
        "KernelConfig::default() must round-trip through TOML serialization \
         (after normalizing config_version)."
    );
}

#[test]
fn queue_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<QueueConfig>("QueueConfig");
}

#[test]
fn budget_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<BudgetConfig>("BudgetConfig");
}

#[test]
fn session_config_default_roundtrips_through_toml() {
    assert_default_roundtrip::<SessionConfig>("SessionConfig");
}
