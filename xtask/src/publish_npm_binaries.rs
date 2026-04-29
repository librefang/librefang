use crate::common::repo_root;
use clap::Parser;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct PublishNpmBinariesArgs {
    /// Version to publish (e.g. 2026.3.2214)
    #[arg(long, env = "VERSION")]
    pub version: String,

    /// GitHub repository (owner/repo)
    #[arg(long, env = "REPO", default_value = "librefang/librefang")]
    pub repo: String,

    /// Git tag for the release (e.g. v2026.3.2214)
    #[arg(long, env = "TAG")]
    pub tag: String,

    /// Dry run — show what would be published
    #[arg(long)]
    pub dry_run: bool,
}

struct Target {
    rust_target: &'static str,
    platform: &'static str,
    arch: &'static str,
    ext: &'static str,
    exe: &'static str,
}

const TARGETS: &[Target] = &[
    Target {
        rust_target: "x86_64-unknown-linux-gnu",
        platform: "linux",
        arch: "x64",
        ext: "tar.gz",
        exe: "librefang",
    },
    Target {
        rust_target: "aarch64-unknown-linux-gnu",
        platform: "linux",
        arch: "arm64",
        ext: "tar.gz",
        exe: "librefang",
    },
    Target {
        rust_target: "x86_64-unknown-linux-musl",
        platform: "linux-musl",
        arch: "x64",
        ext: "tar.gz",
        exe: "librefang",
    },
    Target {
        rust_target: "aarch64-unknown-linux-musl",
        platform: "linux-musl",
        arch: "arm64",
        ext: "tar.gz",
        exe: "librefang",
    },
    Target {
        rust_target: "x86_64-apple-darwin",
        platform: "darwin",
        arch: "x64",
        ext: "tar.gz",
        exe: "librefang",
    },
    Target {
        rust_target: "aarch64-apple-darwin",
        platform: "darwin",
        arch: "arm64",
        ext: "tar.gz",
        exe: "librefang",
    },
    Target {
        rust_target: "x86_64-pc-windows-msvc",
        platform: "win32",
        arch: "x64",
        ext: "zip",
        exe: "librefang.exe",
    },
    Target {
        rust_target: "aarch64-pc-windows-msvc",
        platform: "win32",
        arch: "arm64",
        ext: "zip",
        exe: "librefang.exe",
    },
];

fn npm_suffix(platform: &str, arch: &str) -> String {
    if platform == "linux-musl" {
        format!("linux-{}-musl", arch)
    } else {
        format!("{}-{}", platform, arch)
    }
}

fn npm_os(platform: &str) -> &str {
    if platform == "linux-musl" {
        "linux"
    } else {
        platform
    }
}

