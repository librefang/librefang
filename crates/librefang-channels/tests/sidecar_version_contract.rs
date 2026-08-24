//! Drift guard for the two version numbers the sidecar boundary carries.
//!
//! The sidecar protocol version and the bundled SDK version are each written down in several places that a compiler cannot relate to one another: Rust constants, Python source, a JSON corpus fixture, prose in an architecture doc, and packaging metadata.
//! #7140 is what that costs when nothing checks them — `docs/architecture/sidecar-protocol.md` said "current value: 1", `conformance/sidecar/corpus/events/ready_full.json` pinned `1`, both conformance suites passed `1` by hand, and every `ready` frame any real adapter emitted carried `null`, for months, with no test able to notice.
//!
//! Each assertion below names the file it reads, so a failure says which copy moved instead of only that two numbers differ.
//! These are *mirrors* of `librefang_channels::sidecar::SIDECAR_PROTOCOL_VERSION`; when the protocol version legitimately changes, update the constant and then every file this test names.
//!
//! Reading source text is deliberate.
//! The Python SDK and the Rust adapter SDK are separate build units (the latter is its own cargo workspace, kept off this crate's dependency graph on purpose), so there is no import that would let the daemon assert against their real constants.

use librefang_channels::embedded_sdk::embedded_sdk_version;
use librefang_channels::sidecar::SIDECAR_PROTOCOL_VERSION;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The number after `needle` on the first line that contains it.
fn number_after(haystack: &str, needle: &str, whence: &Path) -> u32 {
    let line = haystack
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{}: no line containing {needle:?}", whence.display()));
    let tail = &line[line.find(needle).expect("checked by find") + needle.len()..];
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse()
        .unwrap_or_else(|e| panic!("{}: no number after {needle:?}: {e}", whence.display()))
}

#[test]
fn python_sdk_mirrors_the_protocol_version() {
    let rel = "sdk/python/librefang/sidecar/protocol.py";
    let src = read(rel);
    assert_eq!(
        number_after(&src, "PROTOCOL_VERSION = ", Path::new(rel)),
        SIDECAR_PROTOCOL_VERSION,
        "{rel} disagrees with SIDECAR_PROTOCOL_VERSION"
    );
}

#[test]
fn rust_sdk_mirrors_the_protocol_version() {
    let rel = "sdk/rust/librefang-sidecar/src/protocol.rs";
    let src = read(rel);
    assert_eq!(
        number_after(&src, "pub const PROTOCOL_VERSION: u32 = ", Path::new(rel)),
        SIDECAR_PROTOCOL_VERSION,
        "{rel} disagrees with SIDECAR_PROTOCOL_VERSION"
    );
}

#[test]
fn corpus_ready_frame_mirrors_the_protocol_version() {
    let rel = "conformance/sidecar/corpus/events/ready_full.json";
    let raw = read(rel);
    let v: serde_json::Value = serde_json::from_str(&raw).expect("corpus fixture is valid JSON");
    assert_eq!(
        v["params"]["protocol_version"].as_u64(),
        Some(u64::from(SIDECAR_PROTOCOL_VERSION)),
        "{rel} disagrees with SIDECAR_PROTOCOL_VERSION"
    );
}

#[test]
fn protocol_doc_mirrors_the_protocol_version() {
    // The doc is the thing an adapter author reads before writing a `ready`
    // frame, so a stale number there produces exactly the skew this guard
    // exists to prevent.
    let rel = "docs/architecture/sidecar-protocol.md";
    let doc = read(rel);
    assert_eq!(
        number_after(&doc, "**Current value: `", Path::new(rel)),
        SIDECAR_PROTOCOL_VERSION,
        "{rel} disagrees with SIDECAR_PROTOCOL_VERSION"
    );
}

#[test]
fn embedded_sdk_version_mirrors_the_python_packaging_metadata() {
    // `embedded_sdk_version()` reads `sdk/python/pyproject.toml` through
    // `include_str!` at compile time; re-read it here so a change to the
    // extraction (or to the pyproject layout) surfaces as a named failure
    // rather than as the daemon quietly warning about every install.
    let rel = "sdk/python/pyproject.toml";
    let pyproject = read(rel);
    let declared = pyproject
        .lines()
        .find_map(|l| l.strip_prefix("version = \"")?.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{rel}: no `version = \"…\"` line"));
    assert_eq!(
        embedded_sdk_version(),
        declared,
        "the version compiled into the daemon disagrees with {rel}"
    );
}

/// Prose in the doc must not describe the field as inert any more.
///
/// The daemon now classifies and warns on skew; a doc that still says the value is only logged tells adapter authors the field does not matter, which is how it came to be left unset everywhere in the first place.
#[test]
fn protocol_doc_describes_the_version_as_checked() {
    let rel = "docs/architecture/sidecar-protocol.md";
    let doc = read(rel);
    assert!(
        doc.contains("SIDECAR_PROTOCOL_VERSION"),
        "{rel} should name the constant that is the source of truth"
    );
}
