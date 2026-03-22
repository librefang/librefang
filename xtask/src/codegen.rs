use crate::common::repo_root;
use clap::Parser;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Parser, Debug)]
pub struct CodegenArgs {
    /// Generate OpenAPI spec only
    #[arg(long)]
    pub openapi: bool,
}

fn generate_openapi(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("Generating OpenAPI spec...");

    // Run the openapi spec test which regenerates openapi.json
    let status = Command::new("cargo")
        .args([
            "test",
            "-p",
            "librefang-api",
            "--",
            "openapi_spec",
            "--nocapture",
        ])
        .current_dir(root)
        .status()?;

    if !status.success() {
        return Err("OpenAPI spec generation failed (test failed)".into());
    }

    let spec_path = root.join("openapi.json");
    if spec_path.exists() {
        let content = fs::read_to_string(&spec_path)?;
        // Count endpoints
        let endpoint_count = content.matches("\"operationId\"").count();
        println!("  Generated openapi.json ({} endpoints)", endpoint_count);
    } else {
        println!("  Warning: openapi.json not found after generation");
    }

    Ok(())
}

pub fn run(args: CodegenArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    // If no specific flag, run all codegen
    let run_all = !args.openapi;

    if run_all || args.openapi {
        generate_openapi(&root)?;
    }

    println!();
    println!("Code generation complete.");
    Ok(())
}
