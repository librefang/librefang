//! `status` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

/// Render the daemon status page.
///
/// Layered data model:
/// - **Local** (always available): `daemon.json` + `config.toml` fields the
///   CLI reads directly, so we never show `?` for information we already have.
/// - **Public** (daemon alive, no auth): `/api/health` for liveness.
/// - **Authenticated** (requires `api_key`): `/api/status` for agent list,
///   session count, and memory usage. When the key is missing we show a
///   locked section with a one-line fix hint instead of leaking empty fields.
pub(crate) fn cmd_status(
    config: Option<PathBuf>,
    json: bool,
    verbose: bool,
    quiet: bool,
    watch: Option<u64>,
) {
    if let Some(secs) = watch {
        let interval = std::time::Duration::from_secs(secs.max(1));
        // Watch mode: redraw indefinitely. A non-zero exit code from a single
        // iteration just means "daemon is currently down or degraded" — we
        // don't bail out of the watch loop for that, the whole point is to
        // keep watching. Ctrl+C (handled upstream in main) is the exit.
        loop {
            // ANSI: clear screen + home cursor. Falls back to ugly output on
            // terminals that don't speak ANSI, which is acceptable for a
            // mode the user explicitly opted into.
            print!("\x1b[2J\x1b[H");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = render_status_once(config.clone(), false, verbose, false);
            ui::blank();
            println!(
                "  {} (refreshing every {}s, Ctrl+C to exit)",
                "hint:".dimmed(),
                secs.max(1),
            );
            std::thread::sleep(interval);
        }
    }

    let code = render_status_once(config, json, verbose, quiet);
    if code != 0 {
        std::process::exit(code);
    }
}

pub(crate) fn render_status_once(
    config: Option<PathBuf>,
    json: bool,
    verbose: bool,
    quiet: bool,
) -> i32 {
    let daemon = daemon_config_context(config.as_deref());
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        render_status_daemon(config.as_deref(), &base, &daemon, json, verbose, quiet)
    } else {
        render_status_inprocess(config, json, quiet)
    }
}