fn download_asset(url: &str, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for attempt in 1..=5 {
        let status = Command::new("curl")
            .args(["-fsSL", "-o", &dest.to_string_lossy(), url])
            .status()?;
        if status.success() {
            return Ok(());
        }
        if attempt < 5 {
            eprintln!("  Retrying download in 10s... ({}/5)", attempt);
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
    }
    Err(format!("Failed to download {}", url).into())
}

/// Download and verify the .sha256 checksum file for an asset.
///
/// Downloads `{asset_url}.sha256`, parses the expected hash, then computes
/// sha256 of the already-downloaded `asset_path` and compares. Returns an
/// error if the download fails or the hashes do not match.
///
/// Note: the `.sha256` sidecar lives in the same GitHub Release as the
/// binary, so this guards against transport corruption / partial downloads,
/// not against an attacker who has write access to the release assets.
fn verify_asset_sha256(
    repo: &str,
    tag: &str,
    asset_name: &str,
    asset_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let sha_url = format!(
        "https://github.com/{}/releases/download/{}/{}.sha256",
        repo, tag, asset_name
    );
    let tmp_sha = asset_path.with_extension("sha256");
    download_asset(&sha_url, &tmp_sha)?;

    let sha_content = fs::read_to_string(&tmp_sha)?;
    let expected = sha_content
        .split_whitespace()
        .next()
        .ok_or("empty .sha256 file")?
        .to_ascii_lowercase();

    let data = fs::read(asset_path)?;
    let actual = sha256_hex(&data);
    fs::remove_file(&tmp_sha).ok();

    if actual != expected {
        return Err(
            format!("SHA256 mismatch for {asset_name}: expected {expected}, got {actual}").into(),
        );
    }
    println!("  ✓ SHA256 verified: {expected}");
    Ok(())
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

pub fn run(args: PublishNpmBinariesArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let work = std::env::temp_dir().join(format!("xtask-npm-{}", std::process::id()));
    fs::create_dir_all(&work)?;

    let is_prerelease = args.version.contains("-beta") || args.version.contains("-rc");
    let npm_tag_args: Vec<&str> = if is_prerelease {
        vec!["--tag", "next"]
    } else {
        vec![]
    };

    for target in TARGETS {
        let suffix = npm_suffix(target.platform, target.arch);
        let pkg_name = format!("@librefang/cli-{}", suffix);
        let pkg_dir = work.join(&suffix);

        println!("=== {} ===", pkg_name);

        // Check if already published
        let check = Command::new("npm")
            .args(["view", &format!("{}@{}", pkg_name, args.version), "version"])
            .output();
        if let Ok(out) = check {
            if out.status.success() {
                println!("  Already published, skipping");
                continue;
            }
        }

        let bin_dir = pkg_dir.join("bin");
        fs::create_dir_all(&bin_dir)?;

        // Download binary
        let asset = format!("librefang-{}.{}", target.rust_target, target.ext);
        let url = format!(
            "https://github.com/{}/releases/download/{}/{}",
            args.repo, args.tag, asset
        );
        let asset_path = pkg_dir.join(&asset);
        println!("  Downloading {}", url);

        if !args.dry_run {
            download_asset(&url, &asset_path)?;

            // Verify SHA256 against the .sha256 file uploaded alongside the release asset
            verify_asset_sha256(&args.repo, &args.tag, &asset, &asset_path)?;

            // Extract
            if target.ext == "tar.gz" {
                Command::new("tar")
                    .args([
                        "xzf",
                        &asset_path.to_string_lossy(),
                        "-C",
                        &bin_dir.to_string_lossy(),
                    ])
                    .status()?;
            } else {
                Command::new("unzip")
                    .args([
                        "-q",
                        "-o",
                        &asset_path.to_string_lossy(),
                        "-d",
                        &bin_dir.to_string_lossy(),
                    ])
                    .status()?;
            }

            // chmod +x
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let exe_path = bin_dir.join(target.exe);
                if exe_path.exists() {
                    fs::set_permissions(&exe_path, fs::Permissions::from_mode(0o755))?;
                }
            }

            fs::remove_file(&asset_path).ok();
        }

        // Generate package.json
        let os_field = npm_os(target.platform);
        let package_json = serde_json::json!({
            "name": pkg_name,
            "version": args.version,
            "description": format!("LibreFang CLI binary for {}", suffix),
            "license": "MIT",
            "repository": {
                "type": "git",
                "url": format!("https://github.com/{}", args.repo)
            },
            "os": [os_field],
            "cpu": [target.arch],
            "bin": {
                "librefang": format!("./bin/{}", target.exe)
            },
            "files": [format!("bin/{}", target.exe)]
        });
        fs::write(
            pkg_dir.join("package.json"),
            serde_json::to_string_pretty(&package_json)?,
        )?;

        if args.dry_run {
            println!("  [dry-run] Would publish {}@{}", pkg_name, args.version);
        } else {
            // --ignore-scripts blocks lifecycle hooks (prepublishOnly,
            // prepack, postpublish) from running during publish, so a
            // future malicious dep cannot exfiltrate NODE_AUTH_TOKEN.
            let mut cmd = Command::new("npm");
            cmd.args([
                "publish",
                &pkg_dir.to_string_lossy(),
                "--access",
                "public",
                "--ignore-scripts",
            ]);
            for a in &npm_tag_args {
                cmd.arg(a);
            }
            let status = cmd.status()?;
            if !status.success() {
                return Err(format!("npm publish failed for {}", pkg_name).into());
            }
            println!("  Published {}@{}", pkg_name, args.version);
        }
    }

    // Publish wrapper package
    println!("=== @librefang/cli ===");
    let check = Command::new("npm")
        .args([
            "view",
            &format!("@librefang/cli@{}", args.version),
            "version",
        ])
        .output();
    if let Ok(out) = check {
        if out.status.success() {
            println!("  Already published, skipping");
            fs::remove_dir_all(&work).ok();
            return Ok(());
        }
    }

    let wrapper_src = root.join("packages/cli-npm");
    let wrapper_dir = work.join("cli-wrapper");
    if wrapper_src.exists() {
        copy_dir_recursive(&wrapper_src, &wrapper_dir)?;

        // Update version
        if !args.dry_run {
            Command::new("npm")
                .args([
                    "version",
                    &args.version,
                    "--no-git-tag-version",
                    "--allow-same-version",
                ])
                .current_dir(&wrapper_dir)
                .status()?;

            // Update optionalDependencies versions
            let pkg_path = wrapper_dir.join("package.json");
            let pkg_content = fs::read_to_string(&pkg_path)?;
            let mut pkg: serde_json::Value = serde_json::from_str(&pkg_content)?;
            if let Some(opt_deps) = pkg
                .get_mut("optionalDependencies")
                .and_then(|v| v.as_object_mut())
            {
                for (_key, val) in opt_deps.iter_mut() {
                    *val = serde_json::Value::String(args.version.clone());
                }
            }
            fs::write(&pkg_path, serde_json::to_string_pretty(&pkg)? + "\n")?;

            let mut cmd = Command::new("npm");
            cmd.args(["publish", "--access", "public", "--ignore-scripts"])
                .current_dir(&wrapper_dir);
            for a in &npm_tag_args {
                cmd.arg(a);
            }
            let status = cmd.status()?;
            if !status.success() {
                return Err("npm publish failed for @librefang/cli wrapper".into());
            }
            println!("  Published @librefang/cli@{}", args.version);
        } else {
            println!("  [dry-run] Would publish @librefang/cli@{}", args.version);
        }
    } else {
        println!("  Warning: packages/cli-npm not found, skipping wrapper");
    }

    fs::remove_dir_all(&work).ok();
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)?.flatten() {
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}
