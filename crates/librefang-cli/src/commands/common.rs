//! Shared CLI helpers used across command groups, split out of `main.rs`:
//! daemon discovery + HTTP client, JSON-over-HTTP, home/config resolution,
//! formatting, and small filesystem/clipboard utilities.

use crate::commands::prelude::*;

/// Resolved daemon-connection context derived from config.toml — home dir,
/// API key, and optional custom log dir. Shared by the status/daemon/logs
/// command groups.
#[derive(Debug, Clone)]
pub(crate) struct DaemonConfigContext {
    pub(crate) home_dir: PathBuf,
    pub(crate) api_key: Option<String>,
    pub(crate) log_dir: Option<PathBuf>,
}

/// Get the LibreFang home directory, respecting LIBREFANG_HOME env var.
pub(crate) fn cli_librefang_home() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return std::path::PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

pub(crate) fn daemon_config_context(config: Option<&std::path::Path>) -> DaemonConfigContext {
    let config = load_config(config).unwrap_or_else(|e| {
        eprintln!("warning: {e}; using default config values for this command");
        librefang_types::config::KernelConfig::default()
    });
    let api_key = {
        let trimmed = config.api_key.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };
    DaemonConfigContext {
        home_dir: config.home_dir,
        api_key,
        log_dir: config.log_dir,
    }
}

/// Load just the `update_channel` field from config.toml without fully deserializing.
pub(crate) fn load_update_channel_from_config() -> Option<librefang_types::config::UpdateChannel> {
    let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: toml::Value = toml::from_str(&content).ok()?;
    config
        .get("update_channel")?
        .as_str()?
        .parse::<librefang_types::config::UpdateChannel>()
        .ok()
}

/// Load the `[skills]` config block and derive the `EnvPassthroughPolicy`
/// the daemon would apply. Falls back to `SkillsConfig::default()` so the
/// conservative built-in deny patterns still apply when no config exists —
/// otherwise `librefang skill test` would silently allow vars that
/// production strips. Errors during read/parse degrade to default; this is
/// a dev-time gate, not a security boundary, but its job is to mirror
/// what prod will do. Returns `None` only when the operator has explicitly
/// cleared both `env_passthrough_denied_patterns` and
/// `env_passthrough_per_skill` — matching the kernel-side semantics.
pub(crate) fn load_skill_env_policy_from_config(
) -> Option<librefang_types::config::EnvPassthroughPolicy> {
    let cfg = (|| -> Option<librefang_types::config::SkillsConfig> {
        let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
        let content = std::fs::read_to_string(&config_path).ok()?;
        let value: toml::Value = toml::from_str(&content).ok()?;
        let skills = value.get("skills")?.clone();
        skills
            .try_into::<librefang_types::config::SkillsConfig>()
            .ok()
    })()
    .unwrap_or_default();
    librefang_types::config::EnvPassthroughPolicy::from_skills_config(&cfg)
}