pub(crate) fn render_status_daemon(
    config: Option<&std::path::Path>,
    base: &str,
    daemon: &DaemonConfigContext,
    json: bool,
    verbose: bool,
    quiet: bool,
) -> i32 {
    let info = read_daemon_info(&daemon.home_dir);
    let (health, health_latency) = fetch_health_timed(base);
    let detail = daemon
        .api_key
        .as_deref()
        .and_then(|k| fetch_status_detail(base, k));
    let cfg = load_config(config).unwrap_or_else(|e| {
        eprintln!("warning: {e}; using default config values for status display");
        librefang_types::config::KernelConfig::default()
    });

    let exit_code = classify_exit(health.as_ref());
    let is_public_bind = info
        .as_ref()
        .map(|i| listener_is_public(&i.listen_addr))
        .unwrap_or(false);
    let (key_env, key_present, key_required) = provider_key_state(&cfg);
    let uptime = uptime_secs(info.as_ref(), detail.as_ref());

    if quiet {
        return render_status_quiet_daemon(
            base,
            info.as_ref(),
            health.as_ref(),
            detail.as_ref(),
            uptime,
            exit_code,
        );
    }

    if json {
        let merged = serde_json::json!({
            "daemon": true,
            "api": base,
            "dashboard": format!("{base}/"),
            "home": daemon.home_dir.display().to_string(),
            "daemon_info": info.as_ref().map(|i| serde_json::json!({
                "pid": i.pid,
                "listen_addr": i.listen_addr,
                "started_at": i.started_at,
                "version": i.version,
                "platform": i.platform,
                "publicly_bound": listener_is_public(&i.listen_addr),
            })),
            "health": health,
            "health_latency_ms": health_latency.map(|d| d.as_millis() as u64),
            "default_provider": cfg.default_model.provider,
            "default_model": cfg.default_model.model,
            "default_model_api_key_env": key_env,
            "default_model_api_key_present": key_present,
            "default_model_api_key_required": key_required,
            "detail": detail,
            "uptime_seconds": uptime_secs(info.as_ref(), detail.as_ref()),
            "authenticated": detail.is_some(),
            "exit_code": exit_code,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&merged).unwrap_or_default()
        );
        return exit_code;
    }

    // --- Overview -----------------------------------------------------------
    ui::section(&i18n::t("section-daemon-status"));
    ui::blank();

    let (status_label, status_good) = match health.as_ref() {
        Some(h) => match h["status"].as_str() {
            Some("ok") => ("ok".to_string(), true),
            Some(other) => (other.to_string(), false),
            None => ("unreachable".to_string(), false),
        },
        None => ("unreachable".to_string(), false),
    };
    if status_good {
        ui::kv_ok(&i18n::t("label-status"), &status_label);
    } else {
        ui::kv_warn(&i18n::t("label-status"), &status_label);
    }

    if let Some(v) = info
        .as_ref()
        .map(|i| i.version.as_str())
        .or_else(|| health.as_ref().and_then(|h| h["version"].as_str()))
    {
        ui::kv(&i18n::t("label-version"), v);
    }
    if let Some(info) = info.as_ref() {
        ui::kv(&i18n::t("label-pid"), &info.pid.to_string());
    }
    if let Some(u) = uptime_secs(info.as_ref(), detail.as_ref()) {
        ui::kv(&i18n::t("label-uptime"), &format_uptime(u));
    }
    if let Some(lat) = health_latency {
        ui::kv(&i18n::t("label-response"), &format_latency(lat));
    }
    // (B) 0.0.0.0 listener: surface the risk inline on the API row so the
    // user sees the bind scope without having to cross-reference anything.
    if is_public_bind {
        ui::kv_warn(
            &i18n::t("label-api"),
            &format!("{base}  \u{26A0} {}", i18n::t("warn-public-bind")),
        );
    } else {
        ui::kv(&i18n::t("label-api"), base);
    }
    ui::kv(&i18n::t("label-dashboard"), &format!("{base}/"));
    ui::kv(
        &i18n::t("label-home"),
        &daemon.home_dir.display().to_string(),
    );
    if let Some(info) = info.as_ref() {
        ui::kv(&i18n::t("label-platform"), &info.platform);
    }
    if let Some(bytes) = dir_size_bytes(&daemon.home_dir.join("data")) {
        ui::kv(&i18n::t("label-data-dir"), &format_bytes(bytes));
    }

    // --- Default model ------------------------------------------------------
    ui::blank();
    // (D) Missing provider key: show the concrete env-var name so the user
    // knows exactly which one to set.
    if key_required && !key_present {
        ui::kv_warn(
            &i18n::t("label-provider"),
            &format!(
                "{}  \u{26A0} {} {}",
                cfg.default_model.provider,
                key_env,
                i18n::t("warn-key-missing"),
            ),
        );
    } else {
        ui::kv(&i18n::t("label-provider"), &cfg.default_model.provider);
    }
    ui::kv(&i18n::t("label-model"), &cfg.default_model.model);

    // --- Health checks (C: always list all, not just degraded) --------------
    if let Some(h) = health.as_ref() {
        if let Some(checks) = h["checks"].as_array() {
            if !checks.is_empty() {
                ui::blank();
                ui::section(&i18n::t("label-checks"));
                for c in checks {
                    let name = c["name"].as_str().unwrap_or("?");
                    let st = c["status"].as_str().unwrap_or("?");
                    if st == "ok" {
                        ui::kv_ok(name, st);
                    } else {
                        ui::kv_warn(name, st);
                    }
                }
            }
        }
    }

    // --- Detail tier --------------------------------------------------------
    match detail.as_ref() {
        Some(body) => render_detail_section(body),
        None => {
            ui::blank();
            ui::section(&i18n::t("section-status-locked"));
            ui::hint(&i18n::t("hint-status-locked"));
        }
    }

    // --- Verbose extras -----------------------------------------------------
    if verbose {
        render_verbose_section(base, &cfg, detail.is_some(), daemon.api_key.as_deref());
    }

    // --- Recent errors (always, if any) -------------------------------------
    let errors = recent_daemon_errors(&daemon.home_dir, 3);
    if !errors.is_empty() {
        ui::blank();
        ui::section(&i18n::t("section-recent-errors"));
        for line in &errors {
            println!("    {}", line.red());
        }
    }

    exit_code
}

