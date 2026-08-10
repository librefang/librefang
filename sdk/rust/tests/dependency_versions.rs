#[test]
fn thiserror_matches_the_workspace_major_version() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("thiserror = \"2\""));
    assert!(!manifest.contains("thiserror = \"1\""));
}
