use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct ValidateConfigArgs {
    /// Path to config file (default: ~/.librefang/config.toml)
    #[arg(long)]
    pub config: Option<String>,

    /// Show the parsed config
    #[arg(long)]
    pub show: bool,
}

fn default_config_path() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| PathBuf::from(h).join(".librefang").join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

pub fn run(args: ValidateConfigArgs) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = args
        .config
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);

    println!("Validating: {}", config_path.display());

    if !config_path.exists() {
        println!("  Config file not found.");
        println!("  This is OK — LibreFang uses defaults when no config exists.");
        return Ok(());
    }

    let content = fs::read_to_string(&config_path)?;

    // Parse as TOML
    let parsed: Result<toml_edit::DocumentMut, _> = content.parse();
    match parsed {
        Ok(doc) => {
            println!("  Syntax: OK (valid TOML)");

            // Check for known sections
            let known_sections = [
                "llm",
                "budget",
                "network",
                "channels",
                "api",
                "logging",
                "agents",
                "extensions",
                "memory",
            ];

            let mut found = Vec::new();
            let mut unknown = Vec::new();

            for (key, _) in doc.iter() {
                if known_sections.contains(&key) {
                    found.push(key.to_string());
                } else {
                    unknown.push(key.to_string());
                }
            }

            if !found.is_empty() {
                println!("  Sections: {}", found.join(", "));
            }

            if !unknown.is_empty() {
                println!("  Warning: unknown sections: {}", unknown.join(", "));
                println!("  (These will be ignored by LibreFang)");
            }

            // Validate specific fields
            if let Some(llm) = doc.get("llm") {
                if let Some(provider) = llm.get("provider") {
                    let p = provider.as_str().unwrap_or("");
                    let valid_providers = [
                        "groq",
                        "openai",
                        "anthropic",
                        "ollama",
                        "openrouter",
                        "lmstudio",
                    ];
                    if !valid_providers.contains(&p) {
                        println!("  Warning: unknown LLM provider '{}'", p);
                    }
                }
            }

            if let Some(budget) = doc.get("budget") {
                if let Some(limit) = budget.get("daily_limit_usd") {
                    if let Some(v) = limit.as_float() {
                        if v < 0.0 {
                            println!("  Error: budget.daily_limit_usd cannot be negative");
                            return Err("invalid config".into());
                        }
                    }
                }
            }

            if let Some(api) = doc.get("api") {
                if let Some(port) = api.get("port") {
                    if let Some(p) = port.as_integer() {
                        if !(1..=65535).contains(&p) {
                            println!("  Error: api.port must be 1-65535, got {}", p);
                            return Err("invalid config".into());
                        }
                    }
                }
            }

            if args.show {
                println!("\n--- Config Contents ---");
                println!("{}", content);
                println!("--- End ---");
            }

            println!("\n  Config is valid.");
        }
        Err(e) => {
            println!("  Syntax: INVALID");
            println!("  Error: {}", e);
            return Err("config.toml has syntax errors".into());
        }
    }

    Ok(())
}