/// Map health response to a semantic exit code.
///
/// - `0` — daemon running and `/api/health` reports `ok`.
/// - `2` — daemon running but `/api/health` reports a non-ok status
///   (`degraded`, `error`, anything else the handler introduces later).
/// - `3` — daemon claims to be listening (we got a `/api/health` URL from
///   `daemon.json`) but the request didn't yield parseable JSON — the
///   process is unreachable even though the port is.
pub(crate) fn classify_exit(health: Option<&serde_json::Value>) -> i32 {
    match health.and_then(|h| h["status"].as_str()) {
        Some("ok") => 0,
        Some(_) => 2,
        None => 3,
    }
}

/// Heuristic for "this port is reachable from the internet if the machine
/// has a public IP." Catches the two common foot-guns: `0.0.0.0` (IPv4 any)
/// and `::` / `[::]` (IPv6 any). IPv4 loopback, IPv6 loopback, and named
/// localhost stay quiet.
pub(crate) fn listener_is_public(listen_addr: &str) -> bool {
    let host = listen_addr
        .rsplit_once(':')
        .map(|(h, _)| h.trim_start_matches('[').trim_end_matches(']'))
        .unwrap_or(listen_addr);
    matches!(host, "0.0.0.0" | "::" | "[::]")
}

/// Compute whether the configured default provider has a usable API key in
/// the environment (or in `provider_api_keys` in config.toml). Local
/// providers (ollama/vllm/lmstudio/lemonade) don't need one.
pub(crate) fn provider_key_state(
    cfg: &librefang_types::config::KernelConfig,
) -> (String, bool, bool) {
    let provider = cfg.default_model.provider.as_str();
    let key_required = !librefang_runtime::provider_health::is_local_provider(provider);
    let key_env = if cfg.default_model.api_key_env.trim().is_empty() {
        format!("{}_API_KEY", provider.to_uppercase())
    } else {
        cfg.default_model.api_key_env.clone()
    };
    let env_has_key = std::env::var(&key_env)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let config_has_key = cfg
        .provider_api_keys
        .get(provider)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    (key_env, env_has_key || config_has_key, key_required)
}

