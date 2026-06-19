use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// Checks if a line contains a potentially untranslated (hardcoded) string.
/// It extracts all string literals in quotes (ignoring escaped quotes)
/// and evaluates their content against exclusions.
fn is_potential_untranslated_literal(lit: &str) -> bool {
    let trimmed = lit.trim();
    if trimmed.is_empty() {
        return false;
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
        return false;
    }

    // Skip Ratatui box-drawing characters and shapes
    if trimmed.contains('\u{2500}')
        || trimmed.contains('\u{25b8}')
        || trimmed.contains('\u{25cf}')
        || trimmed.contains('\u{25cb}')
    {
        return false;
    }

    // Skip empty or simple Rust formatting placeholders (e.g., "{}")
    if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.contains(':') {
        return false;
    }
    if trimmed == "{label}:"
        || trimmed == "{:<13}{}"
        || trimmed == "{:<22}{}"
        || trimmed == "{:<14} ({})"
        || trimmed == "librefang {}"
        || trimmed == "# {}"
        || trimmed == "# {}n"
    {
        return false;
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
        "CARGO_PKG_VERSION",
        "CARGO_PKG_NAME",
        // Additional technical exclusions
        "channel list",
        "channel reload",
        "channel setup",
        "channel rm",
        "pip install librefang-sdk",
        "models list",
        "models set",
        "models aliases",
        "models providers",
        "approvals list",
        "approvals respond",
        "approvals approve",
        "approvals reject",
        "workflow list",
        "workflow create",
        "workflow run",
        "trigger list",
        "trigger create",
        "trigger delete",
        "trigger get",
        "trigger update",
        "trigger enable",
        "trigger disable",
        "cron list",
        "cron create",
        "cron delete",
        "Unknown error",
        "cargo install --git https://github.com/{RELEASE_REPO} --tag {tag} librefang-cli --force",
        "cargo install --git https://github.com/{RELEASE_REPO} librefang-cli --force",
        "$env:LIBREFANG_VERSION='{tag}'; irm {POWERSHELL_INSTALLER_URL} | iex",
        "irm {POWERSHELL_INSTALLER_URL} | iex",
        "curl -fsSL {SHELL_INSTALLER_URL} | LIBREFANG_VERSION={tag} sh",
        "curl -fsSL {SHELL_INSTALLER_URL} | sh",
        "[Environment]::GetEnvironmentVariable('PATH', 'User')",
        "[Environment]::SetEnvironmentVariable('PATH', '{}', 'User')",
        "export path=",
        "export path =",
        "set -gx path",
        "Could not rename binary for deferred deletion: {}",
        "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"",
        "Start-Sleep -Seconds 1rn{script}rnRemove-Item $MyInvocation.MyCommand.Path -ErrorAction SilentlyContinuern",
        "\"{}\" start",
        "{trimmed} trace_id={:032x}",
        "baseline directive must parse",
        "error: failed writing to stdout: {e}",
        "Bearer {key}",
        "Failed to build HTTP client",
        "timed out",
        "Connection refused",
        "Set-Clipboard '{}'",
        "not found",
        // Menu item labels and hints (comments only, dynamic translation done elsewhere)
        "Get started",
        "Providers, API keys, models, migration",
        "Chat with an agent",
        "Quick chat in the terminal",
        "Open dashboard",
        "Launch the web UI in your browser",
        "Open desktop app",
        "Launch the native desktop app",
        "Launch terminal UI",
        "Full interactive TUI dashboard",
        "Show all commands",
        "Print full --help output",
        "Settings",
        "Providers, API keys, models, routing",
        // Brand/technical display names
        // Technical init strings
        "chore: initial librefang config",
        "CLI login",
        // Developer-facing expect/panic strings
        "idx within bounds",
        "Failed to create tokio runtime",
        "Failed to create Tokio runtime",
        "HTTP blocking client with bundled CA roots should always build",
        "log filter not installed",
        "invalid log directive {directive:?}: {e}",
        "Skipping unparseable baseline log directive on reload",
        "invalid language identifier: {e}",
        "failed to parse Fluent resource: {errors:?}",
        "failed to add Fluent resource: {errors:?}",
        "Fluent formatting errors",
        "failed to initialize i18n, falling back to English",
        "default language pack must be valid",
        "failed to initialize default i18n fallback: {error}",
        "HTTP error: {e}",
        "Parse error: {e}",
        "Invalid agent ID",
        "Parse error",
        "Content-Length: {}rnrn{}",
        "Send a message to LibreFang agent '{name}'",
        "Message to send to the agent",
        "Missing 'message' argument",
        "Unknown tool: {tool_name}",
        "Error: {e}",
        "Method not found: {method}",
        "target checked above",
        "instance_id missing",
        "unhandled CLI command `{other}`",
        "Failed to draw",
        // Technical format strings
        "%Y-%m-%d %H:%M",
        // Hand CLI command names for require_daemon
        "hand install",
        "hand list",
        "hand active",
        "hand status",
        "hand activate",
        "hand deactivate",
        "hand info",
        "hand check-deps",
        "hand install-deps",
        "hand pause",
        "hand resume",
        "hand settings",
        "hand set",
        "hand reload",
        "hand chat",
        // Technical commands for monitoring and device/webhook management
        "security status",
        "security audit",
        "security verify",
        "memory list",
        "memory get",
        "memory set",
        "memory delete",
        "devices list",
        "devices remove",
        "webhooks list",
        "webhooks create",
        "webhooks delete",
        "webhooks test",
        // WASM skill scaffold/template fragments
        ") }}))\n        }}\n        other => Err(format!(",
        ")), \n    }}\n}}\n\nskill!(handle);",
        "rustup target add wasm32-unknown-unknown",
        "cargo build --release --target wasm32-unknown-unknown",
        "cp target/wasm32-unknown-unknown/release/skill.wasm skill.wasm",
        "[skill]\nname =",
        "version =",
        "description =",
        "author =",
        "license =",
        "tags = []\n\n[runtime]\ntype =",
        "entry =",
        "[[tools.provided]]\nname =",
        "input_schema = {{ type =",
        ", properties = {{ input = {{ type =",
        "}} }}, required = [",
        "] }}\n\n[requirements]\ntools = []\ncapabilities = []",
        "%Y-%m-%d %H:%M:%S UTC",
        "daemon not running",
        // progress.rs TUI formatting and spinners
        "rx1b[2K{:<14} [{}] {:>3}% ({}/{})",
        "rx1b[2K{ch} {}",
        "x1b[31m✗x1b[0m {msg}",
        // TUI theme agent state badges
        "u{25cf} RUN",
        "u{25cb} NEW",
        "u{25d4} SUS",
        "u{25cb} END",
        "u{25cf} ERR",
        "u{25cb} ---",
    ];
    if exclusions.contains(&trimmed) {
        return false;
    }

    if trimmed.contains("[Unit]") || trimmed.contains("[Service]") || trimmed.contains("[Install]") {
        return false;
    }
    if trimmed.contains("{name:<28}") {
        return false;
    }

    // If the literal does not contain a space, it's highly likely to be a technical key, path, or identifier.
    if trimmed.contains("SHA256:") && (trimmed.contains("Install with:") || trimmed.contains("Size:")) {
        return false;
    }
    if trimmed.starts_with("[{marker}]") {
        return false;
    }
    // Skip SQL statements
    let upper = trimmed.to_uppercase();
    if upper.starts_with("SELECT ")
        || upper.starts_with("INSERT ")
        || upper.starts_with("UPDATE ")
        || upper.starts_with("DELETE ")
        || upper.starts_with("CREATE ")
        || upper.starts_with("DROP ")
    {
        return false;
    }
    if !trimmed.contains(' ') {
        return false;
    }

    // If alphabetic characters remain, it's likely a user-facing string (e.g. English text).
    if trimmed.chars().any(|c| c.is_alphabetic()) {
        return true;
    }
    
    false
}

