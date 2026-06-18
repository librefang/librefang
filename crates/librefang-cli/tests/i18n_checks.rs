use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// Checks if a line contains a potentially untranslated (hardcoded) string.
/// It extracts all string literals in quotes (ignoring escaped quotes)
/// and evaluates their content against exclusions.
fn is_potential_untranslated_string(line: &str) -> bool {
    let mut literals = Vec::new();
    let mut in_quote = false;
    let mut current_literal = String::new();
    let mut chars = line.chars().peekable();

    // A simple state machine to parse string literals in quotes "..."
    while let Some(c) = chars.next() {
        if c == '"' {
            if in_quote {
                literals.push(current_literal.clone());
                current_literal.clear();
                in_quote = false;
            } else {
                in_quote = true;
            }
        } else if in_quote {
            // Handle escaped characters (e.g., \")
            if c == '\\' {
                if let Some(next_c) = chars.next() {
                    current_literal.push(next_c);
                }
            } else {
                current_literal.push(c);
            }
        }
    }

    for lit in literals {
        let trimmed = lit.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Skip service/decorator characters, separators, and formatting elements
        if trimmed == "+"
            || trimmed == "-"
            || trimmed == "*"
            || trimmed == ":"
            || trimmed == ">>"
            || trimmed == "<<"
            || trimmed == "  +"
            || trimmed == "fix:"
            || trimmed == "try:"
            || trimmed == "hint:"
            || trimmed == "  "
            || trimmed == "\n"
        {
            continue;
        }

        // Skip Ratatui box-drawing characters and shapes
        if trimmed.contains('\u{2500}')
            || trimmed.contains('\u{25b8}')
            || trimmed.contains('\u{25cf}')
            || trimmed.contains('\u{25cb}')
        {
            continue;
        }

        // Skip empty or simple Rust formatting placeholders (e.g., "{}")
        if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.contains(':') {
            continue;
        }
        if trimmed == "{label}:"
            || trimmed == "{:<13}{}"
            || trimmed == "{:<22}{}"
            || trimmed == "{:<14} ({})"
        {
            continue;
        }

        // Skip technical identifiers, env vars, config keys, and command names which shouldn't be localized.
        let exclusions = [
            "en",
            "zh-CN",
            "uk",
            "fr",
            "LANGUAGE",
            "LC_ALL",
            "LC_MESSAGES",
            "LANG",
            "config.toml",
            "log_level",
            "log_dir",
            "language",
            "librefang",
            "start",
            "stop",
            "restart",
            "status",
            "doctor",
            "completion",
            "gateway",
            "cron",
            "workflows",
            "trigger",
            "skills",
            "channel",
            "hand",
            "config",
            "chat",
            "agents",
            "completion",
            "mcp",
            "acp",
            "auth",
            "vault",
            "new",
            "models",
            "approvals",
            "sessions",
            "logs",
            "health",
            "security",
            "memory",
            "devices",
            "qr",
            "webhooks",
            "onboard",
            "setup",
            "configure",
            "message",
            "system",
            "service",
            "reset",
            "uninstall",
            "hash-password",
        ];
        if exclusions.contains(&trimmed) {
            continue;
        }

        // If alphabetic characters remain, it's likely a user-facing string (e.g. English text).
        if trimmed.chars().any(|c| c.is_alphabetic()) {
            return true;
        }
    }
    false
}

#[test]
fn test_no_untranslated_strings() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let src_dir = Path::new(&manifest_dir).join("src");

    let mut violations = Vec::new();

    // UI helper methods that output to terminal. All strings passed to them must be localized.
    let ui_methods = [
        "check_ok",
        "check_warn",
        "check_fail",
        "step",
        "success",
        "error",
        "section",
        "kv",
        "kv_ok",
        "kv_warn",
        "hint",
        "next_steps",
        "suggest_cmd",
        "error_with_fix",
        "warn_with_fix",
        "provider_status",
    ];

    for entry in WalkDir::new(&src_dir) {
        let entry = entry.unwrap();
        let path = entry.path();

        // Only scan Rust source files
        if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            let rel_path = path
                .strip_prefix(&src_dir)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");

            // SKIP localization infrastructure and display helper files:
            // - `i18n.rs` — translation registry itself
            // - `ui.rs` — generic terminal output styling utilities
            // - `mod.rs` — module entries
            // - `acp.rs` — internal low-level ACP structures
            // - `progress.rs` — generic progress/spinner widgets
            if rel_path == "i18n.rs"
                || rel_path == "ui.rs"
                || rel_path == "mod.rs"
                || rel_path == "acp.rs"
                || rel_path == "progress.rs"
            {
                continue;
            }

            // SKIP other commands in the commands/ subdirectory except doctor_cmd.rs.
            // This allows incremental translation rollout. Other commands will be added
            // as they are migrated to Fluent.
            if rel_path.starts_with("commands/") && rel_path != "commands/doctor_cmd.rs" {
                continue;
            }

            // SKIP TUI interface as it has its own rendering logic
            if rel_path.starts_with("tui/") {
                continue;
            }

            let content = fs::read_to_string(path).unwrap();
            let lines: Vec<&str> = content.lines().collect();

            let mut in_test_mod = false;
            let mut brace_count = 0;

            for (line_num, line) in lines.iter().enumerate().map(|(i, l)| (i + 1, l.trim())) {
                // Skip comments
                if line.starts_with("//") || line.starts_with("/*") || line.starts_with("*") {
                    continue;
                }

                // Skip inline test modules (mod tests or #[cfg(test)]) since raw strings are allowed in tests
                if line.contains("mod tests") || line.contains("#[cfg(test)]") {
                    in_test_mod = true;
                    brace_count = 0;
                }

                if in_test_mod {
                    // Track test module block closure by brace matching
                    brace_count += line.chars().filter(|&c| c == '{').count() as i32;
                    brace_count -= line.chars().filter(|&c| c == '}').count() as i32;
                    if brace_count <= 0 && line.contains('}') {
                        in_test_mod = false;
                    }
                    continue;
                }

                // Check for standard print macros
                let has_print = line.contains("println!")
                    || line.contains("print!")
                    || line.contains("eprintln!")
                    || line.contains("eprint!");

                // Check for ui::* helper calls
                let has_ui = ui_methods
                    .iter()
                    .any(|m| line.contains(&format!("ui::{m}")));

                // Check for progress bar message updates
                let has_progress = line.contains("progress::auto")
                    || line.contains("p.set_message")
                    || line.contains("p.finish");

                // If an output call is found containing a literal " but not wrapped in i18n::t or i18n::t_args, analyze it further.
                if (has_print || has_ui || has_progress)
                    && line.contains('"')
                    && !line.contains("i18n::t")
                    && !line.contains("i18n::t_args")
                    && is_potential_untranslated_string(line)
                {
                    violations.push(format!(
                        "{}:{} -> {}",
                        path.strip_prefix(&manifest_dir).unwrap().display(),
                        line_num,
                        line
                    ));
                }
            }
        }
    }

    // Panic if any untranslated user-facing strings are found
    if !violations.is_empty() {
        panic!(
            "Found untranslated user-facing strings in CLI commands:\n{}",
            violations.join("\n")
        );
    }
}