/// Write `msg` followed by a newline to stdout, exiting with code 0 on
/// `BrokenPipe`. Use this instead of `println!` for machine-readable (JSON)
/// output that is commonly piped into other tools — e.g.
/// `librefang doctor --json | head -1`. Without this wrapper, SIGPIPE/EPIPE
/// surfaces as a panic on the next write attempt.
pub(crate) fn write_stdout_safe(msg: &str) {
    let out = std::io::stdout();
    let mut lock = out.lock();
    if let Err(e) = writeln!(lock, "{}", msg) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("error: failed writing to stdout: {e}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Daemon detection helpers
// ---------------------------------------------------------------------------

/// Try to find a running daemon. Returns its base URL if found.
/// SECURITY: Restrict file permissions to owner-only (0600) on Unix.
#[cfg(unix)]
pub(crate) fn restrict_file_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub(crate) fn restrict_file_permissions(_path: &std::path::Path) {}

/// SECURITY: Restrict directory permissions to owner-only (0700) on Unix.
#[cfg(unix)]
pub(crate) fn restrict_dir_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
pub(crate) fn restrict_dir_permissions(_path: &std::path::Path) {}

/// Normalize a daemon listen address for client-side probing.
///
/// `0.0.0.0` (the default bind-all address) is replaced with `127.0.0.1`,
/// which avoids DNS/connectivity hangs on macOS when probing locally.
pub(crate) fn normalize_daemon_addr(listen_addr: &str) -> String {
    listen_addr.replace("0.0.0.0", "127.0.0.1")
}

/// Core daemon-detection logic, parameterized over the health-probe.
///
/// Returns `Some(base_url)` iff `daemon.json` is readable AND `probe`
/// reports the daemon's `/api/health` endpoint is up. Extracted so unit
/// tests can inject a fake probe instead of binding real sockets.
pub(crate) fn find_daemon_with_probe<F>(home_dir: &std::path::Path, probe: F) -> Option<String>
where
    F: FnOnce(&str) -> bool,
{
    let info = read_daemon_info(home_dir)?;
    let addr = normalize_daemon_addr(&info.listen_addr);
    let health_url = format!("http://{addr}/api/health");
    if probe(&health_url) {
        Some(format!("http://{addr}"))
    } else {
        None
    }
}

pub(crate) fn find_daemon_in_home(home_dir: &std::path::Path) -> Option<String> {
    find_daemon_with_probe(home_dir, |url| {
        let client = match crate::http_client::client_builder()
            .connect_timeout(std::time::Duration::from_secs(1))
            .timeout(std::time::Duration::from_secs(2))
            .build()
        {
            Ok(c) => c,
            Err(_) => return false,
        };
        client
            .get(url)
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    })
}

pub(crate) fn find_daemon() -> Option<String> {
    find_daemon_in_home(&cli_librefang_home())
}

/// Build an HTTP client for daemon calls.
///
/// When api_key is configured in config.toml, the client automatically
/// includes a `Authorization: Bearer <key>` header on every request.
/// When api_key is empty or missing, no auth header is sent.
pub(crate) fn daemon_client() -> reqwest::blocking::Client {
    daemon_client_with_api_key(read_api_key().as_deref())
}

pub(crate) fn daemon_client_with_api_key(api_key: Option<&str>) -> reqwest::blocking::Client {
    let mut builder =
        crate::http_client::client_builder().timeout(std::time::Duration::from_secs(120));

    if let Some(key) = api_key {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
        builder = builder.default_headers(headers);
    }

    builder.build().expect("Failed to build HTTP client")
}

/// True when `body` is a JSON object carrying a non-empty `error` string —
/// the structured-error shape some commands intentionally surface with a
/// command-specific message (e.g. validation failures returned as 4xx).
///
/// When this holds for a 4xx response, `daemon_json` returns the body
/// untouched so the caller's existing `body.get("error")` handler runs.
/// When it does not (non-JSON body, JSON without `error`, or an empty/
/// non-string `error`), a 4xx is treated as a fatal daemon error instead
/// of slipping through as silent success (#6019).
pub(crate) fn body_carries_usable_error(body: &serde_json::Value) -> bool {
    body.get("error")
        .and_then(|e| e.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

/// Decide whether a daemon response `status` + parsed `body` should be
/// surfaced as a fatal error by `daemon_json`. Extracted as a pure function
/// so the 4xx/5xx classification is unit-testable without binding sockets.
///
/// - 5xx → always a daemon error (historical behaviour).
/// - 4xx → a daemon error *unless* the body carries a usable `error` field,
///   in which case the caller surfaces its own command-specific message.
/// - 2xx/3xx → never surfaced here.
pub(crate) fn should_surface_status_error(
    status: reqwest::StatusCode,
    body: &serde_json::Value,
) -> bool {
    status.is_server_error() || (status.is_client_error() && !body_carries_usable_error(body))
}

/// Helper: send a request to the daemon and parse the JSON body.
/// Exits with error on connection failure.
pub(crate) fn daemon_json(
    resp: Result<reqwest::blocking::Response, reqwest::Error>,
) -> serde_json::Value {
    match resp {
        Ok(r) => {
            let status = r.status();
            let body = r.json::<serde_json::Value>().unwrap_or_default();
            if should_surface_status_error(status, &body) {
                ui::error_with_fix(
                    &i18n::t_args("error-daemon-returned", &[("status", &status.to_string())]),
                    &i18n::t("error-daemon-returned-fix"),
                );
            }
            body
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("timed out") || msg.contains("Timeout") {
                ui::error_with_fix(
                    &i18n::t("error-request-timeout"),
                    &i18n::t("error-request-timeout-fix"),
                );
            } else if msg.contains("Connection refused") || msg.contains("connect") {
                ui::error_with_fix(
                    &i18n::t("error-connect-refused"),
                    &i18n::t("error-connect-refused-fix"),
                );
            } else {
                ui::error_with_fix(
                    &i18n::t_args("error-daemon-comm", &[("error", &msg)]),
                    &i18n::t("error-daemon-comm-fix"),
                );
            }
            std::process::exit(1);
        }
    }
}

/// Generate a local timestamp string in YYYYMMDD-HHMMSS format.
pub(crate) fn format_local_timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Lightweight date string (YYYY-MM-DD) without external dependencies.
pub(crate) fn chrono_lite_date() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    date_from_unix_secs(secs)
}

pub(crate) fn date_from_unix_secs(secs: u64) -> String {
    let days = secs / 86400;
    let mut year = 1970;
    let mut remaining_days = days as i64;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    // Length of month `m` (1-based), accounting for a leap-year February.
    let month_len = |m: u64| -> i64 {
        let base = month_days
            .get((m.saturating_sub(1)) as usize)
            .copied()
            .unwrap_or(28) as i64;
        if m == 2 && is_leap_year(year) {
            29
        } else {
            base
        }
    };
    let mut month: u64 = 1;
    let mut day: i64 = remaining_days + 1;
    let mut md: i64 = month_len(month);
    while day > md {
        day -= md;
        month += 1;
        md = month_len(month);
    }
    format!("{:04}-{:02}-{:02}", year, month, day)
}

pub(crate) fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

/// Parse YYYY-MM-DD to Unix seconds at 00:00:00 UTC.
pub(crate) fn parse_daily_date_timestamp(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: u64 = parts[0].parse().ok()?;
    let month: u64 = parts[1].parse().ok()?;
    let day: u64 = parts[2].parse().ok()?;
    Some(days_since_epoch(year, month, day) * 86400)
}

pub(crate) fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
    let mut days = 0;
    for y in 1970..year {
        days += if is_leap_year(y as i64) { 366 } else { 365 };
    }
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += month_days.get((m - 1) as usize).copied().unwrap_or(28) as u64;
    }
    if is_leap_year(year as i64) && month > 2 {
        days += 1;
    }
    days + day - 1
}

/// Read the daemon api_key from the effective CLI config (if any).
///
/// Returns `None` when the key is missing, empty, or whitespace-only —
/// meaning the daemon is running in public (unauthenticated) mode.
pub(crate) fn read_api_key() -> Option<String> {
    daemon_config_context(None).api_key
}

/// Show context-aware error for kernel boot failures.
pub(crate) fn boot_kernel_error(e: &librefang_kernel::error::KernelError) {
    let msg = e.to_string();
    if msg.contains("parse") || msg.contains("toml") || msg.contains("config") {
        ui::error_with_fix(
            &i18n::t("error-boot-config"),
            &i18n::t("error-boot-config-fix"),
        );
    } else if msg.contains("database") || msg.contains("locked") || msg.contains("sqlite") {
        ui::error_with_fix(&i18n::t("error-boot-db"), &i18n::t("error-boot-db-fix"));
    } else if msg.contains("key") || msg.contains("API") || msg.contains("auth") {
        ui::error_with_fix(&i18n::t("error-boot-auth"), &i18n::t("error-boot-auth-fix"));
    } else {
        ui::error_with_fix(
            &i18n::t_args("error-boot-generic", &[("error", &msg)]),
            &i18n::t("error-boot-generic-fix"),
        );
    }
}

/// Minimal percent-encoder for a single URL path segment. Encodes
/// everything outside the `unreserved` set (RFC 3986 §2.3) plus `/` so
/// the segment can't escape into a parent path. Avoids pulling a new
/// dependency for the one-off use here.
pub(crate) fn percent_encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        let b = *byte;
        let unreserved =
            b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~';
        if unreserved {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Minimal `[y/N]` prompt for destructive operations. Reads a single
/// line from stdin; treats anything other than `y` / `Y` / `yes` /
/// `YES` as "no" (per the issue's `[y/N]` default).
pub(crate) fn prompt_yes_no(prompt: &str, default_yes: bool) -> bool {
    use std::io::Write as _;
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    eprint!("{prompt} {suffix} ");
    let _ = std::io::stderr().flush();
    let mut buf = String::new();
    if std::io::stdin().read_line(&mut buf).is_err() {
        return false;
    }
    let trimmed = buf.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return default_yes;
    }
    matches!(trimmed.as_str(), "y" | "yes")
}

pub(crate) fn format_latency(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1 {
        format!("{}µs", d.as_micros())
    } else {
        format!("{ms}ms")
    }
}

/// Recursively sum file sizes under `dir`. Returns `None` if `dir` does not
/// exist or cannot be read. Symlinks are followed because the default data
/// directory may legitimately symlink subdirs onto another disk.
pub(crate) fn dir_size_bytes(dir: &std::path::Path) -> Option<u64> {
    if !dir.exists() {
        return None;
    }
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Ok(md) = entry.metadata() {
            if md.is_file() {
                total = total.saturating_add(md.len());
            }
        }
    }
    Some(total)
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: &[(&str, u64)] = &[
        ("GiB", 1u64 << 30),
        ("MiB", 1u64 << 20),
        ("KiB", 1u64 << 10),
    ];
    for (unit, thresh) in UNITS {
        if bytes >= *thresh {
            return format!("{:.2} {}", bytes as f64 / *thresh as f64, unit);
        }
    }
    format!("{bytes} B")
}

