//! OpenAPI path-coverage reflection test (refs the "openapi paths
//! incomplete" finding).
//!
//! Catches the *inverse* of `dead_route_audit_test.rs`. The dead-route
//! audit asserts that every path **in the spec** is wired into the axum
//! router. This test asserts that every handler **annotated with
//! `#[utoipa::path(...)]` in the crate source** is actually referenced
//! from `ApiDoc::paths(...)` in `openapi.rs`, and therefore present in
//! the generated `ApiDoc::openapi()` surface.
//!
//! Why this is needed: utoipa only includes a handler in the document if
//! it is explicitly listed inside the `paths(...)` argument of the
//! `#[derive(OpenApi)]` macro. Adding `#[utoipa::path]` to a new handler
//! does **not** auto-register it. Forgetting the `paths(...)` entry
//! silently drops the endpoint from `openapi.json` and from every
//! generated SDK, with no compile error and no runtime symptom — exactly
//! the drift this finding documented (the whole MCP-OAuth flow plus 80+
//! other handlers were missing).
//!
//! Strategy (no extra dependency — the doc's `inventory`-based sketch
//! would have required a new crate):
//!   1. Walk every `*.rs` file under `crates/librefang-api/src/` at test
//!      time via `CARGO_MANIFEST_DIR` (same trick as
//!      `openapi_spec_test.rs` / `config_schema_golden.rs`).
//!   2. Parse each `#[utoipa::path(...)]` attribute, extracting the HTTP
//!      method and `path = "..."` string. This is the source-of-truth
//!      set of `(method, path)` pairs the developer *intended* to expose.
//!   3. Build the same `(method, path)` set from `ApiDoc::openapi()`.
//!   4. Assert the source set is a subset of the documented set. Any
//!      difference is a handler that carries `#[utoipa::path]` but was
//!      never added to `paths(...)`.

use librefang_api::openapi::ApiDoc;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use utoipa::OpenApi;

/// HTTP methods utoipa recognises in a `#[utoipa::path(...)]` attribute.
const HTTP_METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

