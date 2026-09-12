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
        "models overrides",
        "models aliases",
        "models providers",
        "models connect",
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
        // Built-in template names and descriptions (translated dynamically at runtime)
        "General Assistant",
        "Versatile AI assistant for everyday tasks",
        "Code Helper",
        "Programming assistant with code review and debugging",
        "Researcher",
        "Deep research and analysis with web search",
        "Writer",
        "Creative and technical writing assistant",
        "Data Analyst",
        "Data analysis, visualization, and SQL queries",
        "DevOps Engineer",
        "Infrastructure, CI/CD, and deployment assistance",
        "Customer Support",
        "Professional customer service agent",
        "Tutor",
        "Patient educational assistant for learning any subject",
        "API Designer",
        "REST/GraphQL API design and documentation",
        "Meeting Notes",
        "Meeting transcription, summary, and action items",
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
        "draw failed",
        "failed to spawn librefang-tui-stream thread",
        "daemon_client() times out at 120 s; a longer wait can never return 202",
        "spawn_run_workflow builds a 60 s client; a longer wait can never return 202",
        // Redaction markers `GET /api/config` emits in place of a value.
        // Matched, never displayed — the editor renders an i18n string instead.
        "not set",
        // Technical format strings
        "%Y-%m-%d %H:%M",
        "{model:<20} {input}/{output}  ${cost:.4}",
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
        // `require_daemon(...)` labels for the `librefang group` commands
        // (#7745). These name a CLI invocation in an operator-facing error
        // (`start the daemon first, then re-run: librefang group list`), so the
        // literal is the command, not prose — translating it would print a
        // command that does not exist.
        "group list",
        "group show",
        "group create",
        "group delete",
        "group add-member",
        "group remove-member",
        "group of",
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
        "L I B R E F A N G",
        "Brave Search",
        "librefang init",
        "xAI (Grok)",
        "Qwen (Alibaba)",
        "Hugging Face",
        "GitHub Copilot",
        "Claude Code",
        "LM Studio",
        "api_key_env = \"{env_var}\"",
        "init wizard: failed to persist verified API key",
        "init wizard: retry of save_env_key failed",
        "`{other}` is listed in CLI_DISPATCH but has no match arm",
    ];
    if exclusions.contains(&trimmed) {
        return false;
    }

    if trimmed.contains("[capabilities]")
        || trimmed.starts_with("{} ")
        || trimmed.contains("Missing API key")
        || trimmed.starts_with("{} {} — ")
        || trimmed.starts_with("{} — {} ")
    {
        return false;
    }

    if trimmed.contains("[Unit]") || trimmed.contains("[Service]") || trimmed.contains("[Install]")
    {
        return false;
    }
    if trimmed.contains("{name:<28}") {
        return false;
    }

    // If the literal does not contain a space, it's highly likely to be a technical key, path, or identifier.
    if trimmed.contains("SHA256:")
        && (trimmed.contains("Install with:") || trimmed.contains("Size:"))
    {
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

    let next_char = |chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
                     line_number: &mut usize| {
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
            } else if remaining.starts_with("'\\n'")
                || remaining.starts_with("'\\r'")
                || remaining.starts_with("'\\t'")
                || remaining.starts_with("'\\0'")
            {
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
                let is_byte_string =
                    literal_start_idx > 0 && content.as_bytes()[literal_start_idx - 1] == b'b';
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
                if !is_byte_string
                    && !is_localized
                    && is_potential_untranslated_literal(&current_literal)
                {
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
    let end = content[index..]
        .find('\n')
        .map(|i| index + i)
        .unwrap_or(content.len());
    content[start..end].trim().to_string()
}

fn collect_rust_string_literals(content: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut chars = content.char_indices().peekable();
    let mut current = String::new();
    let mut in_quote = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while let Some((idx, c)) = chars.next() {
        if c == '\n' {
            in_line_comment = false;
        }

        if in_line_comment {
            continue;
        }

        if in_block_comment {
            if c == '*' && chars.peek().map(|&(_, next_c)| next_c) == Some('/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }

        if !in_quote {
            if c == '\'' {
                let mut temp_chars = chars.clone();
                let mut consume_count = 0;
                let mut parsed_char_literal = false;

                if let Some((_, first)) = temp_chars.next() {
                    consume_count += 1;
                    if first == '\\' && temp_chars.next().is_some() {
                        consume_count += 1;
                    }
                    if let Some((_, '\'')) = temp_chars.next() {
                        consume_count += 1;
                        parsed_char_literal = true;
                    }
                }

                if parsed_char_literal {
                    for _ in 0..consume_count {
                        chars.next();
                    }
                    continue;
                }
            }

            if c == '/' && chars.peek().map(|&(_, next_c)| next_c) == Some('/') {
                chars.next();
                in_line_comment = true;
                continue;
            }
            if c == '/' && chars.peek().map(|&(_, next_c)| next_c) == Some('*') {
                chars.next();
                in_block_comment = true;
                continue;
            }

            // Skip Rust raw string literals. i18n keys should be plain literals
            // passed to `i18n::t`/`t_args` or stored in key arrays.
            let prev_char_is_ident = idx > 0 && {
                let prev = content.as_bytes()[idx - 1] as char;
                prev.is_alphanumeric() || prev == '_'
            };
            if c == 'r' && !prev_char_is_ident {
                let remaining = &content[idx..];
                if remaining.starts_with("r\"") {
                    chars.next();
                    for (_, rc) in chars.by_ref() {
                        if rc == '"' {
                            break;
                        }
                    }
                    continue;
                }
                if remaining.starts_with("r#") {
                    let mut hashes = 0;
                    let temp_chars = chars.clone();
                    for (_, hc) in temp_chars {
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
        }

        if c == '"' {
            if in_quote {
                literals.push(current.clone());
                current.clear();
                in_quote = false;
            } else {
                in_quote = true;
            }
            continue;
        }

        if in_quote {
            if c == '\\' {
                if let Some((_, next_c)) = chars.next() {
                    current.push(next_c);
                }
            } else {
                current.push(c);
            }
        }
    }

    literals
}

fn collect_locale_keys(locale_file: &Path) -> Vec<String> {
    let content = fs::read_to_string(locale_file).unwrap();
    content
        .lines()
        .filter_map(|line| {
            if line.starts_with(char::is_whitespace) || line.trim_start().starts_with('#') {
                return None;
            }

            let (key, _) = line.split_once('=')?;
            let key = key.trim();
            let mut chars = key.chars();
            if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            if chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                Some(key.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn locale_key_prefixes(keys: &[String]) -> std::collections::BTreeSet<String> {
    keys.iter()
        .filter_map(|key| key.split_once('-').map(|(prefix, _)| prefix.to_string()))
        .collect()
}

fn is_likely_i18n_key_literal(
    literal: &str,
    known_prefixes: &std::collections::BTreeSet<String>,
) -> bool {
    // Not i18n keys: literals whose leading segment collides with a real key prefix.
    // `agent-types` is the operator agent-type directory name (`~/.librefang/agent-types`), not a message id.
    const TECHNICAL_FALSE_POSITIVES: &[&str] = &["daemon-reload", "agent-types"];

    let Some((prefix, _)) = literal.split_once('-') else {
        return false;
    };
    if literal.ends_with('-')
        || TECHNICAL_FALSE_POSITIVES.contains(&literal)
        || literal
            .split('-')
            .skip(1)
            .all(|part| part.chars().all(|c| c.is_ascii_digit()))
        || literal.starts_with("agent-uuid-")
    {
        return false;
    }
    known_prefixes.contains(prefix)
        && literal
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn is_dynamic_i18n_key(key: &str) -> bool {
    (key.starts_with("tui-templates-name-") || key.starts_with("tui-templates-desc-"))
        || (key.starts_with("tui-triggers-type-")
            && (key.ends_with("-name") || key.ends_with("-desc")))
        || key.starts_with("tui-skills-sort-")
}

fn extract_const_array_string_literals(content: &str, const_name: &str) -> Vec<String> {
    let Some(start) = content.find(&format!("const {const_name}")) else {
        return Vec::new();
    };
    let Some(array_start) = content[start..].find("&[") else {
        return Vec::new();
    };
    let block_start = start + array_start;
    let Some(array_end) = content[block_start..].find("];") else {
        return Vec::new();
    };
    collect_rust_string_literals(&content[block_start..block_start + array_end])
}

fn template_slug(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

fn collect_dynamic_required_locale_keys(manifest_dir: &Path) -> std::collections::BTreeSet<String> {
    let mut keys = std::collections::BTreeSet::new();

    let templates_rs =
        fs::read_to_string(manifest_dir.join("src/tui/screens/templates.rs")).unwrap();
    for chunk in extract_const_array_string_literals(&templates_rs, "BUILTIN_TEMPLATES").chunks(5) {
        let Some(name) = chunk.first() else {
            continue;
        };
        let slug = template_slug(name);
        keys.insert(format!("tui-templates-name-{slug}"));
        keys.insert(format!("tui-templates-desc-{slug}"));
    }

    let triggers_rs = fs::read_to_string(manifest_dir.join("src/tui/screens/triggers.rs")).unwrap();
    for name in extract_const_array_string_literals(&triggers_rs, "PATTERN_TYPES") {
        let slug = name.to_lowercase();
        keys.insert(format!("tui-triggers-type-{slug}-name"));
        keys.insert(format!("tui-triggers-type-{slug}-desc"));
    }

    let skills_rs = fs::read_to_string(manifest_dir.join("src/tui/screens/skills.rs")).unwrap();
    for literal in collect_rust_string_literals(&skills_rs) {
        if matches!(literal.as_str(), "trending" | "popular" | "recent") {
            keys.insert(format!("tui-skills-sort-{literal}"));
        }
    }

    keys
}

fn collect_required_i18n_keys(
    manifest_dir: &Path,
    known_prefixes: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<String> {
    let src_dir = manifest_dir.join("src");
    let mut required_keys = collect_dynamic_required_locale_keys(manifest_dir);

    for entry in WalkDir::new(&src_dir) {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read_to_string(path).unwrap();
            for literal in collect_rust_string_literals(&content) {
                if is_likely_i18n_key_literal(&literal, known_prefixes) {
                    required_keys.insert(literal);
                }
            }
        }
    }

    required_keys
}

/// Every locale directory under `locales/` that ships a `main.ftl`, sorted.
///
/// Read from disk so a locale added later is covered without anyone editing a list — the hole in #8151 was `ko` being absent from a hand-written one, and the same hole reopens for the next locale added if the list stays manual.
///
/// A directory without `main.ftl` is skipped rather than failing: the loader resolves that file specifically, so a directory that does not have one is not a locale the binary can serve.
fn shipped_locales(manifest_dir: &Path) -> Vec<String> {
    let locales_dir = manifest_dir.join("locales");
    let mut locales: Vec<String> = std::fs::read_dir(&locales_dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", locales_dir.display()))
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            if !entry.path().join("main.ftl").is_file() {
                return None;
            }
            entry.file_name().into_string().ok()
        })
        .collect();
    locales.sort();
    assert!(
        locales.iter().any(|l| l == "en"),
        "locales/en/main.ftl is the reference every other locale is checked against — \
         finding no `en` means this scan is looking in the wrong place, not that English was dropped"
    );
    locales
}

fn assert_locale_covers_required_i18n_keys(
    manifest_dir: &Path,
    locale: &str,
    required_keys: &std::collections::BTreeSet<String>,
) {
    let locale_keys: std::collections::BTreeSet<String> =
        collect_locale_keys(&manifest_dir.join(format!("locales/{locale}/main.ftl")))
            .into_iter()
            .collect();

    let missing_keys: Vec<String> = required_keys
        .iter()
        .filter(|key| !locale_keys.contains(key.as_str()))
        .cloned()
        .collect();

    if !missing_keys.is_empty() {
        panic!(
            "locales/{locale}/main.ftl is missing {} key(s) referenced by CLI Rust code:\n{}",
            missing_keys.len(),
            missing_keys.join("\n")
        );
    }
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
            let _rel_path = path
                .strip_prefix(&src_dir)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");

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

#[test]
fn test_no_dead_locale_keys() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let manifest_dir = Path::new(&manifest_dir);
    let src_dir = manifest_dir.join("src");
    let locales_dir = manifest_dir.join("locales");

    let mut used_literals = std::collections::BTreeSet::new();
    for entry in WalkDir::new(&src_dir) {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read_to_string(path).unwrap();
            used_literals.extend(collect_rust_string_literals(&content));
        }
    }

    let mut dead_keys = Vec::new();
    for entry in fs::read_dir(&locales_dir).unwrap() {
        let entry = entry.unwrap();
        let locale_dir = entry.path();
        if !locale_dir.is_dir() {
            continue;
        }
        let locale_file = locale_dir.join("main.ftl");
        if !locale_file.is_file() {
            continue;
        }

        let locale = locale_dir.file_name().unwrap().to_string_lossy();
        for key in collect_locale_keys(&locale_file) {
            if !used_literals.contains(&key) && !is_dynamic_i18n_key(&key) {
                dead_keys.push(format!("{locale}/{key}"));
            }
        }
    }

    if !dead_keys.is_empty() {
        panic!(
            "Found locale keys that are not referenced by CLI Rust code:\n{}",
            dead_keys.join("\n")
        );
    }
}

#[test]
fn test_locales_cover_used_i18n_keys() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let manifest_dir = Path::new(&manifest_dir);
    let english_keys: std::collections::BTreeSet<String> =
        collect_locale_keys(&manifest_dir.join("locales/en/main.ftl"))
            .into_iter()
            .collect();
    let known_prefixes = locale_key_prefixes(&english_keys.iter().cloned().collect::<Vec<_>>());
    let required_keys = collect_required_i18n_keys(manifest_dir, &known_prefixes);

    // Every locale that ships, discovered from disk rather than listed here.
    //
    // The list used to be hand-written, and `ko` was missing from it while `locales/ko/main.ftl` was a complete 2510-line locale — so eight Auxiliary-tab keys reached a branch with no Korean translation and nothing failed (#8151).
    // Adding the missing line fixes that one locale; reading the directory fixes the shape, because the failure mode was a locale nobody remembered to list, and a hand-written list re-arms it for the next one added.
    for locale in shipped_locales(manifest_dir) {
        assert_locale_covers_required_i18n_keys(manifest_dir, &locale, &required_keys);
    }
}

/// One reason, and the key/locale pairs it covers, for values that are supposed to match English.
struct IdenticalValueExemption {
    /// Why the English text is the right text here. Written for whoever reads the failure, not for whoever wrote the entry.
    reason: &'static str,
    /// Locales the exemption applies to; empty means all of them.
    locales: &'static [&'static str],
    keys: &'static [&'static str],
}

impl IdenticalValueExemption {
    fn covers(&self, locale: &str, key: &str) -> bool {
        (self.locales.is_empty() || self.locales.contains(&locale)) && self.keys.contains(&key)
    }
}

/// Locale values that are legitimately byte-identical to English, and why.
///
/// Every entry is a deliberate statement that the English text is the correct text for that locale, not a record that translating it is still to do — a key that simply has not been translated yet belongs in the locale file with a translation, not here.
/// `locales` empty means the exemption holds for every locale; naming locales narrows it, so a locale that does translate the value keeps being checked.
///
/// Adding an entry costs a written reason and shows up in review as data rather than as a change to the check, which is the point: #8178 was filed because the exception list is the policy, and a policy that lives inside a regex cannot be reviewed.
const IDENTICAL_VALUE_EXEMPTIONS: &[IdenticalValueExemption] = &[
    IdenticalValueExemption {
        reason: "Product, brand and company names. Spelled the same in every locale by definition, which is what the `brand-` prefix exists to declare.",
        locales: &[],
        keys: &[
            "brand-alibaba-coding-plan",
            "brand-azure-openai",
            "brand-byteplus",
            "brand-claude-code",
            "brand-deepinfra",
            "brand-deepseek",
            "brand-discord",
            "brand-github-copilot",
            "brand-huggingface",
            "brand-kimi-coding",
            "brand-nvidia-nim",
            "brand-openai",
            "brand-openai-codex",
            "brand-openclaw",
            "brand-openclaw-openfang",
            "brand-openfang",
            "brand-openrouter",
            "brand-slack-app",
            "brand-slack-bot",
            "brand-telegram",
            "brand-vertex-ai",
            "brand-zai",
        ],
    },
    IdenticalValueExemption {
        reason: "Identifiers and acronyms used as literal column headers or field labels. `ID`, `URL`, `PID`, `API`, `MCP` and `Top-p` are not translated in any of the shipped locales, and a column header that differs from the API field it shows is harder to read, not easier.",
        locales: &[],
        keys: &[
            "label-api",
            "label-header-id",
            "label-header-url",
            "label-id",
            "label-pid",
            "tui-agents-detail-id",
            "tui-agents-detail-mcp",
            "tui-agents-header-id",
            "tui-agents-param-top-p",
            "tui-event-daemon-http-status",
            "tui-extensions-header-id",
            "tui-memory-header-id",
            "tui-workflows-header-id",
        ],
    },
    IdenticalValueExemption {
        reason: "Commands and config snippets the user copies verbatim into a shell or a config file. Translating any word here produces something that does not run.",
        locales: &[],
        keys: &[
            "auth-api-key-config-entry",
            "auth-hash-config-entry",
            "auth-pool-add-example",
            "channel-install-sdk-cmd",
            "desktop-install-skipped-brew",
            "mcp-vault-set-hint",
            "tui-workflows-placeholder-steps",
        ],
    },
    IdenticalValueExemption {
        reason: "Layouts made of placeables, punctuation and literal marker tokens, with no prose to translate. The `key=value` field names match the config keys or the API fields they report.",
        locales: &[],
        keys: &[
            "auth-pool-header",
            "auth-pool-key-item",
            "channel-last-error-entry",
            "chat-runner-owner-notice",
            "model-picker-item",
            "tui-event-promote-http-error",
            "tui-guide-warn-env",
            "tui-mod-error-symbol",
            "tui-triggers-placeholder-agent-id",
            "tui-triggers-placeholder-max-fires",
        ],
    },
    IdenticalValueExemption {
        reason: "Names of LibreFang features and of the ClawHub marketplace, carried as-is the way the brand keys are.",
        locales: &[],
        keys: &[
            "tui-dashboard-dreams-title",
            "tui-skills-tab-clawhub",
            "ui-brand-title",
        ],
    },
    IdenticalValueExemption {
        reason: "Trigger-type names as they appear on the wire and in `agent.toml`. An operator matching a screen against a config file needs the same spelling in both.",
        locales: &[],
        keys: &[
            "tui-triggers-type-agentspawned-name",
            "tui-triggers-type-channelmessage-name",
            "tui-triggers-type-contentmatch-name",
            "tui-triggers-type-schedule-name",
            "tui-triggers-type-webhook-name",
        ],
    },
    IdenticalValueExemption {
        reason: "Trigger-type names as they appear on the wire and in `agent.toml`. An operator matching a screen against a config file needs the same spelling in both.",
        locales: &["uk", "zh-CN"],
        keys: &[
            "tui-triggers-type-lifecycle-name",
        ],
    },
    IdenticalValueExemption {
        reason: "Only the punctuation differs from English, and Korean and Ukrainian use the same ASCII colon, parentheses and brackets that English does. zh-CN differs solely because it uses the fullwidth forms.",
        locales: &["ko", "uk"],
        keys: &[
            "agent-spawn-id-label",
            "automation-workflow-created-id",
            "channel-prompt-default",
            "channel-prompt-optional",
            "channel-prompt-required",
            "skill-bundle-sha",
            "tui-channels-group-count",
            "tui-event-daemon-failure-detail",
        ],
    },
    IdenticalValueExemption {
        reason: "Binary and decimal unit abbreviations. Korean and Chinese write these in Latin script; only Ukrainian transliterates them into Cyrillic.",
        locales: &["ko", "zh-CN"],
        keys: &[
            "format-bytes-b",
            "format-bytes-gib",
            "format-bytes-kib",
            "format-bytes-mib",
            "format-size-mb",
            "status-mb",
        ],
    },
    IdenticalValueExemption {
        reason: "A language runtime's own name followed by its version. Korean and Ukrainian keep the upstream spelling.",
        locales: &["ko", "uk"],
        keys: &[
            "doctor-check-node-version",
            "doctor-check-python-version",
            "doctor-check-rust-version",
        ],
    },
    IdenticalValueExemption {
        reason: "Example values the user copies into a field that only accepts them as written: `template_id` is the API field name, and agent and workflow names are validated as ASCII slugs.",
        locales: &["ko", "zh-CN"],
        keys: &[
            "mcp-header-template-id",
            "tui-agents-placeholder-name",
            "tui-workflows-placeholder-name",
        ],
    },
    IdenticalValueExemption {
        reason: "`Hand` is a LibreFang product concept. Ukrainian and Chinese keep the English term; Korean transliterates it. This is a terminology decision that has not been made explicitly — it is recorded here rather than settled, so that flipping it is one edit in one place.",
        locales: &["uk", "zh-CN"],
        keys: &[
            "label-hands",
            "label-header-hand",
            "tui-hands-header-hand",
            "tui-hands-title",
            "tui-tab-hands",
        ],
    },
    IdenticalValueExemption {
        reason: "`Hand` is a LibreFang product concept. Ukrainian and Chinese keep the English term; Korean transliterates it. This is a terminology decision that has not been made explicitly — it is recorded here rather than settled, so that flipping it is one edit in one place.",
        locales: &["zh-CN"],
        keys: &[
            "label-hand",
        ],
    },
];

/// Every `key = value` pair in a locale file, with multiline continuations folded in.
///
/// Separate from [`collect_locale_keys`] because that one deliberately discards values; the untranslated-value check is entirely about them.
/// Continuation lines are joined with a newline and trimmed the way Fluent renders them, so a value the loader treats as one string compares as one string here.
fn collect_locale_entries(locale_file: &Path) -> std::collections::BTreeMap<String, String> {
    let content = fs::read_to_string(locale_file)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", locale_file.display()));
    let mut entries = std::collections::BTreeMap::new();
    let mut current: Option<String> = None;
    // A blank line does not end a Fluent message on its own — `doctor-section-*` are written as an empty first line, a blank line, then the indented text — so blank lines are held back and only folded in once an indented line proves the message continued.
    let mut pending_blanks = 0usize;
    for line in content.lines() {
        if line.trim().is_empty() {
            if current.is_some() {
                pending_blanks += 1;
            }
            continue;
        }
        if line.trim_start().starts_with('#') {
            current = None;
            pending_blanks = 0;
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            // A continuation of the previous message, or an attribute / selector line inside it. Either way it belongs to the value already being accumulated.
            if let Some(key) = &current {
                let value: &mut String = entries.get_mut(key).expect("current key was inserted");
                for _ in 0..pending_blanks {
                    value.push('\n');
                }
                value.push('\n');
                value.push_str(line.trim());
            }
            pending_blanks = 0;
            continue;
        }
        current = None;
        pending_blanks = 0;
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let mut chars = key.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            continue;
        }
        // Only the leading space of `key = value` is Fluent syntax; anything past it is part of the value and several keys use it for column alignment.
        let value = value.strip_prefix(' ').unwrap_or(value);
        entries.insert(key.to_string(), value.to_string());
        current = Some(key.to_string());
    }
    entries
}

/// A locale value byte-identical to English is not a translation (#8178).
///
/// The missing-key check next door cannot see this one: the key is present, so coverage is satisfied, and the string renders as fluent English inside an otherwise translated screen.
/// That reads as a deliberate choice rather than as a bug, so nobody files it — eight `tui-settings-*auxiliary*` keys reached `uk` and `zh-CN` with their English values and stayed green the whole time.
///
/// Exemptions are data in [`IDENTICAL_VALUE_EXEMPTIONS`], not a pattern in this function.
/// The distinction matters more than it looks: a heuristic that decides "this looks like a format string" keeps passing as the strings around it drift, and nothing in a diff shows that it stopped catching anything, whereas an entry added to the table is a line a reviewer reads.
///
/// The check is not symmetric with the locale — `en` is the reference and is skipped rather than compared against itself.
#[test]
fn test_locale_values_are_not_copies_of_english() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let manifest_dir = Path::new(&manifest_dir);
    let english = collect_locale_entries(&manifest_dir.join("locales/en/main.ftl"));

    let mut untranslated: Vec<String> = Vec::new();
    for locale in shipped_locales(manifest_dir) {
        if locale == "en" {
            continue;
        }
        let entries =
            collect_locale_entries(&manifest_dir.join(format!("locales/{locale}/main.ftl")));
        for (key, value) in &entries {
            let Some(english_value) = english.get(key) else {
                // A key absent from `en` is a different defect and belongs to the dead-key check, which reports it with the context to act on.
                continue;
            };
            if value != english_value {
                continue;
            }
            if IDENTICAL_VALUE_EXEMPTIONS
                .iter()
                .any(|exemption| exemption.covers(&locale, key))
            {
                continue;
            }
            untranslated.push(format!("  {locale}/{key} = {english_value:?}"));
        }
    }

    assert!(
        untranslated.is_empty(),
        "These locale values are byte-identical to the English text, so the screen shows English \
         to a user who selected another language (#8178):\n{}\n\nTranslate them. If the English \
         text really is correct for that locale — a brand name, a shell command, a column header \
         that matches an API field — add the key to IDENTICAL_VALUE_EXEMPTIONS in this file with \
         a reason saying which.",
        untranslated.join("\n")
    );
}

/// Guards the exemption table against the two ways it rots into a rubber stamp.
///
/// An entry for a key that no longer exists, or for one whose value is no longer identical, is an exemption nobody can see is unused — and the next key to reuse that name inherits it silently.
/// A reason that says nothing defeats the reason the table is data rather than a regex.
#[test]
fn identical_value_exemptions_are_all_still_load_bearing() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let manifest_dir = Path::new(&manifest_dir);
    let english = collect_locale_entries(&manifest_dir.join("locales/en/main.ftl"));
    let locales: Vec<String> = shipped_locales(manifest_dir)
        .into_iter()
        .filter(|l| l != "en")
        .collect();
    let entries: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> =
        locales
            .iter()
            .map(|locale| {
                (
                    locale.clone(),
                    collect_locale_entries(
                        &manifest_dir.join(format!("locales/{locale}/main.ftl")),
                    ),
                )
            })
            .collect();

    let mut stale: Vec<String> = Vec::new();
    let mut seen: std::collections::BTreeSet<(&str, &str)> = std::collections::BTreeSet::new();
    for exemption in IDENTICAL_VALUE_EXEMPTIONS {
        assert!(
            exemption.reason.split_whitespace().count() >= 8,
            "exemption reason {:?} is too short to tell a reader why the English text is correct here",
            exemption.reason
        );
        for named in exemption.locales {
            assert!(
                locales.iter().any(|l| l == named),
                "exemption names locale {named:?}, which ships no locales/{named}/main.ftl"
            );
        }
        for key in exemption.keys {
            assert!(
                seen.insert((
                    if exemption.locales.is_empty() {
                        ""
                    } else {
                        exemption.locales[0]
                    },
                    key
                )),
                "{key} is exempted twice for the same locales; two reasons for one key means one of them is wrong"
            );
            let Some(english_value) = english.get(*key) else {
                stale.push(format!("  {key} — no longer in locales/en/main.ftl"));
                continue;
            };
            let applies: Vec<&String> = locales
                .iter()
                .filter(|l| exemption.locales.is_empty() || exemption.locales.contains(&l.as_str()))
                .collect();
            for locale in applies {
                match entries[locale].get(*key) {
                    None => stale.push(format!("  {locale}/{key} — key absent from that locale")),
                    Some(value) if value != english_value => stale.push(format!(
                        "  {locale}/{key} — now translated ({value:?}), so the exemption is dead"
                    )),
                    Some(_) => {}
                }
            }
        }
    }

    assert!(
        stale.is_empty(),
        "IDENTICAL_VALUE_EXEMPTIONS has entries that no longer describe anything:\n{}\n\nDrop them. \
         An exemption that matches nothing is invisible until a future key reuses the name and \
         inherits it.",
        stale.join("\n")
    );
}
