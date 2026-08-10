#[test]
fn published_runtime_does_not_force_the_multithread_scheduler() {
    let manifest = include_str!("../Cargo.toml");

    let normal_tokio_dependency = if let Some(start) = manifest.find("[dependencies.tokio]") {
        let section = &manifest[start..];
        &section[..section[1..]
            .find("\n[")
            .map_or(section.len(), |end| end + 1)]
    } else {
        manifest
            .lines()
            .find(|line| line.starts_with("tokio = "))
            .expect("normal Tokio dependency")
    };
    assert!(!normal_tokio_dependency.contains("rt-multi-thread"));

    let assert_current_thread = |name: &str, source: &str| {
        assert!(
            source.contains("#[tokio::main(flavor = \"current_thread\")]"),
            "{name} must use the published current-thread runtime"
        );
        assert!(
            !source.contains("#[tokio::main]"),
            "{name} must not require rt-multi-thread"
        );
    };
    for (name, source) in [
        ("echo example", include_str!("../examples/echo.rs")),
        ("README", include_str!("../README.md")),
        ("crate docs", include_str!("../src/lib.rs")),
        ("runtime docs", include_str!("../src/runtime.rs")),
    ] {
        assert_current_thread(name, source);
    }

    // The canonical architecture guide lives outside this published crate.
    // Guard it in the monorepo, but keep crates.io source archives testable.
    let architecture_guide = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../docs/architecture/rust-sidecar-sdk.md");
    if architecture_guide.is_file() {
        let source = std::fs::read_to_string(architecture_guide).unwrap();
        assert_current_thread("architecture guide", &source);
    }
}
