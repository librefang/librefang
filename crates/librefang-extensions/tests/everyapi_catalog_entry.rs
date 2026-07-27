//! Drift guard for the EveryAPI MCP bridge documented in
//! `docs/src/app/integrations/mcp-a2a/page.mdx` (and its zh mirror).
//!
//! The bridge needs no LibreFang code: `everyapi mcp` is an MCP stdio server
//! and LibreFang is an MCP stdio client, so the whole integration is config
//! the user copies out of the docs. That makes the docs the source of truth,
//! and makes an untested docs page the actual failure mode — a catalog entry
//! that does not deserialize, or an approval glob that silently matches zero
//! tools, would ship as prose with nothing to catch it.
//!
//! So rather than duplicating the TOML into a fixture that can drift from the
//! page, these tests extract the fenced blocks straight out of the MDX and
//! exercise them against the real loader:
//!
//! 1. The catalog block parses into `McpCatalogEntry`, loads through
//!    `McpCatalog::load` from a temp home, and converts into the
//!    `McpServerConfigEntry` the installer would write.
//! 2. The `[[mcp_servers]]` block parses into `McpServerConfigEntry` — the
//!    stanza is the mandatory half of the integration and is `deny_unknown_fields`.
//! 3. The `require_approval` globs in the docs match exactly the 8 write tools
//!    and none of the 7 read tools, under the *doubled* namespacing that
//!    EveryAPI's already-prefixed tool names produce.

use librefang_extensions::catalog::McpCatalog;
use librefang_extensions::installer::catalog_entry_to_mcp_server;
use librefang_types::capability::glob_matches;
use librefang_types::config::{McpServerConfigEntry, McpTransportEntry};
use librefang_types::mcp::{McpCatalogEntry, McpCatalogTransport, McpCategory};

/// Repo-relative paths of the two mirrored pages carrying the bridge docs.
/// CARGO_MANIFEST_DIR = `<repo>/crates/librefang-extensions`.
const DOC_PAGES: &[&str] = &[
    "../../docs/src/app/integrations/mcp-a2a/page.mdx",
    "../../docs/src/app/zh/integrations/mcp-a2a/page.mdx",
];

/// The 7 read-only tools, as EveryAPI registers them (`registerTools()` in
/// `clients/cli/internal/mcp/tools.go`). Raw names, before LibreFang's
/// `mcp_{server}_` namespacing.
const READ_TOOLS: &[&str] = &[
    "everyapi_status",
    "everyapi_topup",
    "everyapi_seller_list",
    "everyapi_seller_eligibility",
    "everyapi_edge_list",
    "everyapi_edge_status",
    "everyapi_admin_marketplace_status",
];

/// The 8 write tools. `seller_add_key` uploads a plaintext upstream API key
/// and is the only one of these with no `confirm` gate on the EveryAPI side,
/// which is why the docs single it out.
const WRITE_TOOLS: &[&str] = &[
    "everyapi_seller_add_key",
    "everyapi_seller_withdraw",
    "everyapi_admin_marketplace_set",
    "everyapi_edge_remove",
    "everyapi_seller_add_oauth_codex_start",
    "everyapi_seller_add_oauth_codex_poll",
    "everyapi_seller_add_oauth_claude_start",
    "everyapi_seller_add_oauth_claude_complete",
];

/// Read one of the doc pages, failing with the path so a docs move is legible.
fn read_page(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read {} — was the docs page moved? {e}",
            path.display()
        )
    })
}

