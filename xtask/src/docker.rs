use crate::common::repo_root;
use clap::Parser;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct DockerArgs {
    /// Image tag (default: version from Cargo.toml)
    #[arg(long)]
    pub tag: Option<String>,

    /// Push image after build
    #[arg(long)]
    pub push: bool,

    /// Target platform (default: linux/amd64)
    #[arg(long, default_value = "linux/amd64")]
    pub platform: String,

    /// Also tag as :latest
    #[arg(long)]
    pub latest: bool,
}

const IMAGE_NAME: &str = "ghcr.io/librefang/librefang";

fn read_workspace_version(root: &Path) -> String {
    let content = fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    let doc = content.parse::<toml_edit::DocumentMut>().ok();
    doc.and_then(|d| {
        d["workspace"]["package"]["version"]
            .as_str()
            .map(|s| s.to_string())
    })
    .unwrap_or_else(|| "latest".to_string())
}

pub fn run(args: DockerArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let dockerfile = root.join("Dockerfile");

    if !dockerfile.exists() {
        return Err("Dockerfile not found".into());
    }

    if Command::new("docker").arg("--version").output().is_err() {
        return Err("docker not found — install Docker first".into());
    }

    let version = args.tag.unwrap_or_else(|| read_workspace_version(&root));
    let image_tag = format!("{}:v{}", IMAGE_NAME, version);

    println!("Building Docker image...");
    println!("  Image:    {}", image_tag);
    println!("  Platform: {}", args.platform);
    println!("  Dockerfile: ./Dockerfile");
    println!();

    let mut build_args = vec![
        "build".to_string(),
        "-f".to_string(),
        "Dockerfile".to_string(),
        "-t".to_string(),
        image_tag.clone(),
        "--platform".to_string(),
        args.platform.clone(),
    ];

    if args.latest {
        build_args.push("-t".to_string());
        build_args.push(format!("{}:latest", IMAGE_NAME));
    }

    build_args.push(".".to_string());

    let status = Command::new("docker")
        .args(&build_args)
        .current_dir(&root)
        .status()?;

    if !status.success() {
        return Err("docker build failed".into());
    }

    println!();
    println!("Image built: {}", image_tag);

    if args.push {
        println!();
        println!("Pushing {}...", image_tag);

        let status = Command::new("docker")
            .args(["push", &image_tag])
            .current_dir(&root)
            .status()?;
        if !status.success() {
            return Err("docker push failed".into());
        }
        println!("  Pushed {}", image_tag);

        if args.latest {
            let latest_tag = format!("{}:latest", IMAGE_NAME);
            let status = Command::new("docker")
                .args(["push", &latest_tag])
                .current_dir(&root)
                .status()?;
            if !status.success() {
                return Err("docker push :latest failed".into());
            }
            println!("  Pushed {}", latest_tag);
        }
    }

    Ok(())
}