/// Recursively collect every `.rs` file under `dir`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Extract the substring between the outermost balanced parentheses that
/// follow `#[utoipa::path` starting at byte index `attr_start`. Returns
/// the inner text (without the surrounding parens) and the byte index just
/// past the closing paren, or `None` if no balanced block is found.
fn balanced_paren_block(bytes: &[u8], open_paren: usize) -> Option<(String, usize)> {
    debug_assert_eq!(bytes[open_paren], b'(');
    let mut depth = 0usize;
    let mut i = open_paren;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    let inner = std::str::from_utf8(&bytes[open_paren + 1..i]).ok()?;
                    return Some((inner.to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Pull the `path = "..."` literal out of a `#[utoipa::path(...)]` body.
fn extract_path(block: &str) -> Option<String> {
    let needle = "path";
    let mut search_from = 0;
    while let Some(rel) = block[search_from..].find(needle) {
        let idx = search_from + rel;
        // Ensure this is the `path` key, not a substring like `sandbox_policy`.
        let before_ok = idx == 0
            || !block.as_bytes()[idx - 1].is_ascii_alphanumeric()
                && block.as_bytes()[idx - 1] != b'_';
        let after = &block[idx + needle.len()..];
        let after_trimmed = after.trim_start();
        if before_ok && after_trimmed.starts_with('=') {
            // After the '=' find the first double-quoted string literal.
            let rest = after_trimmed[1..].trim_start();
            if let Some(start) = rest.find('"') {
                if let Some(end) = rest[start + 1..].find('"') {
                    return Some(rest[start + 1..start + 1 + end].to_string());
                }
            }
        }
        search_from = idx + needle.len();
    }
    None
}

/// Pull the HTTP method out of a `#[utoipa::path(...)]` body. Supports
/// both the bareword form (`get`, `post`, …) and the explicit
/// `method = "get"` form.
fn extract_method(block: &str) -> Option<String> {
    // Explicit `method = "..."` form first.
    if let Some(rel) = block.find("method") {
        let after = block[rel + "method".len()..].trim_start();
        if let Some(stripped) = after.strip_prefix('=') {
            let rest = stripped.trim_start();
            let rest = rest.trim_start_matches('"');
            for m in HTTP_METHODS {
                if rest.to_ascii_lowercase().starts_with(m) {
                    return Some((*m).to_string());
                }
            }
        }
    }
    // Bareword form: the method appears as a standalone token, almost
    // always the first token in the block.
    for token in block.split(|c: char| !c.is_ascii_alphanumeric()) {
        let lower = token.to_ascii_lowercase();
        if HTTP_METHODS.contains(&lower.as_str()) {
            return Some(lower);
        }
    }
    None
}

/// Source-of-truth `(method, path)` pairs gathered from every
/// `#[utoipa::path(...)]` attribute in the crate.
fn annotated_pairs() -> BTreeSet<(String, String)> {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);

    let mut pairs = BTreeSet::new();
    for file in files {
        let content = match std::fs::read_to_string(&file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let bytes = content.as_bytes();
        let marker = "#[utoipa::path";
        let mut from = 0;
        while let Some(rel) = content[from..].find(marker) {
            let attr_idx = from + rel;
            // Find the opening paren after the marker.
            let after_marker = attr_idx + marker.len();
            let open_paren = match content[after_marker..].find('(') {
                Some(p) => after_marker + p,
                None => {
                    from = after_marker;
                    continue;
                }
            };
            // Reject lines where the marker is inside a `//` comment.
            let line_start = content[..attr_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_prefix = &content[line_start..attr_idx];
            if line_prefix.trim_start().starts_with("//") {
                from = after_marker;
                continue;
            }
            if let Some((block, next)) = balanced_paren_block(bytes, open_paren) {
                if let (Some(method), Some(path)) = (extract_method(&block), extract_path(&block)) {
                    pairs.insert((method, path));
                }
                from = next;
            } else {
                from = after_marker;
            }
        }
    }
    pairs
}

/// `(method, path)` pairs actually present in the generated OpenAPI doc.
fn documented_pairs() -> BTreeSet<(String, String)> {
    let spec = ApiDoc::openapi();
    let json = spec.to_json().expect("OpenAPI spec must serialize");
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("OpenAPI spec must be valid JSON");
    let paths = parsed["paths"]
        .as_object()
        .expect("OpenAPI spec must declare a `paths` object");

    let mut pairs = BTreeSet::new();
    for (path, ops) in paths {
        if let Some(ops_obj) = ops.as_object() {
            for method in ops_obj.keys() {
                let lower = method.to_ascii_lowercase();
                if HTTP_METHODS.contains(&lower.as_str()) {
                    pairs.insert((lower, path.clone()));
                }
            }
        }
    }
    pairs
}

#[test]
fn every_annotated_handler_is_in_openapi() {
    let annotated = annotated_pairs();
    let documented = documented_pairs();

    // Sanity: the source scan must find a meaningful number of handlers,
    // otherwise a broken parser would make this test pass vacuously.
    assert!(
        annotated.len() > 100,
        "source scan found only {} #[utoipa::path] handlers — the parser \
         likely regressed (expected 300+)",
        annotated.len()
    );

    let missing: Vec<&(String, String)> = annotated.difference(&documented).collect();

    assert!(
        missing.is_empty(),
        "{} handler(s) carry `#[utoipa::path]` in \
         crates/librefang-api/src/ but are NOT referenced from \
         `ApiDoc::paths(...)` in openapi.rs, so they are missing from \
         openapi.json and every generated SDK. Add each one to the \
         `paths(...)` macro (or, if the path was retired, remove the \
         stale `#[utoipa::path]` annotation):\n\n{:#?}",
        missing.len(),
        missing,
    );
}
