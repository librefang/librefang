use crate::common::repo_root;
use clap::Parser;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct PublishSdksArgs {
    /// Publish JavaScript SDK only
    #[arg(long)]
    pub js: bool,

    /// Publish Python SDK only
    #[arg(long)]
    pub python: bool,

    /// Publish Rust SDK only
    #[arg(long)]
    pub rust: bool,

    /// Dry run — validate but don't actually publish
    #[arg(long)]
    pub dry_run: bool,
}

fn has_command(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn publish_js(root: &Path, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    let sdk_dir = root.join("sdk/javascript");
    if !sdk_dir.join("package.json").exists() {
        println!("  Skipping JS SDK (sdk/javascript/package.json not found)");
        return Ok(());
    }

    if !has_command("npm") {
        return Err("npm not found — install Node.js first".into());
    }

    println!("Publishing JavaScript SDK...");

    // --ignore-scripts: block lifecycle hooks (prepublishOnly, prepack,
    // postpublish) during publish so a future malicious dep cannot
    // exfiltrate NODE_AUTH_TOKEN. Matches publish_npm_binaries.rs and
    // the inline `npm publish` calls in release.yml.
    let mut args = vec!["publish", "--access", "public", "--ignore-scripts"];
    if dry_run {
        args.push("--dry-run");
    }

    let status = Command::new("npm")
        .args(&args)
        .current_dir(&sdk_dir)
        .status()?;

    if !status.success() {
        return Err("npm publish failed".into());
    }
    println!("  JS SDK published");
    Ok(())
}

fn publish_python(root: &Path, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    let sdk_dir = root.join("sdk/python");
    if !sdk_dir.join("setup.py").exists() {
        println!("  Skipping Python SDK (sdk/python/setup.py not found)");
        return Ok(());
    }

    // Clean old dist
    let dist_dir = sdk_dir.join("dist");
    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir)?;
    }

    println!("Publishing Python SDK...");

    // Build sdist + wheel
    println!("  Building distribution...");
    let status = Command::new("python3")
        .args(["setup.py", "sdist", "bdist_wheel"])
        .current_dir(&sdk_dir)
        .status()?;
    if !status.success() {
        return Err("python setup.py sdist bdist_wheel failed".into());
    }

    // Upload via twine
    if !has_command("twine") {
        return Err("twine not found — install with: pip install twine".into());
    }

    let mut args = vec!["upload", "dist/*"];
    if dry_run {
        args.push("--repository");
        args.push("testpypi");
    }

    // Use shell for glob expansion
    let shell_cmd = if dry_run {
        "twine upload --repository testpypi dist/*"
    } else {
        "twine upload dist/*"
    };

    let status = Command::new("sh")
        .args(["-c", shell_cmd])
        .current_dir(&sdk_dir)
        .status()?;
    if !status.success() {
        return Err("twine upload failed".into());
    }
    println!("  Python SDK published");
    Ok(())
}

fn publish_rust(root: &Path, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    let sdk_dir = root.join("sdk/rust");
    if !sdk_dir.join("Cargo.toml").exists() {
        println!("  Skipping Rust SDK (sdk/rust/Cargo.toml not found)");
        return Ok(());
    }

    println!("Publishing Rust SDK...");

    let mut args = vec!["publish"];
    if dry_run {
        args.push("--dry-run");
    }

    let status = Command::new("cargo")
        .args(&args)
        .current_dir(&sdk_dir)
        .status()?;
    if !status.success() {
        return Err("cargo publish failed".into());
    }
    println!("  Rust SDK published");
    Ok(())
}

pub fn run(args: PublishSdksArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let publish_all = !args.js && !args.python && !args.rust;

    if args.dry_run {
        println!("=== Dry run mode ===");
        println!();
    }

    let mut errors: Vec<String> = Vec::new();

    if publish_all || args.js {
        if let Err(e) = publish_js(&root, args.dry_run) {
            errors.push(format!("JS: {}", e));
        }
    }

    if publish_all || args.python {
        if let Err(e) = publish_python(&root, args.dry_run) {
            errors.push(format!("Python: {}", e));
        }
    }

    if publish_all || args.rust {
        if let Err(e) = publish_rust(&root, args.dry_run) {
            errors.push(format!("Rust: {}", e));
        }
    }

    if errors.is_empty() {
        println!();
        println!("All SDK publishes complete.");
        Ok(())
    } else {
        println!();
        println!("Some publishes failed:");
        for e in &errors {
            println!("  - {}", e);
        }
        Err(format!("{} SDK publish(es) failed", errors.len()).into())
    }
}
