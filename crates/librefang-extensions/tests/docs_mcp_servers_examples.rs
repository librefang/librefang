//! Every `[[mcp_servers]]` example in the docs must deserialize into the type
//! the daemon actually loads.
//!
//! `McpServerConfigEntry` is `#[serde(deny_unknown_fields)]`, so a stanza with
//! the launch details at the top level of the entry — rather than inside a
//! `transport` table — is not a stylistic slip that degrades gracefully. It is
//! a hard parse error that takes the daemon's whole `config.toml` down with it:
//!
//! ```text
//! unknown field `args`, expected one of `name`, `template_id`, `transport`,
//! `timeout_secs`, `env`, `headers`, `oauth`, `taint_scanning`, `taint_policy`
//! ```
//!
//! Three distinct broken shapes were shipping across six pages when this test
//! was written, all of them copy-paste descendants of an older config schema:
//! top-level `command` / `args` (`operations/faq`), a bare top-level `url`
//! (`configuration/core`), and a `[mcp_servers.transport.Http]` sub-table
//! naming the variant in the path instead of carrying the `type` tag that
//! `#[serde(tag = "type")]` requires (`configuration/features`). A user
//! copying any of them got a daemon that would not boot.
//!
//! Prose can only be checked by reading it; a config example can be checked by
//! parsing it. This walks every `.mdx` page under `docs/src/app`, extracts each
//! `[[mcp_servers]]` stanza out of the fenced TOML blocks, and runs it through
//! the real type. New pages are covered automatically — there is no list to
//! keep in sync.

use std::path::{Path, PathBuf};

use librefang_types::config::McpServerConfigEntry;

/// Mirrors the `config.toml` fragment shape so a stanza parses in the same
/// array-of-tables context the daemon reads it in.
#[derive(serde::Deserialize)]
struct ConfigFragment {
    #[allow(dead_code)]
    mcp_servers: Vec<McpServerConfigEntry>,
}

/// One extracted stanza, carrying enough provenance to point a failure at the
/// exact page and line rather than at an anonymous blob of TOML.
struct Stanza {
    /// Repo-relative, for a readable assertion message.
    page: String,
    /// 1-indexed line of the `[[mcp_servers]]` header within the page.
    line: usize,
    toml: String,
}

fn docs_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = `<repo>/crates/librefang-extensions`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/src/app")
        .canonicalize()
        .expect("docs/src/app should exist — was the docs tree moved?")
}

fn mdx_pages(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Build output and dependencies carry vendored .mdx that is not ours.
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "node_modules" || name == ".next" {
                continue;
            }
            mdx_pages(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("mdx") {
            out.push(path);
        }
    }
}

/// Pull every `[[mcp_servers]]` stanza out of the ```toml fences in `src`.
///
/// A stanza runs from its `[[mcp_servers]]` header to the next top-level
/// table header, absorbing any `[mcp_servers.…]` / `[[mcp_servers.…]]`
/// sub-tables along the way — those belong to the entry and dropping them
/// would fabricate a parse error the page does not have.
fn extract_stanzas(src: &str, page: &str) -> Vec<Stanza> {
    let mut out = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;
    let mut in_toml_fence = false;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("```") {
            in_toml_fence = !in_toml_fence && trimmed == "```toml";
            i += 1;
            continue;
        }
        if !in_toml_fence || trimmed != "[[mcp_servers]]" {
            i += 1;
            continue;
        }

        let start = i;
        let mut stanza = vec![lines[i]];
        i += 1;
        while i < lines.len() {
            let t = lines[i].trim();
            if t.starts_with("```") {
                break;
            }
            let is_subtable = t.starts_with("[mcp_servers.") || t.starts_with("[[mcp_servers.");
            if t.starts_with('[') && !is_subtable {
                break;
            }
            stanza.push(lines[i]);
            i += 1;
        }
        out.push(Stanza {
            page: page.to_string(),
            line: start + 1,
            toml: stanza.join("\n"),
        });
    }
    out
}

#[test]
fn every_documented_mcp_servers_example_parses() {
    let root = docs_root();
    let mut pages = Vec::new();
    mdx_pages(&root, &mut pages);
    assert!(
        pages.len() > 50,
        "only found {} .mdx pages under {} — the walk is probably not reaching the docs tree",
        pages.len(),
        root.display()
    );

    let mut stanzas = Vec::new();
    for path in &pages {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        let src = std::fs::read_to_string(path).expect("read page");
        stanzas.extend(extract_stanzas(&src, &rel));
    }

    // The extractor silently finding nothing would make this test vacuous —
    // it would pass just as happily against a docs tree it failed to parse.
    assert!(
        stanzas.len() >= 20,
        "extracted only {} [[mcp_servers]] stanzas; the docs carry far more, so the extractor is broken",
        stanzas.len()
    );

    let failures: Vec<String> = stanzas
        .iter()
        .filter_map(|s| {
            toml::from_str::<ConfigFragment>(&s.toml)
                .err()
                .map(|e| format!("{}:{}\n{}\n  --> {e}", s.page, s.line, s.toml))
        })
        .collect();

    assert!(
        failures.is_empty(),
        "{} of {} documented [[mcp_servers]] examples do not deserialize into \
         McpServerConfigEntry. A user copying one gets a daemon that fails to \
         load its config — `deny_unknown_fields` makes this fatal, not lenient. \
         Launch details belong inside a `transport` table with an explicit \
         `type` tag, and `env` is a list of strings, not a table.\n\n{}",
        failures.len(),
        stanzas.len(),
        failures.join("\n\n")
    );
}
