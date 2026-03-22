use crate::common::repo_root;
use clap::Parser;
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
pub struct DoctorArgs {
    /// Port to check (default: 4545)
    #[arg(long, default_value = "4545")]
    pub port: u16,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

fn check_tool(name: &str) -> Option<String> {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn check_port(port: u16) -> bool {
    TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}

fn check_api_key_env(name: &str) -> &'static str {
    match std::env::var(name) {
        Ok(v) if v.is_empty() => "set but empty",
        Ok(v) if v.len() < 10 => "set but suspiciously short",
        Ok(_) => "set",
        Err(_) => "not set",
    }
}

fn check_config(config_path: &Path) -> Vec<String> {
    let mut issues = Vec::new();

    if !config_path.exists() {
        issues.push("config.toml not found — using defaults".to_string());
        return issues;
    }

    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            issues.push(format!("cannot read config.toml: {}", e));
            return issues;
        }
    };

    // Try to parse as TOML
    if content.parse::<toml_edit::DocumentMut>().is_err() {
        issues.push("config.toml has invalid TOML syntax".to_string());
    }

    // Check for common issues
    if content.contains("YOUR_API_KEY") || content.contains("sk-xxx") {
        issues.push("config.toml contains placeholder API key".to_string());
    }

    issues
}

pub fn run(args: DoctorArgs) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    let mut warnings = 0;
    let mut errors = 0;

    println!("=== LibreFang Doctor ===");
    println!();

    // 1. Toolchain
    println!("--- Toolchain ---");
    let tools: Vec<(&str, bool)> = vec![
        ("cargo", true),
        ("rustup", true),
        ("pnpm", false),
        ("gh", false),
        ("docker", false),
        ("curl", true),
    ];

    for (name, required) in &tools {
        match check_tool(name) {
            Some(ver) => println!("  OK    {} ({})", name, ver),
            None => {
                if *required {
                    println!("  ERR   {} — required but not found", name);
                    errors += 1;
                } else {
                    println!("  WARN  {} — not found (optional)", name);
                    warnings += 1;
                }
            }
        }
    }
    println!();

    // 2. Port status
    println!("--- Port {} ---", args.port);
    if check_port(args.port) {
        println!("  IN USE — something is listening on port {}", args.port);

        // Try health check
        let health_url = format!("http://127.0.0.1:{}/api/health", args.port);
        let output = Command::new("curl")
            .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", &health_url])
            .output();

        match output {
            Ok(o) if String::from_utf8_lossy(&o.stdout).trim() == "200" => {
                println!("  OK    LibreFang daemon is running and healthy");
            }
            _ => {
                println!("  WARN  Port is in use but not responding to health check");
                warnings += 1;
            }
        }
    } else {
        println!("  FREE  — daemon is not running");
    }
    println!();

    // 3. Config
    println!("--- Config ---");
    let home = home_dir();
    if let Some(ref home) = home {
        let librefang_dir = home.join(".librefang");
        let config_path = librefang_dir.join("config.toml");

        if librefang_dir.exists() {
            println!("  OK    {} exists", librefang_dir.display());
        } else {
            println!("  WARN  {} not found", librefang_dir.display());
            warnings += 1;
        }

        let config_issues = check_config(&config_path);
        if config_issues.is_empty() {
            println!("  OK    config.toml is valid");
        } else {
            for issue in &config_issues {
                println!("  WARN  {}", issue);
                warnings += 1;
            }
        }
    }
    println!();

    // 4. API Keys
    println!("--- API Keys ---");
    let api_keys = ["GROQ_API_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"];
    for key in &api_keys {
        let status = check_api_key_env(key);
        let prefix = if status == "set" { "OK   " } else { "INFO " };
        println!("  {}  {} — {}", prefix, key, status);
    }
    println!();

    // 5. Workspace
    println!("--- Workspace ---");
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.exists() {
        println!("  OK    Cargo.toml found at {}", root.display());
    } else {
        println!("  ERR   Cargo.toml not found");
        errors += 1;
    }

    let cargo_lock = root.join("Cargo.lock");
    if cargo_lock.exists() {
        println!("  OK    Cargo.lock present");
    } else {
        println!("  WARN  Cargo.lock missing — run cargo fetch");
        warnings += 1;
    }

    // Check binary
    let binary = if cfg!(target_os = "windows") {
        root.join("target/release/librefang.exe")
    } else {
        root.join("target/release/librefang")
    };
    if binary.exists() {
        println!("  OK    Release binary exists: {}", binary.display());
    } else {
        println!("  INFO  No release binary — build with: cargo build --release -p librefang-cli");
    }

    // Check git status
    let git_status = Command::new("git")
        .args(["status", "--short"])
        .current_dir(&root)
        .output();
    if let Ok(output) = git_status {
        let lines = String::from_utf8_lossy(&output.stdout);
        let dirty_count = lines.lines().count();
        if dirty_count == 0 {
            println!("  OK    Working tree is clean");
        } else {
            println!("  INFO  {} uncommitted change(s)", dirty_count);
        }
    }
    println!();

    // Summary
    println!("=== Summary ===");
    if errors > 0 {
        println!("  {} error(s), {} warning(s)", errors, warnings);
        Err(format!("{} error(s) found", errors).into())
    } else if warnings > 0 {
        println!("  {} warning(s), no errors", warnings);
        Ok(())
    } else {
        println!("  All checks passed!");
        Ok(())
    }
}
