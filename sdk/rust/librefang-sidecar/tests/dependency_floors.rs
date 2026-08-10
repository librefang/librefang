fn section<'a>(manifest: &'a str, header: &str) -> Option<&'a str> {
    let start = manifest.find(header)?;
    let body = &manifest[start + header.len()..];
    Some(&body[..body.find("\n[").unwrap_or(body.len())])
}

fn quoted_value<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let value = line.split_once(marker)?.1;
    value
        .split_once('"')?
        .1
        .split_once('"')
        .map(|(value, _)| value)
}

fn dependency_version<'a>(manifest: &'a str, group: &str, name: &str) -> Option<&'a str> {
    let inline_header = format!("[{group}]");
    if let Some(inline_section) = section(manifest, &inline_header) {
        if let Some(line) = inline_section
            .lines()
            .find(|line| line.starts_with(&format!("{name} = ")))
        {
            return quoted_value(line, "version = ").or_else(|| quoted_value(line, "= "));
        }
    }

    let normalized_header = format!("[{group}.{name}]");
    section(manifest, &normalized_header)?
        .lines()
        .find_map(|line| quoted_value(line, "version = "))
}

#[test]
fn core_dependencies_declare_the_tested_version_floors() {
    let manifest = include_str!("../Cargo.toml");
    assert_eq!(
        dependency_version(manifest, "dependencies", "serde"),
        Some("1.0.229")
    );
    assert_eq!(
        dependency_version(manifest, "dependencies", "serde_json"),
        Some("1.0.151")
    );
    assert_eq!(
        dependency_version(manifest, "dependencies", "tokio"),
        Some("1.53.1")
    );
    assert_eq!(
        dependency_version(manifest, "dev-dependencies", "tokio"),
        Some("1.53.1")
    );
}
