//! Shell bleed detection — scan script files for environment variable leaks.
//!
//! When an agent runs `python3 script.py` or `bash run.sh`, the script file
//! may reference environment variables that contain secrets. This module scans
//! the script file for env var patterns and returns warnings.

use std::path::{Path, PathBuf};
use tracing::debug;

/// Warning about a potential environment variable leak in a script.
#[derive(Debug, Clone)]
pub struct ShellBleedWarning {
    /// Script file that contains the leak.
    pub file: PathBuf,
    /// Line number (1-indexed) where the pattern was found.
    pub line_number: usize,
    /// The matched pattern (e.g., "$OPENAI_API_KEY").
    pub pattern: String,
    /// Suggestion for fixing the leak.
    pub suggestion: String,
}

/// Environment variables that are safe to reference in scripts.
const SAFE_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "TERM",
    "USER",
    "LOGNAME",
    "SHELL",
    "PWD",
    "OLDPWD",
    "HOSTNAME",
    "DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "APPDATA",
    "LOCALAPPDATA",
    "COMSPEC",
    "WINDIR",
    "PATHEXT",
    "PYTHONPATH",
    "NODE_PATH",
    "GOPATH",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "VIRTUAL_ENV",
    "CONDA_DEFAULT_ENV",
    "PYTHONUNBUFFERED",
    "CI",
    "GITHUB_ACTIONS",
    "GITHUB_WORKSPACE",
    "GITHUB_SHA",
    "GITHUB_REF",
];

/// Maximum script file size to scan (100 KB).
const MAX_SCRIPT_SIZE: usize = 100 * 1024;

/// Patterns that suggest a script file path in a command.
const SCRIPT_EXTENSIONS: &[&str] = &[".py", ".sh", ".bash", ".rb", ".pl", ".js", ".ts", ".ps1"];

/// Extract the script file path from a command string, if any.
///
/// Handles patterns like:
/// - `python3 script.py`
/// - `bash -c ./run.sh`
/// - `node app.js`
fn extract_script_path(command: &str) -> Option<String> {
    for part in command.split_whitespace().skip(1) {
        // Skip flags
        if part.starts_with('-') {
            continue;
        }
        // Check if this looks like a script file
        for ext in SCRIPT_EXTENSIONS {
            if part.ends_with(ext) {
                return Some(part.to_string());
            }
        }
    }
    None
}

/// Scan a script file for environment variable references that may leak secrets.
///
/// Returns a list of warnings for each potential leak found.
/// Does not block execution — warnings are prepended to the tool result.
pub fn scan_script_for_shell_bleed(
    command: &str,
    workspace_root: Option<&Path>,
) -> Vec<ShellBleedWarning> {
    let script_path = match extract_script_path(command) {
        Some(p) => p,
        None => return Vec::new(),
    };

    // Resolve relative to workspace root
    let full_path = if let Some(root) = workspace_root {
        root.join(&script_path)
    } else {
        PathBuf::from(&script_path)
    };

    // Read the script file
    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(_) => {
            debug!(path = %full_path.display(), "Cannot read script file for shell bleed scan");
            return Vec::new();
        }
    };

    // Size limit
    if content.len() > MAX_SCRIPT_SIZE {
        debug!(
            path = %full_path.display(),
            size = content.len(),
            "Script too large for shell bleed scan"
        );
        return Vec::new();
    }

    let mut warnings = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        // Skip comments
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with("//") || trimmed.starts_with("--") {
            continue;
        }

        // Scan for env var patterns: $VAR, ${VAR}, os.environ["VAR"],
        // os.getenv("VAR"), process.env.VAR, ENV["VAR"]
        let env_vars = extract_env_var_refs(line);

        for var_name in env_vars {
            // Skip safe vars
            if SAFE_VARS.contains(&var_name.as_str()) {
                continue;
            }

            // Flag vars that look like secrets
            let lower = var_name.to_lowercase();
            let is_suspicious = lower.contains("key")
                || lower.contains("secret")
                || lower.contains("token")
                || lower.contains("password")
                || lower.contains("credential")
                || lower.contains("auth")
                || lower.contains("api_key")
                || lower.contains("apikey");

            if is_suspicious {
                warnings.push(ShellBleedWarning {
                    file: full_path.clone(),
                    line_number: line_idx + 1,
                    pattern: var_name.clone(),
                    suggestion: format!(
                        "Consider passing '{}' as a tool parameter instead of reading it from the environment.",
                        var_name
                    ),
                });
            }
        }
    }

    warnings
}

