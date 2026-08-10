#[test]
fn published_package_uses_an_explicit_allowlist() {
    let manifest = include_str!("../Cargo.toml");
    let after_start = manifest
        .split_once("include = [")
        .expect("package include allowlist")
        .1;
    let include_body = after_start
        .split_once(']')
        .expect("closing package include bracket")
        .0;
    let entries: Vec<_> = include_body
        .split(',')
        .map(|entry| entry.trim().trim_matches('"'))
        .filter(|entry| !entry.is_empty())
        .collect();

    assert_eq!(
        entries,
        [
            "src/**",
            "examples/**",
            "tests/**",
            "README.md",
            "Cargo.toml"
        ]
    );
}
