use librefang_types::mcp::McpCatalogEntry;
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
