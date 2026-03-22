use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
pub struct ApiDocsArgs {
    /// Output directory for generated docs
    #[arg(long, default_value = "api-docs")]
    pub output: String,

    /// Open in browser after generation
    #[arg(long)]
    pub open: bool,

    /// Regenerate openapi.json before building docs
    #[arg(long)]
    pub refresh: bool,
}

fn repo_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("cannot get cwd");
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = fs::read_to_string(&cargo_toml).unwrap_or_default();
            if content.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            panic!("could not find workspace root");
        }
    }
}

fn find_openapi_spec(root: &Path) -> Option<PathBuf> {
    let candidates = [
        root.join("openapi.json"),
        root.join("crates/librefang-api/openapi.json"),
        root.join("docs/openapi.json"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

pub fn run(args: ApiDocsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let output_dir = root.join(&args.output);

    // Optionally regenerate spec
    if args.refresh {
        println!("Regenerating OpenAPI spec...");
        let status = Command::new("cargo")
            .args(["test", "-p", "librefang-api", "--", "openapi_spec"])
            .current_dir(&root)
            .status()?;
        if !status.success() {
            return Err("Failed to regenerate OpenAPI spec".into());
        }
        println!();
    }

    // Find the spec file
    let spec_path = find_openapi_spec(&root)
        .ok_or("openapi.json not found — run 'cargo xtask codegen --openapi' first")?;

    println!("OpenAPI spec: {}", spec_path.display());

    // Read spec to get some stats
    let spec_content = fs::read_to_string(&spec_path)?;
    let spec: serde_json::Value = serde_json::from_str(&spec_content)?;

    let endpoint_count = spec["paths"]
        .as_object()
        .map(|p| {
            p.values()
                .map(|v| v.as_object().map(|m| m.len()).unwrap_or(0))
                .sum::<usize>()
        })
        .unwrap_or(0);

    let title = spec["info"]["title"].as_str().unwrap_or("LibreFang API");
    let version = spec["info"]["version"].as_str().unwrap_or("unknown");

    println!("  Title: {}", title);
    println!("  Version: {}", version);
    println!("  Endpoints: {}", endpoint_count);

    // Generate HTML docs using redocly or swagger-ui
    fs::create_dir_all(&output_dir)?;

    // Generate a standalone Swagger UI HTML page
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title} — API Documentation</title>
    <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script>
        SwaggerUIBundle({{
            url: "openapi.json",
            dom_id: '#swagger-ui',
            deepLinking: true,
            presets: [
                SwaggerUIBundle.presets.apis,
                SwaggerUIBundle.SwaggerUIStandalonePreset
            ],
            layout: "BaseLayout"
        }});
    </script>
</body>
</html>"#
    );

    let index_path = output_dir.join("index.html");
    fs::write(&index_path, html)?;

    // Copy spec file
    let dest_spec = output_dir.join("openapi.json");
    fs::copy(&spec_path, &dest_spec)?;

    println!("\n  Generated: {}", output_dir.display());
    println!("  Open: {}", index_path.display());

    if args.open {
        let _ = Command::new("open")
            .arg(&index_path)
            .status()
            .or_else(|_| Command::new("xdg-open").arg(&index_path).status());
    }

    Ok(())
}
