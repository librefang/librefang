use crate::common::repo_root;
use clap::Parser;
use regex::Regex;
use std::fs;
use std::path::Path;

#[derive(Parser, Debug)]
pub struct SyncVersionsArgs {
    /// Version to set (e.g. 2026.3.2114). If omitted, syncs other files to the current Cargo.toml version.
    pub version: Option<String>,
}

fn read_workspace_version(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(root.join("Cargo.toml"))?;
    let doc = content.parse::<toml_edit::DocumentMut>()?;
    let version = doc["workspace"]["package"]["version"]
        .as_str()
        .ok_or("could not read workspace.package.version from Cargo.toml")?
        .to_string();
    Ok(version)
}

fn validate_calver(version: &str) -> Result<(), Box<dyn std::error::Error>> {
    let re = Regex::new(r"^[0-9]{4}\.[0-9]{1,2}\.[0-9]{2,4}(-(beta|rc)[0-9]+)?$")?;
    if !re.is_match(version) {
        return Err(format!(
            "'{}' is not a valid CalVer (expected: YYYY.M.DDHH e.g. 2026.3.2114)",
            version
        )
        .into());
    }
    Ok(())
}

fn update_cargo_toml(root: &Path, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = root.join("Cargo.toml");
    let content = fs::read_to_string(&path)?;
    let mut doc = content.parse::<toml_edit::DocumentMut>()?;
    doc["workspace"]["package"]["version"] = toml_edit::value(version);
    fs::write(&path, doc.to_string())?;
    println!("  Updated Cargo.toml workspace version");
    Ok(())
}

fn update_json_version(
    path: &Path,
    version: &str,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(path)?;
    // Replace only the top-level "version" field (indented with exactly 2 spaces)
    let re = Regex::new(r#"(?m)^(  "version"\s*:\s*")[^"]*(")"#)?;
    let new_content = re.replace(&content, format!(r#"${{1}}{}${{2}}"#, version).as_str());
    fs::write(path, new_content.as_ref())?;
    println!("  Updated {}", label);
    Ok(())
}

fn update_rust_sdk_cargo(path: &Path, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(path)?;
    let mut doc = content.parse::<toml_edit::DocumentMut>()?;
    doc["package"]["version"] = toml_edit::value(version);
    fs::write(path, doc.to_string())?;
    println!("  Updated sdk/rust/Cargo.toml");
    Ok(())
}

fn update_rust_sdk_readme(path: &Path, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(());
    }
    // Extract MAJOR.MINOR from version
    let parts: Vec<&str> = version.splitn(3, '.').collect();
    let major_minor = if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        version.to_string()
    };
    let content = fs::read_to_string(path)?;
    // Only replace within [dependencies] code blocks, not the entire file
    let re = Regex::new(r"(?s)(\[dependencies\].*?```)")?;
    let new_content = re.replace_all(&content, |caps: &regex::Captures| {
        let block = caps.get(0).unwrap().as_str();
        let ver_re = Regex::new(r#"librefang = "[^"]*""#).unwrap();
        ver_re
            .replace_all(block, format!(r#"librefang = "{}""#, major_minor).as_str())
            .to_string()
    });
    fs::write(path, new_content.as_ref())?;
    println!("  Updated sdk/rust/README.md");
    Ok(())
}

fn update_python_setup(path: &Path, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(());
    }
    // PEP 440: -beta1 → b1, -rc1 → rc1
    let py_version = version.replace("-beta", "b").replace("-rc", "rc");
    let content = fs::read_to_string(path)?;
    let re = Regex::new(r#"version="[^"]*""#)?;
    let new_content = re.replace(&content, format!(r#"version="{}""#, py_version).as_str());
    fs::write(path, new_content.as_ref())?;
    println!("  Updated sdk/python/setup.py (PEP 440: {})", py_version);
    Ok(())
}

fn update_tauri_conf(path: &Path, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(());
    }
    // Parse version components
    let base_ver = version.split('-').next().unwrap_or(version);
    let parts: Vec<&str> = base_ver.split('.').collect();
    if parts.len() != 3 {
        return Err(format!("unexpected version format for Tauri: {}", version).into());
    }
    let yyyy: u32 = parts[0].parse()?;
    let yy = yyyy % 100;
    let month: u32 = parts[1].parse()?;
    let ddhh: u32 = parts[2].parse()?;

    // Determine pre-release suffix
    let prerelease_re = Regex::new(r"-(beta|rc)([0-9]+)")?;
    let tauri_patch = if let Some(caps) = prerelease_re.captures(version) {
        let kind = caps.get(1).unwrap().as_str();
        let n: u32 = caps.get(2).unwrap().as_str().parse()?;
        match kind {
            "beta" => ddhh * 10 + n,
            "rc" => ddhh * 10 + 4 + n,
            _ => ddhh * 10 + 9,
        }
    } else {
        ddhh * 10 + 9
    };

    let tauri_version = format!("{}.{}.{}", yy, month, tauri_patch);

    let content = fs::read_to_string(path)?;
    let re = Regex::new(r#""version"\s*:\s*"[^"]*""#)?;
    let new_content = re.replace(
        &content,
        format!(r#""version": "{}""#, tauri_version).as_str(),
    );
    fs::write(path, new_content.as_ref())?;
    println!(
        "  Updated crates/librefang-desktop/tauri.conf.json ({})",
        tauri_version
    );
    Ok(())
}

pub fn run(args: SyncVersionsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    let current = read_workspace_version(&root)?;

    let version = if let Some(v) = args.version {
        validate_calver(&v)?;
        if v == current {
            println!("Version is already {}", v);
        } else {
            println!("Bumping version: {} -> {}", current, v);
            update_cargo_toml(&root, &v)?;
        }
        v
    } else {
        println!("Syncing to current version: {}", current);
        current
    };

    // Update JS SDK
    update_json_version(
        &root.join("sdk/javascript/package.json"),
        &version,
        "sdk/javascript/package.json",
    )?;

    // Update Rust SDK
    update_rust_sdk_cargo(&root.join("sdk/rust/Cargo.toml"), &version)?;

    // Update Rust SDK README
    update_rust_sdk_readme(&root.join("sdk/rust/README.md"), &version)?;

    // Update Python SDK
    update_python_setup(&root.join("sdk/python/setup.py"), &version)?;

    // Update WhatsApp gateway
    update_json_version(
        &root.join("packages/whatsapp-gateway/package.json"),
        &version,
        "packages/whatsapp-gateway/package.json",
    )?;

    // Update Tauri desktop config
    update_tauri_conf(
        &root.join("crates/librefang-desktop/tauri.conf.json"),
        &version,
    )?;

    // Verification
    println!();
    println!("Verification:");
    let new_ver = read_workspace_version(&root)?;
    println!("  Cargo.toml:      {}", new_ver);
    println!();
    println!("Done. Run 'git diff' to review changes.");

    Ok(())
}