/// Extract the first ```toml fence in `page` whose body contains `marker`.
///
/// Panics naming both the page and the marker: if a docs edit drops or renames
/// the block, the failure should say which snippet went missing rather than
/// surfacing as an opaque TOML parse error on an empty string.
fn extract_toml_fence(page: &str, rel: &str, marker: &str) -> String {
    let mut in_fence = false;
    let mut current = String::new();
    for line in page.lines() {
        if line.trim_start().starts_with("```") {
            if in_fence {
                if current.contains(marker) {
                    return current;
                }
                current.clear();
                in_fence = false;
            } else if line.trim() == "```toml" {
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            current.push_str(line);
            current.push('\n');
        }
    }
    panic!("no ```toml block containing {marker:?} found in {rel} — did the EveryAPI MCP section change?");
}

/// Namespaced form LibreFang gives an MCP tool.
///
/// Delegates to the production `format_mcp_tool_name` rather than
/// re-implementing it. That is load-bearing: if de-duplication were ever added
/// to the real function, `mcp_everyapi_everyapi_status` would collapse to
/// `mcp_everyapi_status`, every documented glob would silently match zero
/// tools, and the approval gate would stop applying with no error anywhere.
/// A local copy of the formatting would keep passing through exactly that
/// change. Reached via `librefang-runtime`'s `pub use librefang_runtime_mcp as
/// mcp`, which is already a dev-dependency — no new dependency needed.
fn namespaced(server: &str, tool: &str) -> String {
    librefang_runtime::mcp::format_mcp_tool_name(server, tool)
}

#[test]
fn documented_catalog_entry_parses_and_loads() {
    for rel in DOC_PAGES {
        let page = read_page(rel);
        let toml_src = extract_toml_fence(&page, rel, "id = \"everyapi\"");

        // 1. It deserializes into the catalog type at all.
        let entry: McpCatalogEntry = toml::from_str(&toml_src).unwrap_or_else(|e| {
            panic!("catalog TOML in {rel} does not parse as McpCatalogEntry: {e}")
        });
        assert_eq!(entry.id, "everyapi");
        assert!(matches!(entry.category, McpCategory::Cloud));
        match &entry.transport {
            McpCatalogTransport::Stdio { command, args } => {
                assert_eq!(command, "everyapi");
                assert_eq!(args, &["mcp".to_string()]);
            }
            other => panic!("expected stdio transport in {rel}, got {other:?}"),
        }
        // No credential is declared: the MCP server reads the gateway's own
        // credentials file, so `librefang mcp add everyapi` must not prompt.
        assert!(
            entry.required_env.is_empty(),
            "the EveryAPI entry must declare no required_env — the server reads \
             ~/.config/everyapi/credentials.json itself ({rel})"
        );

        // 2. It survives the real loader from a catalog dir on disk.
        let home = tempfile::TempDir::new().expect("tempdir");
        let catalog_dir = home.path().join("mcp").join("catalog");
        std::fs::create_dir_all(&catalog_dir).expect("create catalog dir");
        std::fs::write(catalog_dir.join("everyapi.toml"), &toml_src).expect("write entry");

        let mut catalog = McpCatalog::new(home.path());
        let loaded = catalog.load(home.path());
        assert_eq!(
            loaded, 1,
            "catalog from {rel} should load exactly one entry"
        );
        let from_disk = catalog
            .get("everyapi")
            .unwrap_or_else(|| panic!("entry from {rel} not addressable by id after load"));

        // 3. It converts into the config entry `librefang mcp add` would write.
        let server = catalog_entry_to_mcp_server(from_disk);
        assert_eq!(server.name, "everyapi");
        assert_eq!(server.template_id.as_deref(), Some("everyapi"));
        assert!(
            server.env.is_empty(),
            "no env passthrough should be synthesised for the EveryAPI entry ({rel})"
        );
        match server.transport {
            Some(McpTransportEntry::Stdio { command, args }) => {
                assert_eq!(command, "everyapi");
                assert_eq!(args, vec!["mcp".to_string()]);
            }
            other => panic!("expected stdio transport after conversion in {rel}, got {other:?}"),
        }
    }
}

#[test]
fn documented_mcp_servers_stanza_parses() {
    // `McpServerConfigEntry` is `deny_unknown_fields`, so this catches the
    // pre-existing docs bug class where `command` sat at the top level of the
    // entry instead of inside a `transport` table.
    #[derive(serde::Deserialize)]
    struct ConfigFragment {
        mcp_servers: Vec<McpServerConfigEntry>,
    }

    for rel in DOC_PAGES {
        let page = read_page(rel);
        let toml_src = extract_toml_fence(&page, rel, "command = \"everyapi\"");
        let parsed: ConfigFragment = toml::from_str(&toml_src)
            .unwrap_or_else(|e| panic!("[[mcp_servers]] stanza in {rel} does not parse: {e}"));

        assert_eq!(parsed.mcp_servers.len(), 1);
        let entry = &parsed.mcp_servers[0];
        // The approval globs below hardcode this name; a rename in the docs
        // would silently invalidate them.
        assert_eq!(
            entry.name, "everyapi",
            "the documented server name is load-bearing for the require_approval globs ({rel})"
        );
        assert!(entry.env.is_empty());
        match &entry.transport {
            Some(McpTransportEntry::Stdio { command, args }) => {
                assert_eq!(command, "everyapi");
                assert_eq!(args, &["mcp".to_string()]);
            }
            other => panic!("expected stdio transport in {rel}, got {other:?}"),
        }
    }
}

#[test]
fn tool_names_are_double_prefixed() {
    // EveryAPI's tools are already named `everyapi_*`, and LibreFang prepends
    // `mcp_{server}_` without de-duplicating. Pin the resulting shape: the
    // approval globs in the docs are only correct because of it.
    assert_eq!(
        namespaced("everyapi", "everyapi_status"),
        "mcp_everyapi_everyapi_status"
    );
    assert_eq!(
        namespaced("everyapi", "everyapi_seller_withdraw"),
        "mcp_everyapi_everyapi_seller_withdraw"
    );
    // A renamed server breaks every documented glob — this is what the docs warn about.
    assert_eq!(
        namespaced("every-api", "everyapi_status"),
        "mcp_every_api_everyapi_status"
    );
}

#[test]
fn documented_approval_globs_gate_writes_and_only_writes() {
    // The glob set the docs tell operators to paste into `[approval]`.
    // `ApprovalManager::requires_approval` matches each pattern with the same
    // `glob_matches` used here.
    let patterns = [
        "mcp_everyapi_everyapi_seller_add_*",
        "mcp_everyapi_everyapi_seller_withdraw",
        "mcp_everyapi_everyapi_admin_marketplace_set",
        "mcp_everyapi_everyapi_edge_remove",
    ];
    let gated = |tool: &str| {
        patterns
            .iter()
            .any(|p| glob_matches(p, &namespaced("everyapi", tool)))
    };

    assert_eq!(
        READ_TOOLS.len() + WRITE_TOOLS.len(),
        15,
        "the server exposes 15 tools"
    );

    for tool in WRITE_TOOLS {
        assert!(
            gated(tool),
            "write tool {tool} is NOT covered by the documented require_approval globs"
        );
    }
    for tool in READ_TOOLS {
        assert!(
            !gated(tool),
            "read tool {tool} is gated by the documented globs — the read-only \
             profile would prompt for approval on a harmless call"
        );
    }

    // The patterns must also appear verbatim in both pages, so a docs edit that
    // changes one without changing this test fails rather than drifting.
    for rel in DOC_PAGES {
        let page = read_page(rel);
        for pattern in &patterns {
            assert!(
                page.contains(pattern),
                "documented glob {pattern:?} missing from {rel}"
            );
        }
    }
}