/// Scan the last chunk of `daemon.log` for ERROR-level entries. We read a
/// capped suffix of the file so a multi-GB log doesn't blow up memory, then
/// walk it backwards and collect the most recent N. An empty result means
/// either the log is missing or genuinely has no recent errors — the caller
/// treats both the same way (no section rendered).
pub(crate) fn recent_daemon_errors(home_dir: &std::path::Path, limit: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let log = home_dir.join("daemon.log");
    let mut file = match std::fs::File::open(&log) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    const TAIL_BYTES: u64 = 128 * 1024;
    let start = len.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return Vec::new();
    }
    buf.lines()
        .rev()
        // Match the tracing-subscriber default format. ` ERROR ` with padding
        // before and after is specific enough to avoid false positives from
        // log lines that happen to contain the word "error".
        .filter(|line| line.contains(" ERROR ") || line.starts_with("ERROR "))
        .take(limit)
        .map(|l| l.trim_end().to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// One-line quiet summary for `librefang status -q`. Stays stable across
/// releases so scripts can parse it — prefix is always `librefang`, second
/// token is a state word, remaining tokens are `key=value`.
pub(crate) fn render_status_quiet_daemon(
    base: &str,
    info: Option<&librefang_api::server::DaemonInfo>,
    health: Option<&serde_json::Value>,
    detail: Option<&serde_json::Value>,
    uptime: Option<u64>,
    exit_code: i32,
) -> i32 {
    let state = match health.and_then(|h| h["status"].as_str()) {
        Some("ok") => "ok",
        Some(other) => other,
        None => "unreachable",
    };
    let version = info
        .map(|i| i.version.as_str())
        .or_else(|| health.and_then(|h| h["version"].as_str()))
        .unwrap_or("?");
    let uptime_s = uptime.map(format_uptime).unwrap_or_else(|| "?".to_string());
    let auth_s = if detail.is_some() {
        let agents = detail.and_then(|d| d["agent_count"].as_u64()).unwrap_or(0);
        format!("agents={agents}")
    } else {
        "locked".to_string()
    };
    println!("librefang {version} {state} uptime={uptime_s} {auth_s} ({base})");
    exit_code
}

/// Extra verbose-only section. Everything in here is best-effort: anything
/// that fails to load just isn't shown — we never stop the main render.
pub(crate) fn render_verbose_section(
    base: &str,
    cfg: &librefang_types::config::KernelConfig,
    authenticated: bool,
    api_key: Option<&str>,
) {
    ui::blank();
    ui::section(&i18n::t("section-verbose"));

    // --- Auth mode ----------------------------------------------------------
    let mut auth_bits: Vec<String> = Vec::new();
    if !cfg.api_key.trim().is_empty() {
        auth_bits.push(i18n::t("auth-api-key"));
    }
    // Dashboard auth / user keys live under [auth] in config. Detect by
    // presence of non-empty dashboard credentials so we don't depend on
    // features that may vary across versions.
    if !cfg.dashboard_pass_hash.trim().is_empty() || !cfg.dashboard_pass.trim().is_empty() {
        auth_bits.push(i18n::t("auth-dashboard-login"));
    }
    let auth_value = if auth_bits.is_empty() {
        i18n::t("auth-none")
    } else {
        auth_bits.join(" + ")
    };
    ui::kv(&i18n::t("label-auth"), &auth_value);

    // --- MCP server count ---------------------------------------------------
    let mcp_count = cfg.mcp_servers.len();
    if mcp_count > 0 {
        ui::kv(&i18n::t("label-mcp"), &mcp_count.to_string());
    }

    // --- OFP peers ----------------------------------------------------------
    // Pass the API key when we have one: `/api/network/status` is in the
    // dashboard-read allowlist, so it transitions from public to
    // auth-required the moment `require_auth_for_reads` kicks in (which
    // happens automatically as soon as *any* auth is configured).
    if let Some((enabled, connected, total)) = fetch_peer_status(base, api_key) {
        if enabled {
            ui::kv(
                &i18n::t("label-peers"),
                &format!("{connected} connected / {total} known"),
            );
        }
    }

    // --- Authenticated counts ----------------------------------------------
    if authenticated {
        if let Some(key) = api_key {
            if let Some(n) = fetch_array_count(base, "/api/channels", key) {
                ui::kv(&i18n::t("label-channels"), &n.to_string());
            }
            if let Some(n) = fetch_array_count(base, "/api/skills", key) {
                ui::kv(&i18n::t("label-skills"), &n.to_string());
            }
            if let Some(n) = fetch_array_count(base, "/api/hands", key) {
                ui::kv(&i18n::t("label-hands"), &n.to_string());
            }
        }
    }

    // --- Config warnings ----------------------------------------------------
    let warnings = cfg.validate();
    if !warnings.is_empty() {
        ui::blank();
        ui::section(&i18n::t("label-config-warnings"));
        for w in warnings {
            ui::check_warn(&w);
        }
    }
}

pub(crate) fn fetch_peer_status(base: &str, api_key: Option<&str>) -> Option<(bool, u64, u64)> {
    let client = daemon_client_with_api_key(api_key);
    let resp = client
        .get(format!("{base}/api/network/status"))
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().ok()?;
    let enabled = body["enabled"].as_bool().unwrap_or(false);
    let connected = body["connected_peers"].as_u64().unwrap_or(0);
    let total = body["total_peers"].as_u64().unwrap_or(0);
    Some((enabled, connected, total))
}

pub(crate) fn fetch_array_count(base: &str, path: &str, api_key: &str) -> Option<u64> {
    let client = daemon_client_with_api_key(Some(api_key));
    let resp = client.get(format!("{base}{path}")).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().ok()?;
    // Dashboard endpoints vary: agents/skills/hands/channels each shape
    // their list differently. Probe in order: bare array → {total} →
    // common array keys. Whichever matches first wins; if none do we
    // return None so the caller quietly omits the row.
    if let Some(a) = body.as_array() {
        return Some(a.len() as u64);
    }
    if let Some(n) = body["total"].as_u64() {
        return Some(n);
    }
    for key in ["items", "channels", "skills", "hands", "agents"] {
        if let Some(a) = body[key].as_array() {
            return Some(a.len() as u64);
        }
    }
    None
}

pub(crate) fn render_detail_section(body: &serde_json::Value) {
    let total = body["agent_count"].as_u64().unwrap_or(0);
    let active = body["active_agent_count"].as_u64().unwrap_or(0);
    let sessions = body["session_count"].as_u64().unwrap_or(0);
    let memory_mb = body["memory_used_mb"].as_u64();

    ui::blank();
    ui::kv(
        &i18n::t("label-agents"),
        &format!("{active} running / {total} total"),
    );
    ui::kv(&i18n::t("label-sessions"), &sessions.to_string());
    if let Some(mb) = memory_mb {
        ui::kv(&i18n::t("label-memory"), &format!("{mb} MB"));
    }

    if let Some(agents) = body["agents"].as_array() {
        if !agents.is_empty() {
            ui::blank();
            ui::section(&i18n::t("section-active-agents"));
            render_agents_table(agents);
        }
    }
}

/// Render the agent list as a column-aligned table. Empty input is a no-op
/// so the caller can unconditionally call this after a non-empty check.
pub(crate) fn render_agents_table(agents: &[serde_json::Value]) {
    // Cap ID column at 12 so we don't push the model column off the screen
    // — users rarely need more than a handful of id bytes for correlation.
    const ID_TRIM: usize = 12;
    let id_trim = |s: &str| -> String {
        if s.len() <= ID_TRIM {
            s.to_string()
        } else {
            s.chars().take(ID_TRIM).collect()
        }
    };

    // Migrated to crate::table::Table (#3306) — keeps content layout stable
    // while removing 30+ lines of manual width math and giving us automatic
    // ASCII fallback when stdout is piped.
    let mut t = crate::table::Table::new(&["NAME", "ID", "STATE", "MODEL"]);
    for a in agents {
        let id = id_trim(a["id"].as_str().unwrap_or("?"));
        let model = format!(
            "{}:{}",
            a["model_provider"].as_str().unwrap_or("?"),
            a["model_name"].as_str().unwrap_or("?"),
        );
        t.add_row(&[
            a["name"].as_str().unwrap_or("?"),
            id.as_str(),
            a["state"].as_str().unwrap_or("?"),
            model.as_str(),
        ]);
    }
    t.print();
}

pub(crate) fn render_status_inprocess(config: Option<PathBuf>, json: bool, quiet: bool) -> i32 {
    // Quiet mode short-circuits the kernel boot — we don't need to load 22
    // workflow templates just to print "daemon down". Pull what we can from
    // the config file alone.
    if quiet {
        let cfg = load_config(config.as_deref()).unwrap_or_else(|e| {
            eprintln!("warning: {e}; using default config values for status display");
            librefang_types::config::KernelConfig::default()
        });
        println!(
            "librefang down home={} default={}/{}",
            cfg.home_dir.display(),
            cfg.default_model.provider,
            cfg.default_model.model,
        );
        return 1;
    }

    let kernel = boot_kernel(config);
    let agent_count = kernel.agent_registry_ref().count();
    let cfg = kernel.config_ref();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "in-process",
                "agent_count": agent_count,
                "home": cfg.home_dir.display().to_string(),
                "data_dir": cfg.data_dir.display().to_string(),
                "data_dir_bytes": dir_size_bytes(&cfg.data_dir),
                "default_provider": cfg.default_model.provider,
                "default_model": cfg.default_model.model,
                "daemon": false,
                "exit_code": 1,
            }))
            .unwrap_or_default()
        );
        return 1;
    }

    ui::section(&i18n::t("section-status-inprocess"));
    ui::blank();
    ui::kv(&i18n::t("label-agents"), &agent_count.to_string());
    ui::kv(&i18n::t("label-provider"), &cfg.default_model.provider);
    ui::kv(&i18n::t("label-model"), &cfg.default_model.model);
    ui::kv(&i18n::t("label-home"), &cfg.home_dir.display().to_string());
    if let Some(bytes) = dir_size_bytes(&cfg.data_dir) {
        ui::kv(
            &i18n::t("label-data-dir"),
            &format!("{} ({})", cfg.data_dir.display(), format_bytes(bytes)),
        );
    } else {
        ui::kv(
            &i18n::t("label-data-dir"),
            &cfg.data_dir.display().to_string(),
        );
    }
    ui::kv_warn(
        &i18n::t("label-daemon"),
        &i18n::t("label-daemon-not-running"),
    );
    ui::blank();
    ui::hint(&i18n::t("hint-run-start"));

    if agent_count > 0 {
        ui::blank();
        ui::section(&i18n::t("section-persisted-agents"));
        for entry in kernel.agent_registry_ref().list() {
            println!("    {} ({}) -- {:?}", entry.name, entry.id, entry.state);
        }
    }

    1
}