pub(crate) fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!(
            "{}d {}h {}m",
            secs / 86400,
            (secs % 86400) / 3600,
            (secs % 3600) / 60
        )
    }
}

/// Copy text to the system clipboard. Returns true on success.
pub(crate) fn copy_to_clipboard(text: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Use PowerShell to set clipboard (handles special characters better than cmd)
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Set-Clipboard '{}'", text.replace('\'', "''")),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Write as IoWrite;
        std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Write as IoWrite;
        // Try xclip first, then xsel
        let result = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false);
        if result {
            return true;
        }
        std::process::Command::new("xsel")
            .args(["--clipboard", "--input"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        false
    }
}

/// Try to open a URL in the default browser. Returns true on success.
pub(crate) fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().is_ok()
    }
    #[cfg(target_os = "linux")]
    {
        // Try multiple openers in order. xdg-open is the standard, but it
        // (or the browser it launches) can fail with EPERM in sandboxed
        // environments (containers, Snap, Flatpak, user-namespace
        // restrictions). Fall through to alternatives if any opener fails.
        let openers = [
            "xdg-open",
            "sensible-browser",
            "x-www-browser",
            "firefox",
            "google-chrome",
            "chromium",
            "chromium-browser",
        ];
        for opener in &openers {
            let result = std::process::Command::new(opener)
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if result.is_ok() {
                return true;
            }
        }
        false
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = url;
        false
    }
}

