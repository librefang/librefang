use clap::Parser;
use std::fs;
use std::path::PathBuf;

const REDACTED: &str = "[REDACTED]";

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["api_key", "secret", "password", "credential", "private_key"]
        .iter()
        .any(|marker| key.contains(marker))
        || key == "token"
        || key.ends_with("_token")
}

fn redact_value(value: &mut toml_edit::Value) {
    match value {
        toml_edit::Value::Array(array) => {
            for value in array.iter_mut() {
                redact_value(value);
            }
        }
        toml_edit::Value::InlineTable(table) => {
            for (key, value) in table.iter_mut() {
                if is_sensitive_key(&key) {
                    *value = toml_edit::Value::from(REDACTED);
                } else {
                    redact_value(value);
                }
            }
        }
        _ => {}
    }
}

fn redact_item(item: &mut toml_edit::Item) {
    match item {
        toml_edit::Item::Value(value) => redact_value(value),
        toml_edit::Item::Table(table) => {
            for (key, item) in table.iter_mut() {
                if is_sensitive_key(&key) {
                    *item = toml_edit::value(REDACTED);
                } else {
                    redact_item(item);
                }
            }
        }
        toml_edit::Item::ArrayOfTables(tables) => {
            for table in tables.iter_mut() {
                for (key, item) in table.iter_mut() {
                    if is_sensitive_key(&key) {
                        *item = toml_edit::value(REDACTED);
                    } else {
                        redact_item(item);
                    }
                }
            }
        }
        toml_edit::Item::None => {}
    }
}

fn redacted_config(doc: &toml_edit::DocumentMut) -> String {
    let mut redacted = doc.clone();
    for (key, item) in redacted.iter_mut() {
        if is_sensitive_key(&key) {
            *item = toml_edit::value(REDACTED);
        } else {
            redact_item(item);
        }
    }
    redacted.to_string()
}

fn validate_known_fields(doc: &toml_edit::DocumentMut) -> Result<(), String> {
    if let Some(provider) = doc.get("llm").and_then(|section| section.get("provider")) {
        if provider.as_str().is_none() {
            return Err("llm.provider must be a string".to_string());
        }
    }

    if let Some(limit) = doc
        .get("budget")
        .and_then(|section| section.get("daily_limit_usd"))
    {
        let Some(value) = limit.as_float() else {
            return Err("budget.daily_limit_usd must be a floating-point number".to_string());
        };
        if value < 0.0 {
            return Err("budget.daily_limit_usd cannot be negative".to_string());
        }
    }

    if let Some(port) = doc.get("api").and_then(|section| section.get("port")) {
        let Some(value) = port.as_integer() else {
            return Err("api.port must be an integer".to_string());
        };
        if !(1..=65535).contains(&value) {
            return Err(format!("api.port must be 1-65535, got {value}"));
        }
    }

    Ok(())
}

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
            if let Err(error) = validate_known_fields(&doc) {
                println!("  Error: {error}");
                return Err("invalid config".into());
            }

            if let Some(llm) = doc.get("llm") {
                if let Some(provider) = llm.get("provider") {
                    let p = provider
                        .as_str()
                        .expect("type checked by validate_known_fields");
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

            if args.show {
                println!("\n--- Config Contents (secrets redacted) ---");
                println!("{}", redacted_config(&doc));
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

#[cfg(test)]
mod tests {
    use super::{is_sensitive_key, redacted_config, validate_known_fields, REDACTED};

    #[test]
    fn sensitive_key_matching_covers_nested_config_secret_names() {
        for key in [
            "api_key",
            "client_secret",
            "admin_password",
            "access_token",
            "credentials",
            "private_key_path",
        ] {
            assert!(is_sensitive_key(key), "{key} must be redacted");
        }
        for key in ["provider", "model", "client_id", "token_budget"] {
            assert!(!is_sensitive_key(key), "{key} must remain visible");
        }
    }

    #[test]
    fn config_display_redacts_tables_inline_tables_and_arrays() {
        let doc = r#"
api_key = "root-secret"
provider = "openai"

[network]
shared_secret = "network-secret"

[[extensions]]
name = "visible"
credentials = "extension-secret"
settings = { password = "inline-secret", region = "visible-region" }
rules = [{ access_token = "array-secret", label = "visible-label" }]
"#
        .parse::<toml_edit::DocumentMut>()
        .unwrap();

        let shown = redacted_config(&doc);

        for secret in [
            "root-secret",
            "network-secret",
            "extension-secret",
            "inline-secret",
            "array-secret",
        ] {
            assert!(!shown.contains(secret), "leaked {secret}");
        }
        assert_eq!(shown.matches(REDACTED).count(), 5);
        for visible in ["openai", "visible", "visible-region", "visible-label"] {
            assert!(shown.contains(visible), "lost {visible}");
        }
    }

    #[test]
    fn known_fields_reject_wrong_toml_types() {
        for (source, expected) in [
            ("[llm]\nprovider = 42", "llm.provider must be a string"),
            (
                "[budget]\ndaily_limit_usd = \"ten\"",
                "budget.daily_limit_usd must be a floating-point number",
            ),
            ("[api]\nport = \"4545\"", "api.port must be an integer"),
        ] {
            let doc = source.parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(validate_known_fields(&doc).unwrap_err(), expected);
        }
    }

    #[test]
    fn known_fields_retain_range_validation() {
        let negative_budget = "[budget]\ndaily_limit_usd = -1.0"
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert_eq!(
            validate_known_fields(&negative_budget).unwrap_err(),
            "budget.daily_limit_usd cannot be negative"
        );

        let invalid_port = "[api]\nport = 70000"
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert_eq!(
            validate_known_fields(&invalid_port).unwrap_err(),
            "api.port must be 1-65535, got 70000"
        );
    }
}
