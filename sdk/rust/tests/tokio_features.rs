#[test]
fn published_tokio_dependency_uses_only_required_features() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        manifest.contains("tokio = { version = \"1\", features = [\"rt\", \"sync\", \"macros\"] }")
    );
    assert!(!manifest.contains("features = [\"full\"]"));
}