/// Require a running daemon — exit with helpful message if not found.
pub(crate) fn require_daemon(command: &str) -> String {
    find_daemon().unwrap_or_else(|| {
        ui::error_with_fix(
            &i18n::t_args("error-require-daemon", &[("command", command)]),
            &i18n::t("error-require-daemon-fix"),
        );
        ui::hint(&i18n::t("hint-or-chat"));
        std::process::exit(1);
    })
}

pub(crate) fn boot_kernel(config: Option<PathBuf>) -> LibreFangKernel {
    match LibreFangKernel::boot(config.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            boot_kernel_error(&e);
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Skill evolve commands — thin CLI wrappers over librefang_skills::evolution
// ---------------------------------------------------------------------------

/// Read a file path, or stdin if path is "-".
pub(crate) fn read_file_or_stdin(path: &std::path::Path) -> std::io::Result<String> {
    if path == std::path::Path::new("-") {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path)
    }
}

// ---------------------------------------------------------------------------
// Provider / API key helpers
// ---------------------------------------------------------------------------

/// Map a provider name to its conventional environment variable name.
pub(crate) fn provider_to_env_var(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
        "groq" => "GROQ_API_KEY".to_string(),
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        "gemini" => "GEMINI_API_KEY".to_string(),
        "google" => "GOOGLE_API_KEY".to_string(),
        "deepseek" => "DEEPSEEK_API_KEY".to_string(),
        "openrouter" => "OPENROUTER_API_KEY".to_string(),
        "together" => "TOGETHER_API_KEY".to_string(),
        "mistral" => "MISTRAL_API_KEY".to_string(),
        "fireworks" => "FIREWORKS_API_KEY".to_string(),
        "perplexity" => "PERPLEXITY_API_KEY".to_string(),
        "cohere" => "COHERE_API_KEY".to_string(),
        "xai" => "XAI_API_KEY".to_string(),
        "brave" => "BRAVE_API_KEY".to_string(),
        "tavily" => "TAVILY_API_KEY".to_string(),
        other => format!("{}_API_KEY", other.to_uppercase()),
    }
}

