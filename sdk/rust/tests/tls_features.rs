#[test]
fn tls_backends_are_explicit_and_selectable() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("default = [\"default-tls\"]"));
    assert!(manifest.contains("default-tls = [\"reqwest/default-tls\"]"));
    assert!(manifest.contains("rustls-tls = [\"reqwest/rustls-tls\"]"));
    assert!(manifest.contains(
        "reqwest = { version = \"0.12\", default-features = false, features = [\"json\", \"stream\", \"charset\", \"http2\", \"system-proxy\"] }"
    ));
}