#[allow(clippy::while_let_on_iterator)]
fn scan_file_for_untranslated_strings(content: &str) -> Vec<(usize, String, String)> {
    let mut violations = Vec::new();
    let mut chars = content.char_indices().peekable();
    
    let mut in_quote = false;
    let mut current_literal = String::new();
    let mut literal_start_idx = 0;
    
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut in_test_mod = false;
    let mut test_mod_brace_depth = 0;
    let mut brace_depth = 0;
    
    let mut line_number = 1;
    
    let next_char = |chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, line_number: &mut usize| {
        if let Some((_, nc)) = chars.next() {
            if nc == '\n' {
                *line_number += 1;
            }
            Some(nc)
        } else {
            None
        }
    };

    while let Some((idx, c)) = chars.next() {
        if c == '\n' {
            line_number += 1;
            in_line_comment = false;
        }

        // Skip Rust raw string literals to prevent quote desynchronization
        let prev_char_is_ident = idx > 0 && {
            let prev = content.as_bytes()[idx - 1] as char;
            prev.is_alphanumeric() || prev == '_'
        };
        if !in_quote && !in_line_comment && !in_block_comment && c == 'r' && !prev_char_is_ident {
            let remaining = &content[idx..];
            if remaining.starts_with("r\"") {
                chars.next(); // consume '"'
                while let Some((_, rc)) = chars.next() {
                    if rc == '\n' {
                        line_number += 1;
                    }
                    if rc == '"' {
                        break;
                    }
                }
                continue;
            } else if remaining.starts_with("r#") {
                let mut hashes = 0;
                let mut temp_chars = chars.clone();
                while let Some((_, hc)) = temp_chars.next() {
                    if hc == '#' {
                        hashes += 1;
                    } else if hc == '"' {
                        break;
                    } else {
                        hashes = 0;
                        break;
                    }
                }
                if hashes > 0 {
                    for _ in 0..hashes + 1 {
                        chars.next();
                    }
                    let end_pattern = format!("\"{}", "#".repeat(hashes));
                    while let Some((inner_idx, rc)) = chars.next() {
                        if rc == '\n' {
                            line_number += 1;
                        }
                        if rc == '"' && content[inner_idx..].starts_with(&end_pattern) {
                            for _ in 0..hashes {
                                chars.next();
                            }
                            break;
                        }
                    }
                    continue;
                }
            }
        }
        
        // Handle comments
        if in_line_comment {
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.peek().map(|&(_, next_c)| next_c) == Some('/') {
                chars.next(); // consume '/'
                in_block_comment = false;
            }
            continue;
        }
        
        // Check for comment start
        if !in_quote {
            if c == '/' && chars.peek().map(|&(_, next_c)| next_c) == Some('/') {
                chars.next(); // consume '/'
                in_line_comment = true;
                continue;
            }
            if c == '/' && chars.peek().map(|&(_, next_c)| next_c) == Some('*') {
                chars.next(); // consume '*'
                in_block_comment = true;
                continue;
            }
        }
        
        // Track braces to skip mod tests block
        if !in_quote {
            if c == '{' {
                brace_depth += 1;
            } else if c == '}' {
                brace_depth -= 1;
                if in_test_mod && brace_depth < test_mod_brace_depth {
                    in_test_mod = false;
                }
            }
        }
        
        // Check for test mod declaration start
        if !in_quote && !in_test_mod {
            let remaining = &content[idx..];
            if remaining.starts_with("mod tests") || remaining.starts_with("#[cfg(test)]") {
                in_test_mod = true;
                test_mod_brace_depth = brace_depth + 1;
            }
        }
        
        if in_test_mod {
            continue;
        }
        
        // Handle character literals and lifetimes (to prevent quote desynchronization)
        if c == '\'' && !in_quote {
            let remaining = &content[idx..];
            if remaining.starts_with("'\\\\'") {
                next_char(&mut chars, &mut line_number); // \
                next_char(&mut chars, &mut line_number); // \
                next_char(&mut chars, &mut line_number); // '
                continue;
            } else if remaining.starts_with("'\\''") {
                next_char(&mut chars, &mut line_number); // \
                next_char(&mut chars, &mut line_number); // '
                next_char(&mut chars, &mut line_number); // '
                continue;
            } else if remaining.starts_with("'\\\"'") {
                next_char(&mut chars, &mut line_number); // \
                next_char(&mut chars, &mut line_number); // "
                next_char(&mut chars, &mut line_number); // '
                continue;
            } else if remaining.starts_with("'\\n'") || remaining.starts_with("'\\r'") || remaining.starts_with("'\\t'") || remaining.starts_with("'\\0'") {
                next_char(&mut chars, &mut line_number); // \
                next_char(&mut chars, &mut line_number); // char
                next_char(&mut chars, &mut line_number); // '
                continue;
            } else if remaining.starts_with("'\\u{") {
                let mut temp_chars = chars.clone();
                let mut parsed_ok = false;
                let mut chars_to_consume = 0;
                if let Some((_, '\\')) = temp_chars.next() {
                    chars_to_consume += 1;
                    if let Some((_, 'u')) = temp_chars.next() {
                        chars_to_consume += 1;
                        if let Some((_, '{')) = temp_chars.next() {
                            chars_to_consume += 1;
                            let mut found_brace = false;
                            for (_, next_c) in temp_chars.by_ref() {
                                chars_to_consume += 1;
                                if next_c == '}' {
                                    found_brace = true;
                                    break;
                                }
                                if !next_c.is_ascii_hexdigit() {
                                    break;
                                }
                            }
                            if found_brace {
                                if let Some((_, '\'')) = temp_chars.next() {
                                    chars_to_consume += 1;
                                    parsed_ok = true;
                                }
                            }
                        }
                    }
                }
                if parsed_ok {
                    for _ in 0..chars_to_consume {
                        next_char(&mut chars, &mut line_number);
                    }
                    continue;
                }
            } else if remaining.starts_with("'\\x") {
                let mut temp_chars = chars.clone();
                let mut parsed_ok = false;
                let mut chars_to_consume = 0;
                if let Some((_, '\\')) = temp_chars.next() {
                    chars_to_consume += 1;
                    if let Some((_, 'x')) = temp_chars.next() {
                        chars_to_consume += 1;
                        if let Some((_, h1)) = temp_chars.next() {
                            chars_to_consume += 1;
                            if let Some((_, h2)) = temp_chars.next() {
                                chars_to_consume += 1;
                                if h1.is_ascii_hexdigit() && h2.is_ascii_hexdigit() {
                                    if let Some((_, '\'')) = temp_chars.next() {
                                        chars_to_consume += 1;
                                        parsed_ok = true;
                                    }
                                }
                            }
                        }
                    }
                }
                if parsed_ok {
                    for _ in 0..chars_to_consume {
                        next_char(&mut chars, &mut line_number);
                    }
                    continue;
                }
            } else {
                let mut temp_chars = chars.clone();
                if let Some((_, mid_c)) = temp_chars.next() {
                    if mid_c != '\\' {
                        if let Some((_, '\'')) = temp_chars.next() {
                            next_char(&mut chars, &mut line_number); // consume mid_c
                            next_char(&mut chars, &mut line_number); // consume '\''
                            continue;
                        }
                    }
                }
            }
        }

        // Handle string literals
        if c == '"' {
            if in_quote {
                // End of string literal
                let is_byte_string = literal_start_idx > 0 && content.as_bytes()[literal_start_idx - 1] == b'b';
                let prefix = &content[..literal_start_idx];
                let collapsed: String = prefix.chars().filter(|ch| !ch.is_whitespace()).collect();
                let is_localized = collapsed.ends_with("i18n::t(")
                      || collapsed.ends_with("i18n::t_args(")
                      || collapsed.ends_with("debug!(")
                      || collapsed.ends_with("info!(")
                      || collapsed.ends_with("warn!(")
                      || collapsed.ends_with("error!(")
                      || collapsed.ends_with("trace!(")
                      || collapsed.ends_with("about=")
                      || collapsed.ends_with("long_about=")
                      || collapsed.ends_with("help=")
                      || collapsed.ends_with("after_help=")
                      || collapsed.ends_with("value_name=")
                      || collapsed.ends_with("rename_all=")
                      || collapsed.ends_with("name=")
                      || collapsed.ends_with("conflicts_with=")
                      || collapsed.ends_with("conflicts_with_all=")
                      || collapsed.ends_with("required_unless_present=")
                      || collapsed.ends_with("requires=")
                      || collapsed.ends_with("default_value=")
                      || collapsed.ends_with("env=")
                      || collapsed.ends_with("aliases=")
                      || collapsed.ends_with("alias=")
                      || collapsed.ends_with("short=")
                      || collapsed.ends_with("long=")
                      || collapsed.ends_with("constAFTER_HELP:&str=");
                if !is_byte_string && !is_localized && is_potential_untranslated_literal(&current_literal) {
                    let line_content = get_line_at_index(content, literal_start_idx);
                    violations.push((line_number, current_literal.clone(), line_content));
                }
                
                current_literal.clear();
                in_quote = false;
            } else {
                in_quote = true;
                literal_start_idx = idx;
            }
        } else if in_quote {
            if c == '\\' {
                if let Some((_, next_c)) = chars.next() {
                    if next_c == '\n' {
                        line_number += 1;
                    }
                    current_literal.push(next_c);
                }
            } else {
                current_literal.push(c);
            }
        }
    }
    violations
}

fn get_line_at_index(content: &str, index: usize) -> String {
    let start = content[..index].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = content[index..].find('\n').map(|i| index + i).unwrap_or(content.len());
    content[start..end].trim().to_string()
}

#[test]
fn test_no_untranslated_strings() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let src_dir = Path::new(&manifest_dir).join("src");

    let mut violations = Vec::new();

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

            if rel_path == "tui/mod.rs"
                || rel_path.starts_with("tui/screens/")
            {
                continue;
            }

            let content = fs::read_to_string(path).unwrap();
            let file_violations = scan_file_for_untranslated_strings(&content);
            for (line_num, literal, line_content) in file_violations {
                violations.push(format!(
                    "{}:{} -> literal \"{}\" in line: {}",
                    path.strip_prefix(&manifest_dir).unwrap().display(),
                    line_num,
                    literal,
                    line_content
                ));
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