/// Test an API key by hitting the provider's models/health endpoint.
///
/// Returns true if the key is accepted (status != 401/403).
/// Returns true on timeout/network errors (best-effort — don't block setup).
pub(crate) fn test_api_key(provider: &str, key: &str) -> bool {
    if key.is_empty() {
        return false;
    }

    let client = match crate::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return true, // can't build client — assume ok
    };

    let result = match provider.to_lowercase().as_str() {
        "groq" => client
            .get("https://api.groq.com/openai/v1/models")
            .bearer_auth(key)
            .send(),
        "anthropic" => client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .send(),
        "openai" => client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(key)
            .send(),
        "gemini" | "google" => client
            .get(format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={key}"
            ))
            .send(),
        "deepseek" => client
            .get("https://api.deepseek.com/models")
            .bearer_auth(key)
            .send(),
        "openrouter" => client
            .get("https://openrouter.ai/api/v1/models")
            .bearer_auth(key)
            .send(),
        "byteplus" => client
            .get("https://ark.ap-southeast.bytepluses.com/api/v3/models")
            .bearer_auth(key)
            .send(),
        "elevenlabs" => client
            .get("https://api.elevenlabs.io/v1/user")
            .header("xi-api-key", key)
            .send(),
        _ => return true, // unknown provider — skip test
    };

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            status != 401 && status != 403
        }
        Err(_) => true, // network error — don't block setup
    }
}

// ---------------------------------------------------------------------------
// Background daemon start
// ---------------------------------------------------------------------------

/// Spawn `librefang start` as a detached background process.
///
/// Polls for daemon health for up to 10 seconds. Returns the daemon URL on success.
pub(crate) fn start_daemon_background() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find executable: {e}"))?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        std::process::Command::new(&exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(&exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    // Poll for daemon readiness
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(url) = find_daemon() {
            return Ok(url);
        }
    }

    Err("Daemon did not become ready within 10 seconds".to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        })
        .join(".librefang")
}

pub(crate) fn prompt_input(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap_or(0);
    line.trim().to_string()
}

pub(crate) fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) {
    std::fs::create_dir_all(dst).unwrap();
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let path = entry.path();
            let dest_path = dst.join(entry.file_name());
            if path.is_dir() {
                copy_dir_recursive(&path, &dest_path);
            } else {
                let _ = std::fs::copy(&path, &dest_path);
            }
        }
    }
}

/// JSON → TOML converter. Duplicates the `json_to_toml_value` helper from
/// the API crate to avoid a cross-crate dependency.
pub(crate) fn json_to_toml_value_cli(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::Null => toml::Value::String(String::new()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            toml::Value::Array(arr.iter().map(json_to_toml_value_cli).collect())
        }
        serde_json::Value::Object(map) => {
            let mut t = toml::value::Table::new();
            for (k, v) in map {
                t.insert(k.clone(), json_to_toml_value_cli(v));
            }
            toml::Value::Table(t)
        }
    }
}

