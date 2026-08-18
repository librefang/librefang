use librefang_types::mcp::{McpCatalogEntry, McpCatalogTransport};
use std::path::PathBuf;

#[test]
fn every_mcp_fixture_keeps_setup_instructions_at_the_catalog_root() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/mcp");
    let mut fixture_paths = std::fs::read_dir(&fixture_dir)
        .expect("MCP fixture directory must exist")
        .map(|entry| {
            entry
                .expect("fixture directory entry must be readable")
                .path()
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect::<Vec<_>>();
    fixture_paths.sort();

    assert!(!fixture_paths.is_empty(), "MCP fixture directory is empty");
    for path in fixture_paths {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let entry = toml::from_str::<McpCatalogEntry>(&source)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
        assert!(
            !entry.setup_instructions.trim().is_empty(),
            "{} lost top-level setup_instructions (check TOML table ordering)",
            path.display(),
        );
    }
}

#[test]
fn sqlite_fixture_forwards_the_declared_database_path() {
    let entry =
        toml::from_str::<McpCatalogEntry>(include_str!("fixtures/registry/mcp/sqlite-mcp.toml"))
            .expect("SQLite MCP fixture must parse");

    assert!(entry
        .required_env
        .iter()
        .any(|env| env.name == "SQLITE_DB_PATH"));
    match entry.transport {
        McpCatalogTransport::Stdio { command, args } => {
            assert_eq!(command, "uvx");
            assert_eq!(
                args,
                [
                    "mcp-server-sqlite==2025.4.25",
                    "--db-path",
                    "$SQLITE_DB_PATH",
                ]
            );
        }
        _ => panic!("SQLite MCP fixture must use stdio transport"),
    }
}
