use crate::common::repo_root;
use crate::local_check_mode;
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

    // Run the openapi spec test which regenerates openapi.json.
    // The `--test openapi_spec_test` flag selects the integration test binary
    // by file name; we don't also pass a name filter because the test
    // function is `generate_openapi_json`, and an unmatched filter silently
    // turns the command into a no-op that leaves a stale openapi.json on disk.
    let status = Command::new("cargo")
        .args([
            "test",
            "-p",
            "librefang-api",
            "--test",
            "openapi_spec_test",
            "--",
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
    local_check_mode::apply_for_subcommand("codegen");

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