/// Extract environment variable references from a line of code.
fn extract_env_var_refs(line: &str) -> Vec<String> {
    let mut vars = Vec::new();

    // Pattern: $VAR_NAME or ${VAR_NAME} (shell/bash)
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            let mut var = String::new();
            if chars.peek() == Some(&'{') {
                chars.next(); // consume '{'
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    var.push(c);
                }
            } else {
                for c in chars.by_ref() {
                    if c.is_alphanumeric() || c == '_' {
                        var.push(c);
                    } else {
                        break;
                    }
                }
            }
            if !var.is_empty() {
                vars.push(var);
            }
        }
    }

    // Pattern: os.environ["VAR"] or os.getenv("VAR") (Python)
    for pattern in &[
        "os.environ[\"",
        "os.environ['",
        "os.getenv(\"",
        "os.getenv('",
    ] {
        let mut search_from = 0;
        while let Some(pos) = line[search_from..].find(pattern) {
            let start = search_from + pos + pattern.len();
            let quote_char = if pattern.ends_with('"') { '"' } else { '\'' };
            if let Some(end) = line[start..].find(quote_char) {
                let var = &line[start..start + end];
                if !var.is_empty() {
                    vars.push(var.to_string());
                }
                search_from = start + end;
            } else {
                break;
            }
        }
    }

    // Pattern: process.env.VAR (Node.js)
    let mut search_from = 0;
    while let Some(pos) = line[search_from..].find("process.env.") {
        let start = search_from + pos + "process.env.".len();
        let var: String = line[start..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !var.is_empty() {
            vars.push(var);
        }
        search_from = start;
    }

    vars
}

/// Format warnings for prepending to a tool result.
pub fn format_warnings(warnings: &[ShellBleedWarning]) -> String {
    if warnings.is_empty() {
        return String::new();
    }

    let mut output = String::from("[SHELL BLEED WARNING] The script references environment variables that may contain secrets:\n");
    for w in warnings {
        output.push_str(&format!(
            "  - {} (line {}): ${} — {}\n",
            w.file.display(),
            w.line_number,
            w.pattern,
            w.suggestion
        ));
    }
    output.push_str("Consider using tool parameters or a .env file instead.\n\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_env_var_refs_shell() {
        let vars = extract_env_var_refs("echo $OPENAI_API_KEY and ${SECRET_TOKEN}");
        assert!(vars.contains(&"OPENAI_API_KEY".to_string()));
        assert!(vars.contains(&"SECRET_TOKEN".to_string()));
    }

    #[test]
    fn test_extract_env_var_refs_python() {
        let vars = extract_env_var_refs("key = os.environ[\"OPENAI_API_KEY\"]");
        assert!(vars.contains(&"OPENAI_API_KEY".to_string()));

        let vars = extract_env_var_refs("key = os.getenv('SECRET_TOKEN')");
        assert!(vars.contains(&"SECRET_TOKEN".to_string()));
    }

    #[test]
    fn test_extract_env_var_refs_node() {
        let vars = extract_env_var_refs("const key = process.env.API_KEY");
        assert!(vars.contains(&"API_KEY".to_string()));
    }

    #[test]
    fn test_safe_vars_excluded() {
        // PATH is safe, should not generate a warning
        assert!(SAFE_VARS.contains(&"PATH"));
        assert!(SAFE_VARS.contains(&"HOME"));
    }

    #[test]
    fn test_extract_script_path() {
        assert_eq!(
            extract_script_path("python3 script.py"),
            Some("script.py".to_string())
        );
        assert_eq!(
            extract_script_path("node app.js"),
            Some("app.js".to_string())
        );
        assert_eq!(extract_script_path("ls -la"), None);
        assert_eq!(
            extract_script_path("bash -c ./run.sh"),
            Some("./run.sh".to_string())
        );
    }

    #[test]
    fn test_scan_nonexistent_script() {
        let warnings = scan_script_for_shell_bleed("python3 nonexistent.py", None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_scan_non_script_command() {
        let warnings = scan_script_for_shell_bleed("ls -la", None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_scan_empty_commands() {
        for command in ["", " \t\n"] {
            assert!(scan_script_for_shell_bleed(command, None).is_empty());
        }
    }

    #[test]
    fn test_format_warnings_empty() {
        assert_eq!(format_warnings(&[]), "");
    }

    #[test]
    fn test_format_warnings_has_content() {
        let warnings = vec![ShellBleedWarning {
            file: PathBuf::from("test.py"),
            line_number: 5,
            pattern: "API_KEY".to_string(),
            suggestion: "Use tool params".to_string(),
        }];
        let output = format_warnings(&warnings);
        assert!(output.contains("SHELL BLEED WARNING"));
        assert!(output.contains("API_KEY"));
        assert!(output.contains("line 5"));
    }
}