/// Fetch the public `/api/health` payload along with the round-trip time.
/// Returns `(None, None)` on network failure and `(None, Some(_))` when the
/// server responded but the body didn't parse, so the caller can still
/// surface "responded in 42ms but unreadable" if needed.
pub(crate) fn fetch_health_timed(
    base: &str,
) -> (Option<serde_json::Value>, Option<std::time::Duration>) {
    let client = daemon_client_with_api_key(None);
    let start = std::time::Instant::now();
    let resp = match client.get(format!("{base}/api/health")).send() {
        Ok(r) => r,
        Err(_) => return (None, None),
    };
    let elapsed = start.elapsed();
    if !resp.status().is_success() {
        return (None, Some(elapsed));
    }
    (resp.json::<serde_json::Value>().ok(), Some(elapsed))
}

/// Fetch the authenticated `/api/status` payload. Returns `None` on any
/// failure — including 401 — so the renderer falls back to the locked
/// section rather than printing `?` for every field.
pub(crate) fn fetch_status_detail(base: &str, api_key: &str) -> Option<serde_json::Value> {
    let client = daemon_client_with_api_key(Some(api_key));
    let resp = client.get(format!("{base}/api/status")).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<serde_json::Value>().ok()
}

/// Prefer authoritative uptime from the daemon; fall back to `now - started_at`
/// from `daemon.json` when the detail tier is unavailable.
pub(crate) fn uptime_secs(
    info: Option<&librefang_api::server::DaemonInfo>,
    detail: Option<&serde_json::Value>,
) -> Option<u64> {
    if let Some(body) = detail {
        if let Some(u) = body["uptime_seconds"].as_u64() {
            return Some(u);
        }
    }
    let info = info?;
    let started = chrono::DateTime::parse_from_rfc3339(&info.started_at).ok()?;
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(started.with_timezone(&chrono::Utc));
    u64::try_from(delta.num_seconds()).ok()
}

pub(crate) fn cmd_health(json: bool) {
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            let body = daemon_json(client.get(format!("{base}/api/health")).send());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                );
                return;
            }
            ui::success(&i18n::t("health-ok"));
            if let Some(status) = body["status"].as_str() {
                ui::kv(&i18n::t("label-status"), status);
            }
            if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                let hours = uptime / 3600;
                let mins = (uptime % 3600) / 60;
                ui::kv(&i18n::t("label-uptime"), &format!("{hours}h {mins}m"));
            }
        }
        None => {
            if json {
                println!("{}", serde_json::json!({"error": "daemon not running"}));
                std::process::exit(1);
            }
            ui::error(&i18n::t("health-not-running"));
            ui::hint(&i18n::t("hint-start-daemon"));
            std::process::exit(1);
        }
    }
}
