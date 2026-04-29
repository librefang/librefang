use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
pub struct PublishPypiBinariesArgs {
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

    /// Build wheels only; do not run `twine upload`.
    /// When set, wheels are written to `--dist-dir` (or a temp dir if unset)
    /// and the workflow is expected to upload them via OIDC trusted
    /// publishing (`pypa/gh-action-pypi-publish`).
    #[arg(long)]
    pub build_only: bool,

    /// Output directory for built wheels (used with `--build-only`).
    /// If unset, a temp directory is used.
    #[arg(long)]
    pub dist_dir: Option<PathBuf>,
}

struct PyTarget {
    rust_target: &'static str,
    platform_tag: &'static str,
    ext: &'static str,
    #[cfg_attr(not(unix), allow(dead_code))]
    exe: &'static str,
}

const TARGETS: &[PyTarget] = &[
    PyTarget {
        rust_target: "x86_64-unknown-linux-gnu",
        platform_tag: "manylinux_2_17_x86_64.manylinux2014_x86_64",
        ext: "tar.gz",
        exe: "librefang",
    },
    PyTarget {
        rust_target: "aarch64-unknown-linux-gnu",
        platform_tag: "manylinux_2_17_aarch64.manylinux2014_aarch64",
        ext: "tar.gz",
        exe: "librefang",
    },
    PyTarget {
        rust_target: "x86_64-unknown-linux-musl",
        platform_tag: "musllinux_1_2_x86_64",
        ext: "tar.gz",
        exe: "librefang",
    },
    PyTarget {
        rust_target: "aarch64-unknown-linux-musl",
        platform_tag: "musllinux_1_2_aarch64",
        ext: "tar.gz",
        exe: "librefang",
    },
    PyTarget {
        rust_target: "x86_64-apple-darwin",
        platform_tag: "macosx_10_12_x86_64",
        ext: "tar.gz",
        exe: "librefang",
    },
    PyTarget {
        rust_target: "aarch64-apple-darwin",
        platform_tag: "macosx_11_0_arm64",
        ext: "tar.gz",
        exe: "librefang",
    },
    PyTarget {
        rust_target: "x86_64-pc-windows-msvc",
        platform_tag: "win_amd64",
        ext: "zip",
        exe: "librefang.exe",
    },
    PyTarget {
        rust_target: "aarch64-pc-windows-msvc",
        platform_tag: "win_arm64",
        ext: "zip",
        exe: "librefang.exe",
    },
];

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

/// Download and verify the .sha256 sidecar for an asset.
///
/// Mirrors `publish_npm_binaries::verify_asset_sha256`. Fails fast if the
/// sidecar is missing or the hash mismatches — we refuse to repackage a
/// binary into a wheel without confirming what the GitHub Release
/// actually served us.
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

/// Convert version for PEP 440: -beta1 → b1, -rc1 → rc1
fn pypi_version(version: &str) -> String {
    version.replace("-beta", "b").replace("-rc", "rc")
}