/// Resolve an agent name-or-id to a UUID by querying the daemon.
pub(crate) fn resolve_agent_id(base: &str, name_or_id: &str) -> String {
    if uuid::Uuid::try_parse(name_or_id).is_ok() {
        return name_or_id.to_string();
    }
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/agents")).send());
    let agents = body
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array());
    if let Some(arr) = agents {
        if let Some(agent) = arr.iter().find(|a| a["name"].as_str() == Some(name_or_id)) {
            if let Some(id) = agent["id"].as_str() {
                return id.to_string();
            }
        }
    }
    name_or_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use serde_json::json;

    #[test]
    fn date_from_unix_secs_matches_known_dates() {
        assert_eq!(date_from_unix_secs(0), "1970-01-01");
        // day-of-year 15 — within January
        assert_eq!(date_from_unix_secs(1_768_435_200), "2026-01-15");
        // last day of January
        assert_eq!(date_from_unix_secs(1_769_817_600), "2026-01-31");
        // first day of March (cross-February boundary)
        assert_eq!(date_from_unix_secs(1_772_323_200), "2026-03-01");
        // mid-year
        assert_eq!(date_from_unix_secs(1_781_049_600), "2026-06-10");
        // year-end
        assert_eq!(date_from_unix_secs(1_798_675_200), "2026-12-31");
        // leap day
        assert_eq!(date_from_unix_secs(1_709_164_800), "2024-02-29");
        // year-end in a leap year (366th day)
        assert_eq!(date_from_unix_secs(1_735_603_200), "2024-12-31");
    }

    #[test]
    fn usable_error_requires_non_empty_error_string() {
        assert!(body_carries_usable_error(
            &json!({ "error": "validation failed" })
        ));
        // Empty / whitespace-only error strings are not usable messages.
        assert!(!body_carries_usable_error(&json!({ "error": "" })));
        assert!(!body_carries_usable_error(&json!({ "error": "   " })));
        // Non-string `error` (object/array/number/null) is not a usable message.
        assert!(!body_carries_usable_error(
            &json!({ "error": { "code": 1 } })
        ));
        assert!(!body_carries_usable_error(&json!({ "error": null })));
        // Object without an `error` key, and non-object bodies.
        assert!(!body_carries_usable_error(&json!({ "ok": true })));
        assert!(!body_carries_usable_error(&json!("plain string")));
        // Null is what `unwrap_or_default()` yields for a non-JSON body.
        assert!(!body_carries_usable_error(&serde_json::Value::Null));
    }

    #[test]
    fn server_error_is_always_surfaced() {
        // 5xx is a daemon error regardless of body shape (historical behaviour).
        let null = serde_json::Value::Null;
        assert!(should_surface_status_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &null
        ));
        assert!(should_surface_status_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &json!({ "error": "boom" })
        ));
    }

    #[test]
    fn client_error_without_usable_body_is_surfaced() {
        // #6019 / #6017: a 405 (PUT-only route POSTed) with a non-JSON body
        // deserializes to Null; previously this slipped through as success.
        let null = serde_json::Value::Null;
        assert!(should_surface_status_error(
            StatusCode::METHOD_NOT_ALLOWED,
            &null
        ));
        // 404/400 with an `error`-less JSON object are equally surfaced.
        assert!(should_surface_status_error(
            StatusCode::NOT_FOUND,
            &json!({ "message": "not found" })
        ));
        assert!(should_surface_status_error(
            StatusCode::BAD_REQUEST,
            &json!({ "error": "" })
        ));
    }

    #[test]
    fn client_error_with_usable_body_is_left_for_caller() {
        // A 4xx that carries a real `error` key is returned for the caller's
        // own command-specific handler — not hard-errored here.
        assert!(!should_surface_status_error(
            StatusCode::BAD_REQUEST,
            &json!({ "error": "schedule expression is invalid" })
        ));
        assert!(!should_surface_status_error(
            StatusCode::CONFLICT,
            &json!({ "error": "already exists" })
        ));
    }

    #[test]
    fn success_status_is_never_surfaced() {
        let null = serde_json::Value::Null;
        assert!(!should_surface_status_error(StatusCode::OK, &null));
        assert!(!should_surface_status_error(
            StatusCode::CREATED,
            &json!({ "error": "ignored on 2xx" })
        ));
    }
}