fn build_wheel(
    target: &PyTarget,
    pypi_ver: &str,
    repo: &str,
    tag: &str,
    work: &Path,
    dist: &Path,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let pkg_name = "librefang";
    let wheel_dir = work.join(format!("wheel-{}", target.rust_target));
    let data_dir = format!("{}-{}.data/scripts", pkg_name, pypi_ver);
    let dist_info = format!("{}-{}.dist-info", pkg_name, pypi_ver);

    let _ = fs::remove_dir_all(&wheel_dir);
    fs::create_dir_all(wheel_dir.join(&data_dir))?;
    fs::create_dir_all(wheel_dir.join(&dist_info))?;

    // Download and extract binary
    let asset = format!("librefang-{}.{}", target.rust_target, target.ext);
    let url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        repo, tag, asset
    );
    println!("  Downloading {}", url);

    if !dry_run {
        let asset_path = wheel_dir.join(&asset);
        download_asset(&url, &asset_path)?;

        // Verify SHA256 against the .sha256 sidecar uploaded alongside the
        // release asset. Refuse to repackage on mismatch.
        verify_asset_sha256(repo, tag, &asset, &asset_path)?;

        let scripts_dir = wheel_dir.join(&data_dir);
        if target.ext == "tar.gz" {
            Command::new("tar")
                .args([
                    "xzf",
                    &asset_path.to_string_lossy(),
                    "-C",
                    &scripts_dir.to_string_lossy(),
                ])
                .status()?;
        } else {
            Command::new("unzip")
                .args([
                    "-q",
                    "-o",
                    &asset_path.to_string_lossy(),
                    "-d",
                    &scripts_dir.to_string_lossy(),
                ])
                .status()?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exe_path = scripts_dir.join(target.exe);
            if exe_path.exists() {
                fs::set_permissions(&exe_path, fs::Permissions::from_mode(0o755))?;
            }
        }

        fs::remove_file(&asset_path).ok();
    }

    // Write METADATA
    let metadata = format!(
        "Metadata-Version: 2.1\n\
         Name: {pkg_name}\n\
         Version: {pypi_ver}\n\
         Summary: LibreFang Agent OS CLI\n\
         Home-page: https://librefang.ai\n\
         License: MIT\n\
         Project-URL: Repository, https://github.com/{repo}\n\
         Project-URL: Documentation, https://librefang.ai/docs\n\
         Project-URL: Issues, https://github.com/{repo}/issues\n\
         Requires-Python: >=3.8\n\
         Description-Content-Type: text/markdown\n\n\
         # librefang\n\n\
         LibreFang Agent OS — command-line interface.\n"
    );
    fs::write(wheel_dir.join(&dist_info).join("METADATA"), metadata)?;

    // Write WHEEL
    let wheel_meta = format!(
        "Wheel-Version: 1.0\n\
         Generator: librefang-xtask\n\
         Root-Is-Purelib: false\n\
         Tag: py3-none-{}\n",
        target.platform_tag
    );
    fs::write(wheel_dir.join(&dist_info).join("WHEEL"), wheel_meta)?;

    if !dry_run {
        // Build RECORD with SHA256 hashes
        let record_path = wheel_dir.join(&dist_info).join("RECORD");
        let mut record = String::new();
        for entry in walkdir(&wheel_dir)? {
            let rel = entry
                .strip_prefix(&wheel_dir)
                .unwrap()
                .to_string_lossy()
                .to_string();
            if rel == format!("{}/RECORD", dist_info) {
                continue;
            }
            let content = fs::read(&entry)?;
            let hash = sha256_base64url(&content);
            let size = content.len();
            record.push_str(&format!("{},sha256={},{}\n", rel, hash, size));
        }
        record.push_str(&format!("{}/RECORD,,\n", dist_info));
        fs::write(&record_path, record)?;

        // Build wheel (zip with .whl extension)
        let wheel_name = format!(
            "{}-{}-py3-none-{}.whl",
            pkg_name, pypi_ver, target.platform_tag
        );
        let wheel_path = dist.join(&wheel_name);
        let status = Command::new("zip")
            .args(["-q", "-r", &wheel_path.to_string_lossy(), "."])
            .current_dir(&wheel_dir)
            .status()?;
        if !status.success() {
            return Err(format!("Failed to create wheel {}", wheel_name).into());
        }
        println!("  Built {}", wheel_name);
    } else {
        let wheel_name = format!(
            "{}-{}-py3-none-{}.whl",
            pkg_name, pypi_ver, target.platform_tag
        );
        println!("  [dry-run] Would build {}", wheel_name);
    }

    Ok(())
}

fn walkdir(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn sha256_base64url(data: &[u8]) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("python3")
        .args(["-c", "import hashlib,base64,sys;h=hashlib.sha256(sys.stdin.buffer.read()).digest();print(base64.urlsafe_b64encode(h).decode().rstrip('='))"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("python3 required for SHA256 hashing");

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(data);
    }
    let output = child
        .wait_with_output()
        .expect("failed to wait for python3");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn run(args: PublishPypiBinariesArgs) -> Result<(), Box<dyn std::error::Error>> {
    let pypi_ver = pypi_version(&args.version);
    let work = std::env::temp_dir().join(format!("xtask-pypi-{}", std::process::id()));
    let dist = match &args.dist_dir {
        Some(p) => p.clone(),
        None => work.join("dist"),
    };
    fs::create_dir_all(&dist)?;

    println!(
        "Publishing PyPI wheels for v{} (PEP 440: {})",
        args.version, pypi_ver
    );
    if args.build_only {
        println!("  --build-only set: wheels → {}", dist.display());
    }

    for target in TARGETS {
        println!("\n=== {} ({}) ===", target.rust_target, target.platform_tag);
        build_wheel(
            target,
            &pypi_ver,
            &args.repo,
            &args.tag,
            &work,
            &dist,
            args.dry_run,
        )?;
    }

    if !args.dry_run && !args.build_only {
        // Legacy path: twine + long-lived API token. Prefer `--build-only`
        // and let the workflow upload via OIDC trusted publishing
        // (`pypa/gh-action-pypi-publish`) — see `cli_pypi` in release.yml.
        println!("\n=== Uploading to PyPI (twine, legacy) ===");
        let _ = Command::new("pip")
            .args(["install", "--quiet", "twine"])
            .status();

        // Use shell for glob expansion
        let status = Command::new("sh")
            .args([
                "-c",
                &format!("twine upload --skip-existing {}/*.whl", dist.display()),
            ])
            .status()?;

        if !status.success() {
            return Err("twine upload failed".into());
        }
    } else if args.build_only {
        println!(
            "\n=== Build complete; {} wheels in {} ===",
            TARGETS.len(),
            dist.display()
        );
    }

    println!("\nDone.");
    // Always clean up the temp working dir. When --dist-dir is explicit,
    // wheels live outside `work` so they survive this cleanup.
    fs::remove_dir_all(&work).ok();
    Ok(())
}
