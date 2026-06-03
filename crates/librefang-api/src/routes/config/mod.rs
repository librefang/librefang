//! Health, status, configuration, security, and migration handlers.

use super::AppState;
use crate::types::*;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::config_reload::{validate_config_for_reload, HotAction};
use std::sync::Arc;

mod manage;
mod migration;
mod security;
mod system;

pub use manage::*;
pub use migration::*;
pub use security::*;
pub use system::*;

/// Build routes for the config/health/security/migration domain.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .route("/metrics", axum::routing::get(prometheus_metrics))
        .route("/health", axum::routing::get(health))
        .route("/health/detail", axum::routing::get(health_detail))
        .route("/status", axum::routing::get(status))
        .route(
            "/dashboard/snapshot",
            axum::routing::get({
                |State(state): State<Arc<AppState>>| async move {
                    axum::Json(dashboard_snapshot_inner(&state).await)
                }
            }),
        )
        .route("/version", axum::routing::get(version))
        .route("/config", axum::routing::get(get_config))
        .route("/config/export", axum::routing::get(export_config))
        .route("/config/schema", axum::routing::get(config_schema))
        .route("/config/set", axum::routing::post(config_set))
        .route("/config/reload", axum::routing::post(config_reload))
        .route("/security", axum::routing::get(security_status))
        .route("/migrate/detect", axum::routing::get(migrate_detect))
        .route("/migrate/scan", axum::routing::post(migrate_scan))
        .route("/migrate", axum::routing::post(run_migrate))
        .route("/shutdown", axum::routing::post(shutdown))
        .route("/init", axum::routing::post(quick_init))
}

/// Best-effort host identifier for the machine running the daemon.
///
/// Exposed only via authenticated endpoints (`/api/status`,
/// `/api/dashboard/snapshot`) — deliberately **not** surfaced on the
/// public `/api/version` endpoint, because hostname is a per-machine
/// identifier that a remote scanner could correlate to a specific
/// deployment target. `$HOSTNAME` is honoured first for parity with
/// containers that synthesise it; `hostname(1)` is the POSIX fallback.
/// Returns `None` only when both fail (rare).
fn system_hostname() -> Option<String> {
    if let Ok(h) = std::env::var("HOSTNAME") {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    #[cfg(unix)]
    {
        std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Best-effort RSS memory probe for the running process, in MB.
///
/// Shared between `/api/status` and `/api/dashboard/snapshot` so both
/// endpoints surface the same number. Returns `None` on platforms where
/// neither `ps` nor `tasklist` is available, or when parsing the output
/// fails — callers should render a placeholder in that case rather than
/// treating `0` as a real reading.
fn current_process_rss_mb() -> Option<u64> {
    #[cfg(unix)]
    {
        std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|kb| kb / 1024)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("tasklist")
            .args([
                "/FI",
                &format!("PID eq {}", std::process::id()),
                "/FO",
                "CSV",
                "/NH",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| {
                // tasklist CSV: "name","pid","session","session#","mem usage"
                let fields: Vec<&str> = s.trim().split(',').collect();
                fields
                    .last()
                    .map(|v| {
                        v.trim_matches('"')
                            .replace(" K", "")
                            .replace(",", "")
                            .replace(" ", "")
                    })
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|kb| kb / 1024)
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Returns `true` when at least one web search provider is configured —
/// either an API-key-based provider with its env var set, or SearXNG with a
/// non-empty URL. Drives the dashboard's "Configure API key" warning chip;
/// must stay in sync with the providers actually wired into the search
/// runtime, otherwise the UI nags users who already have a working setup.
fn is_web_search_configured(web: &librefang_types::config::WebConfig) -> bool {
    let env_set = |env_var: &str| {
        std::env::var(env_var)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .is_some()
    };
    !web.searxng.url.trim().is_empty()
        || env_set(&web.tavily.api_key_env)
        || env_set(&web.brave.api_key_env)
        || env_set(&web.jina.api_key_env)
        || env_set(&web.perplexity.api_key_env)
}

fn redacted_web(web: &librefang_types::config::WebConfig) -> serde_json::Value {
    serde_json::json!({
        "search_provider": format!("{:?}", web.search_provider),
        "cache_ttl_minutes": web.cache_ttl_minutes,
        "search_available": is_web_search_configured(web),
        "brave": {
            "api_key_env": web.brave.api_key_env,
            "max_results": web.brave.max_results,
            "country": web.brave.country,
            "search_lang": web.brave.search_lang,
            "freshness": web.brave.freshness,
        },
        "tavily": {
            "api_key_env": web.tavily.api_key_env,
            "search_depth": web.tavily.search_depth,
            "max_results": web.tavily.max_results,
            "include_answer": web.tavily.include_answer,
        },
        "perplexity": {
            "api_key_env": web.perplexity.api_key_env,
            "model": web.perplexity.model,
        },
        "jina": {
            "api_key_env": web.jina.api_key_env,
            "max_results": web.jina.max_results,
            "country": web.jina.country,
            "language": web.jina.language,
            "use_eu_endpoint": web.jina.use_eu_endpoint,
            "no_cache": web.jina.no_cache,
        },
        "searxng": {
            "url": web.searxng.url,
        },
        "fetch": {
            "max_chars": web.fetch.max_chars,
            "max_response_bytes": web.fetch.max_response_bytes,
            "timeout_secs": web.fetch.timeout_secs,
            "readability": web.fetch.readability,
        },
    })
}

// ---------------------------------------------------------------------------
// Health-detail derived-metrics cache (#3776)
//
// `query_model_performance()` runs a `GROUP BY model` over `usage_events`,
// which can grow unbounded. The health endpoint is often probed every few
// seconds by external monitors (Prometheus blackbox, k8s readiness, etc.) so
// we memoize the derived snapshot for `HEALTH_METRICS_TTL` to keep the probe
// cheap. The TTL is short enough that operators still see fresh data.
// ---------------------------------------------------------------------------
const HEALTH_METRICS_TTL: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Clone)]
struct LlmHealthSnapshot {
    /// Total LLM calls aggregated across every model in `usage_events`.
    total_calls: u64,
    /// Call-count-weighted mean latency in milliseconds across all models.
    avg_latency_ms: f64,
    /// Highest single-call latency observed across all models.
    max_latency_ms: u64,
    /// Number of distinct models seen.
    model_count: usize,
}

static LLM_HEALTH_CACHE: std::sync::OnceLock<
    std::sync::Mutex<Option<(std::time::Instant, LlmHealthSnapshot)>>,
> = std::sync::OnceLock::new();

fn llm_health_snapshot(state: &AppState) -> LlmHealthSnapshot {
    let cell = LLM_HEALTH_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(guard) = cell.lock() {
        if let Some((ts, snap)) = guard.as_ref() {
            if ts.elapsed() < HEALTH_METRICS_TTL {
                return snap.clone();
            }
        }
    }

    let perf = state
        .kernel
        .memory_substrate()
        .usage()
        .query_model_performance()
        .unwrap_or_default();

    let total_calls: u64 = perf.iter().map(|m| m.call_count).sum();
    let weighted_sum: f64 = perf
        .iter()
        .map(|m| m.avg_latency_ms * m.call_count as f64)
        .sum();
    let avg_latency_ms = if total_calls > 0 {
        weighted_sum / total_calls as f64
    } else {
        0.0
    };
    let max_latency_ms = perf.iter().map(|m| m.max_latency_ms).max().unwrap_or(0);

    let snap = LlmHealthSnapshot {
        total_calls,
        avg_latency_ms,
        max_latency_ms,
        model_count: perf.len(),
    };

    if let Ok(mut guard) = cell.lock() {
        *guard = Some((std::time::Instant::now(), snap.clone()));
    }
    snap
}

/// Strip embedded `user:pass@` credentials from a URL, keeping host/port.
///
/// Used for telemetry / OTLP endpoints that may legitimately contain a
/// basic-auth tuple in the URL. Returns the input unchanged when no `@`
/// follows the scheme — i.e. when there is nothing to redact.
fn redact_url_credentials(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        if let Some(at_pos) = after_scheme.find('@') {
            let host_and_rest = &after_scheme[at_pos..]; // includes '@'
            return format!("{}://***{}", &url[..scheme_end], host_and_rest);
        }
    }
    url.to_string()
}

/// Known framework source directories under the user's OS home, used as the
/// migration source allow-list. Legacy OpenClaw aliases are included so the
/// existing `~/.clawdbot` / `~/.moldbot` / `~/.moltbot` layouts still import.
const MIGRATE_SOURCE_DIR_NAMES: &[&str] = &[
    ".openclaw",
    ".clawdbot",
    ".moldbot",
    ".moltbot",
    ".openfang",
    ".langchain",
    ".autogpt",
];

/// Build the containment allow-list for a migration *source* path: the
/// librefang home plus any known framework source directory that actually
/// exists under the OS home.
///
/// #5577 confined both source and target to the librefang home, which
/// regressed the documented "migrate from `~/.openclaw`" flow — the source
/// dirs are siblings of `~/.librefang`, not descendants, so a real
/// `source_dir: "~/.openclaw"` was rejected. Only *existing* directories are
/// added: `validate_path_containment` rejects a non-canonicalizable root with
/// a 500, so a missing `~/.autogpt` must never enter the list. Migration
/// targets are deliberately NOT widened by this list — writes stay confined
/// to the librefang home.
fn migrate_source_roots(
    librefang_home: &std::path::Path,
    os_home: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut roots = vec![librefang_home.to_path_buf()];
    if let Some(home) = os_home {
        for name in MIGRATE_SOURCE_DIR_NAMES {
            let dir = home.join(name);
            if dir.is_dir() {
                roots.push(dir);
            }
        }
    }
    roots
}

/// Section grouping for the ConfigPage UI. Each entry carries the section key,
/// the fields that belong to it, and any flags the UI cares about
/// (root-level rendering, hot-reload safety).
#[doc(hidden)]
pub fn ui_sections_overlay() -> serde_json::Value {
    serde_json::json!([
        {
            "key": "general",
            "root_level": true,
            "fields": [
                "api_listen", "api_key", "log_level", "network_enabled", "mode",
                "language", "usage_footer", "stable_prefix_mode", "prompt_caching",
                "max_cron_jobs", "agent_max_iterations", "workspaces_dir",
                // Newly surfaced root-level scalars (#4678).
                "update_channel", "max_history_messages", "max_upload_size_bytes",
                "max_concurrent_bg_llm", "max_agent_call_depth", "max_request_body_bytes",
                "workflow_stale_timeout_minutes", "workflow_default_total_timeout_secs",
                "tool_timeout_secs",
                "local_probe_interval_secs", "require_auth_for_reads",
                "external_auth_proxy",
                "dashboard_user", "log_dir", "data_dir", "home_dir",
                "cors_origin", "trust_forwarded_for",
                "cron_session_max_tokens", "cron_session_max_messages",
                "cron_session_warn_fraction", "cron_session_warn_total_tokens",
                // Cron session compaction (#3693) — keep alongside the
                // other cron_session_* knobs so the dashboard renders them
                // as one cohesive cluster.
                "cron_session_compaction_mode", "cron_session_compaction_keep_recent",
                "strict_config"
            ]
        },
        {"key": "default_model", "struct_field": "default_model", "hot_reloadable": true},
        {"key": "memory", "struct_field": "memory"},
        {"key": "memory_wiki", "struct_field": "memory_wiki"},
        {"key": "proactive_memory", "struct_field": "proactive_memory"},
        {"key": "auto_dream", "struct_field": "auto_dream"},
        {"key": "web", "struct_field": "web"},
        {"key": "browser", "struct_field": "browser"},
        {"key": "network", "struct_field": "network"},
        {"key": "extensions", "struct_field": "extensions"},
        {"key": "vault", "struct_field": "vault"},
        {"key": "a2a", "struct_field": "a2a"},
        {"key": "channels", "struct_field": "channels"},
        {"key": "approval", "struct_field": "approval"},
        {"key": "exec_policy", "struct_field": "exec_policy"},
        {"key": "oauth", "struct_field": "oauth"},
        {"key": "external_auth", "struct_field": "external_auth"},
        {"key": "terminal", "struct_field": "terminal"},
        {"key": "docker", "struct_field": "docker"},
        {"key": "session", "struct_field": "session"},
        {"key": "queue", "struct_field": "queue"},
        {"key": "webhook_triggers", "struct_field": "webhook_triggers"},
        {"key": "vertex_ai", "struct_field": "vertex_ai"},
        {"key": "tts", "struct_field": "tts"},
        {"key": "canvas", "struct_field": "canvas"},
        {"key": "media", "struct_field": "media"},
        {"key": "links", "struct_field": "links"},
        {"key": "reload", "struct_field": "reload"},
        {"key": "budget", "struct_field": "budget"},
        {"key": "thinking", "struct_field": "thinking"},
        {"key": "pairing", "struct_field": "pairing"},
        {"key": "broadcast", "struct_field": "broadcast"},
        {"key": "auto_reply", "struct_field": "auto_reply"},
        // ── Newly exposed sub-struct sections (#4678) ──
        {"key": "llm", "struct_field": "llm"},
        {"key": "skills", "struct_field": "skills"},
        {"key": "triggers", "struct_field": "triggers"},
        {"key": "notification", "struct_field": "notification"},
        {"key": "task_board", "struct_field": "task_board"},
        {"key": "tool_policy", "struct_field": "tool_policy"},
        {"key": "context_engine", "struct_field": "context_engine"},
        {"key": "audit", "struct_field": "audit"},
        {"key": "health_check", "struct_field": "health_check"},
        {"key": "heartbeat", "struct_field": "heartbeat"},
        {"key": "plugins", "struct_field": "plugins"},
        {"key": "registry", "struct_field": "registry"},
        {"key": "hands", "struct_field": "hands"},
        {"key": "privacy", "struct_field": "privacy"},
        {"key": "sanitize", "struct_field": "sanitize"},
        {"key": "inbox", "struct_field": "inbox"},
        {"key": "telemetry", "struct_field": "telemetry"},
        {"key": "rl_export", "struct_field": "rl_export"},
        {"key": "prompt_intelligence", "struct_field": "prompt_intelligence"},
        {"key": "rate_limit", "struct_field": "rate_limit"},
        {"key": "tool_invoke", "struct_field": "tool_invoke"},
        {"key": "parallel_tools", "struct_field": "parallel_tools"},
        {"key": "tool_results", "struct_field": "tool_results"},
        {"key": "compaction", "struct_field": "compaction"},
        {"key": "gateway_compression", "struct_field": "gateway_compression"},
        {"key": "prompt_cache", "struct_field": "prompt_cache"},
        {"key": "azure_openai", "struct_field": "azure_openai"},
        {"key": "proxy", "struct_field": "proxy"},
        // Tool-exec backend selection (local / docker / daytona / ssh).
        {"key": "tool_exec", "struct_field": "tool_exec"},
        // ── Newly exposed collection-typed sections (#4678) ──
        {"key": "taint_rules", "struct_field": "taint_rules"},
        {"key": "fallback_providers", "struct_field": "fallback_providers"},
        {"key": "credential_pools", "struct_field": "credential_pools"},
        {"key": "sidecar_channels", "struct_field": "sidecar_channels"},
        {"key": "provider_urls", "struct_field": "provider_urls"},
        {"key": "provider_proxy_urls", "struct_field": "provider_proxy_urls"},
        {"key": "provider_regions", "struct_field": "provider_regions"},
        {"key": "provider_request_timeout_secs", "struct_field": "provider_request_timeout_secs"},
        {"key": "provider_max_retries", "struct_field": "provider_max_retries"},
        {"key": "tool_timeouts", "struct_field": "tool_timeouts"},
        // Background autonomous-loop executor knobs (#5168).
        {"key": "background", "struct_field": "background"}
    ])
}

/// Per-field UI hints keyed by JSON-pointer path (so the frontend doesn't
/// have to re-walk `$ref` chains). Carries numeric ranges, step granularity,
/// curated select options (with human labels when applicable), and dynamic
/// provider/model options sourced from the catalog.
#[doc(hidden)]
pub fn ui_options_overlay(
    provider_options: Vec<String>,
    model_options: Vec<serde_json::Value>,
) -> serde_json::Value {
    // Language labels — preserved from the previous hand-authored schema so
    // the UI keeps showing native-script names, not locale codes.
    let languages = serde_json::json!([
        {"value": "en", "label": "English"},
        {"value": "zh", "label": "中文"},
        {"value": "ja", "label": "日本語"},
        {"value": "ko", "label": "한국어"},
        {"value": "es", "label": "Español"},
        {"value": "fr", "label": "Français"},
        {"value": "de", "label": "Deutsch"},
        {"value": "it", "label": "Italiano"},
        {"value": "pt", "label": "Português"},
        {"value": "ru", "label": "Русский"},
        {"value": "ar", "label": "العربية"},
        {"value": "hi", "label": "हिन्दी"},
        {"value": "tr", "label": "Türkçe"},
        {"value": "pl", "label": "Polski"},
        {"value": "nl", "label": "Nederlands"},
        {"value": "vi", "label": "Tiếng Việt"},
        {"value": "th", "label": "ภาษาไทย"},
        {"value": "id", "label": "Bahasa Indonesia"}
    ]);

    serde_json::json!({
        // ── general (root-level KernelConfig fields) ──
        "/log_level": {"select": ["trace", "debug", "info", "warn", "error"]},
        "/mode": {"select": ["stable", "default", "dev"]},
        "/language": {"select": languages},
        "/usage_footer": {"select": ["off", "tokens", "cost", "full"]},
        "/max_cron_jobs": {"min": 0, "max": 100, "step": 1},
        "/agent_max_iterations": {"min": 1, "max": 500, "step": 1},

        // ── default_model ──
        "/default_model/provider": {"select": provider_options},
        "/default_model/model": {"select_objects": model_options},

        // ── memory ──
        "/memory/consolidation_threshold": {"min": 1, "max": 1_000_000, "step": 1},
        "/memory/decay_rate": {"min": 0, "max": 1, "step": 0.01},
        "/memory/embedding_provider": {"select": [
            "auto", "openai", "openrouter", "groq", "mistral", "together",
            "fireworks", "cohere", "ollama", "bedrock", "vllm", "lmstudio"
        ]},
        "/memory/consolidation_interval_hours": {
            "number_select": ["0", "1", "6", "12", "24", "48", "168"]
        },

        // ── proactive_memory ──
        "/proactive_memory/max_retrieve": {"min": 1, "max": 100, "step": 1},
        "/proactive_memory/extraction_threshold": {"min": 0, "max": 1, "step": 0.01},
        "/proactive_memory/session_ttl_hours": {"min": 1, "max": 8760, "step": 1},
        "/proactive_memory/confidence_decay_rate": {"min": 0, "max": 1, "step": 0.001},
        "/proactive_memory/duplicate_threshold": {"min": 0, "max": 1, "step": 0.01},
        "/proactive_memory/max_memories_per_agent": {"min": 0, "max": 100_000, "step": 100},

        // ── auto_dream ──
        "/auto_dream/min_hours": {"min": 0, "max": 168, "step": 0.5},
        "/auto_dream/min_sessions": {"min": 0, "max": 1000, "step": 1},
        "/auto_dream/check_interval_secs": {"min": 60, "max": 86_400, "step": 60},
        "/auto_dream/timeout_secs": {"min": 30, "max": 3600, "step": 30},

        // ── web ──
        "/web/search_provider": {"select": ["brave", "tavily", "perplexity", "jina", "searxng", "duck_duck_go", "auto"]},
        "/web/cache_ttl_minutes": {"min": 0, "max": 10_080, "step": 1},

        // ── browser ──
        "/browser/viewport_width": {"min": 320, "max": 3840, "step": 1},
        "/browser/viewport_height": {"min": 240, "max": 2160, "step": 1},
        "/browser/timeout_secs": {"min": 5, "max": 300, "step": 1},
        "/browser/idle_timeout_secs": {"min": 0, "max": 3600, "step": 1},
        "/browser/max_sessions": {"min": 1, "max": 20, "step": 1},

        // ── network ──
        "/network/max_peers": {"min": 1, "max": 1000, "step": 1},

        // ── extensions ──
        "/extensions/reconnect_max_attempts": {"min": 0, "max": 100, "step": 1},
        "/extensions/reconnect_max_backoff_secs": {"min": 1, "max": 3600, "step": 1},
        "/extensions/health_check_interval_secs": {"min": 5, "max": 3600, "step": 1},

        // ── terminal ──
        "/terminal/max_windows": {"min": 1, "max": 64, "step": 1},

        // ── rate_limit ──
        "/rate_limit/api_requests_per_minute": {"min": 0, "max": 100_000, "step": 100},
        "/rate_limit/retry_after_secs": {"min": 1, "max": 3600, "step": 1},
        "/rate_limit/max_ws_per_ip": {"min": 1, "max": 100, "step": 1},

        // ── triggers ──
        "/triggers/cooldown_secs": {"min": 0, "max": 3600, "step": 1},
        "/triggers/max_per_event": {"min": 1, "max": 1000, "step": 1},
        "/triggers/max_depth": {"min": 1, "max": 50, "step": 1},

        // ── compaction ──
        "/compaction/threshold_messages": {"min": 5, "max": 1000, "step": 1},
        "/compaction/keep_recent": {"min": 1, "max": 100, "step": 1},
        "/compaction/max_summary_tokens": {"min": 100, "max": 16_000, "step": 100},
        "/compaction/token_threshold_ratio": {"min": 0, "max": 1, "step": 0.05},

        // ── registry ──
        "/registry/cache_ttl_secs": {"min": 60, "max": 604_800, "step": 60},

        // ── health_check ──
        "/health_check/health_check_interval_secs": {"min": 5, "max": 3600, "step": 1},

        // ── heartbeat ──
        "/heartbeat/check_interval_secs": {"min": 5, "max": 3600, "step": 1},

        // ── inbox ──
        "/inbox/poll_interval_secs": {"min": 1, "max": 600, "step": 1},

        // ── audit ──
        "/audit/retention_days": {"min": 1, "max": 3650, "step": 1},

        // ── telemetry ──
        "/telemetry/sample_rate": {"min": 0, "max": 1, "step": 0.01},

        // ── parallel_tools ──
        "/parallel_tools/max_concurrent": {"min": 1, "max": 64, "step": 1},

        // ── tool_results ──
        "/tool_results/spill_threshold_bytes": {"min": 1024, "max": 10_485_760, "step": 1024}
    })
}

/// Allowlist of user-tunable config paths writable via POST /api/config/set
/// (#3458). Anything not in this list MUST be edited on disk.
///
/// Each entry is matched against the dot-separated path the caller supplies.
/// Trailing `.*` wildcards permit any single key under a section (used for
/// per-channel toggles like `channels.telegram.enabled`).
fn is_writable_config_path(path: &str) -> bool {
    // Exact-match list — single user-tunable scalars.
    const EXACT: &[&str] = &[
        // UI / locale (no security impact).
        "ui.theme",
        "ui.locale",
        "ui.timezone",
        "ui.language",
        "log_level",
        // History trim cap (gotcha bound by MIN_HISTORY_MESSAGES on reload).
        "max_history_messages",
        // Approval policy display knobs (NOT the second_factor enforcement
        // mode, NOT totp_* — those would let an Owner-role attacker silently
        // turn off 2FA after an API-key leak).
        "approval.auto_approve_autonomous",
        "approval.auto_approve",
        "approval.totp_grace_period_secs",
        // ── Newly user-tunable root-level scalars (#4678) ──
        // Update channel + size / depth caps; default model / mode flags;
        // localisation. Deliberately excludes `api_key`, `dashboard_pass*`,
        // `dashboard_user`, `cors_origin`, `trust_forwarded_for`,
        // `network_enabled`, `api_listen`, `trusted_*`, `home_dir`, `data_dir`,
        // `log_dir`, `cron_session_*`, and `require_auth_for_reads` — those
        // are infrastructure / auth knobs that need a deliberate file edit.
        "update_channel",
        "max_upload_size_bytes",
        "max_concurrent_bg_llm",
        "max_agent_call_depth",
        "max_request_body_bytes",
        "workflow_stale_timeout_minutes",
        "tool_timeout_secs",
        "local_probe_interval_secs",
        "prompt_caching",
        "stable_prefix_mode",
        "usage_footer",
        "language",
        "mode",
        "agent_max_iterations",
        "max_cron_jobs",
        // ── Collection-typed sections, primitive-valued only (#4678) ──
        // The dashboard's StringMapEditor / NumberMapEditor saves the
        // entire collection as one JSON value posted at the section's
        // bare path. Restricted to BTreeMap<String, String|u64> sections
        // because their value type is primitive — there is no nested
        // payload that could carry a credential past the path-string
        // SCRUB check. Vec<Struct> sections (sidecar_channels,
        // fallback_providers, taint_rules) are intentionally NOT here:
        // their items have nested fields (e.g. SidecarChannel.env) that
        // SCRUB_SUFFIXES — which only inspects the dotted path string —
        // cannot police inside a wholesale JSON payload.
        // `sidecar_channels` writes go through the dedicated
        // `POST /api/channels/sidecar/{name}/configure` endpoint, which
        // validates against the cached `--describe` schema and splits
        // secrets vs non-secrets across `secrets.env` and `config.toml`.
        // `fallback_providers` / `taint_rules` remain edit-on-disk for
        // now (round-4 review of #4678).
        "provider_urls",
        "provider_regions",
        "provider_proxy_urls",
        "provider_request_timeout_secs",
        "provider_max_retries",
        "tool_timeouts",
        // ── Round-5 review of #4678 — safe network knobs ──
        // The whole `network.` prefix was withdrawn (see SECTION_PREFIXES
        // comment below) because `network.bootstrap_peers` was reachable
        // as a depth-1 leaf and post-auth flips would redirect DHT
        // discovery to attacker-controlled peers. The display knobs
        // listed here have no peer-redirection or auth surface.
        // Excludes `listen_addresses` (binding 0.0.0.0 post-auth would
        // expose a previously loopback-only API surface — edit on disk),
        // and excludes `bootstrap_peers` / `shared_secret`.
        "network.mdns_enabled",
        "network.max_peers",
        "network.max_messages_per_peer_per_minute",
    ];
    if EXACT.contains(&path) {
        return true;
    }

    // Section prefixes — any leaf under these prefixes is allowed. The
    // section itself is NOT writable as a whole (would clobber the table),
    // because validate_config_key_path requires the path to have a leaf.
    const SECTION_PREFIXES: &[&str] = &[
        // Per-channel enable/feature toggles. Excludes `*.token` /
        // `*.shared_secret` because those keys are scrubbed below.
        "channels.",
        // Web search / fetch knobs (URLs and timeouts).
        "web.",
        // Rate-limit display knobs.
        "rate_limit.",
        // Queue / concurrency tuning.
        "queue.",
        // ── Newly user-tunable section prefixes (#4678) ──
        // Tool invocation / parallelism / result spill / policy.
        "tool_invoke.",
        "parallel_tools.",
        "tool_results.",
        "tool_policy.",
        // Per-tool timeout overrides — values are integers (seconds), no secrets.
        "tool_timeouts.",
        // Compaction & trigger system tuning.
        "compaction.",
        "triggers.",
        // Registry / inbox / health / heartbeat / notification.
        "registry.",
        "inbox.",
        "health_check.",
        "heartbeat.",
        "notification.",
        // Task board, prompt intelligence, context engine.
        "task_board.",
        "prompt_intelligence.",
        "context_engine.",
        // Auto-dream scheduler.
        "auto_dream.",
        // Media / link / TTS / canvas behaviour.
        "media.",
        "links.",
        "tts.",
        "canvas.",
        // Extensions reconnect tuning, session retention.
        "extensions.",
        "session.",
        // Memory tuning.
        "proactive_memory.",
        "memory.",
        // Browser / Docker sandbox / vault tuning. SCRUB_SUFFIXES still
        // blocks `*.api_key`, `*.password`, `*.bypass`, `*.admin`, `*.owner`.
        "browser.",
        "docker.",
        "vault.",
        // Pairing & A2A — token_env / shared_secret keys are blocked by SCRUB.
        "pairing.",
        "a2a.",
        // Sanitize / privacy display switches.
        "sanitize.",
        "privacy.",
        // Note: `audit.` and `telemetry.` are intentionally NOT here
        // (round-4 review of #4678). They expose `audit.anchor_path`
        // (Merkle tamper-detect target) and `telemetry.otlp_endpoint`
        // (trace export destination) — neither is acceptable to mutate
        // post-auth. Display knobs (sample_rate, retention_days) are
        // available via /api/config but not via /api/config/set; users
        // edit those on disk where the change leaves a file mtime trail.
        // Webhook trigger toggles (token / token_env still SCRUB-blocked).
        "webhook_triggers.",
        // Auto-reply / broadcast routing.
        "auto_reply.",
        "broadcast.",
        // Provider URL/region/timeout/proxy maps (URLs are public endpoints;
        // SCRUB-suffix list still blocks any `*.api_key` keys that snuck in).
        "provider_urls.",
        "provider_regions.",
        "provider_proxy_urls.",
        "provider_request_timeout_secs.",
        "provider_max_retries.",
        // Vertex AI region + Azure OpenAI configuration knobs (the
        // SCRUB suffix list still blocks api_key/_env/client_secret
        // entries embedded in either section).
        "vertex_ai.",
        "azure_openai.",
        // Note: `proxy.` is intentionally NOT here (round-4 review of
        // #4678). Owner-role posting `proxy.http_proxy` could MITM all
        // outbound LLM traffic in flight. The proxy URL is a system
        // boundary that should be edited on disk (file mtime trail).
        // Default model selection (provider/model/base_url; api_key SCRUB-blocked).
        "default_model.",
        // Extended thinking parameters.
        "thinking.",
        // Budget caps (USD ceilings, alert threshold, per-hour token cap).
        "budget.",
        // Reload mode/debounce.
        "reload.",
        // Note: `external_oauth.`, `external_auth.`, `oauth.` are
        // intentionally NOT here (round-4 review of #4678). They expose
        // `*.issuer_url`, `*.allowed_domains`, `*.redirect_url`,
        // `*.require_email_verified`, `*.client_id` — flipping any of
        // those post-auth lets an Owner-role attacker redirect login,
        // broaden the email allowlist, or skip email verification
        // (regression vector for #3703). SCRUB only blocks
        // `_secret_env` and the new `_env` suffix; non-secret-but-
        // load-bearing identity fields aren't in SCRUB. Edit on disk.
        // Terminal access controls.
        "terminal.",
        // Note: `network.` is intentionally NOT here (round-5 review of
        // #4678). `network.bootstrap_peers` was reachable as a depth-1
        // leaf and is a `Vec<String>`; an Owner-role attacker who flipped
        // it post-auth could redirect DHT discovery to attacker peers
        // (parallel threat model to the round-4 removal of `proxy.`
        // for outbound LLM MITM). Safe display knobs (`mdns_enabled`,
        // `max_peers`, `max_messages_per_peer_per_minute`) are EXACT-listed
        // above; everything else stays edit-on-disk.
        // Approval policy fields are intentionally NOT a section prefix:
        // the existing EXACT list above covers the safe display knobs
        // (`auto_approve_autonomous`, `auto_approve`, `totp_grace_period_secs`),
        // and the test suite asserts that `approval.second_factor` stays
        // closed — flipping it via the dashboard would let an Owner-role
        // attacker silently disable 2FA after an API-key leak.
        // Shell exec policy (timeouts, mode, allowed_env_vars list).
        "exec_policy.",
        // LLM auxiliary chains.
        "llm.",
        // Plugins / skills tuning.
        "plugins.",
        "skills.",
    ];
    // Section prefixes where the depth-1 leaf (vendor / collection-element)
    // is itself a struct containing credential-shaped fields that
    // SCRUB_SUFFIXES cannot police inside a wholesale JSON payload.
    // Writes against these prefixes must be depth-2 (per-leaf) only —
    // same defect class round-4 explicitly removed `sidecar_channels` /
    // `fallback_providers` / `taint_rules` for. Round-5 review of #4678.
    //
    // `channels.<vendor>` is `OneOrMany<*Config>` containing
    // `*_token_env` / `*_secret_env` / etc.; depth-1 wholesale-replacement
    // would let an Owner-role caller redirect the env-var that resolves
    // a bot/API token. Depth-2 (`channels.telegram.enabled` etc.) goes
    // through SCRUB_SUFFIXES which catches the `_env` blanket.
    const DEPTH_2_ONLY_PREFIXES: &[&str] = &["channels."];
    let in_section = SECTION_PREFIXES.iter().any(|pfx| {
        if !path.starts_with(pfx) {
            return false;
        }
        let rest = &path[pfx.len()..];
        if rest.is_empty() {
            return false;
        }
        let segments = rest.split('.').count();
        if DEPTH_2_ONLY_PREFIXES.contains(pfx) {
            segments == 2
        } else {
            // Single leaf (e.g. "web.search_provider") or one nested level
            // (e.g. "default_model.provider") — not deeper.
            segments == 1 || segments == 2
        }
    });
    if !in_section {
        return false;
    }

    // Within an allowed section, refuse keys that obviously carry secrets or
    // override security-critical knobs even if the operator points us at one
    // of the curated sections by name.
    const SCRUB_SUFFIXES: &[&str] = &[
        ".api_key",
        ".token",
        ".secret",
        ".shared_secret",
        ".password",
        ".bypass",
        ".admin",
        ".owner",
        // Round-4 review of #4678: env-var-name redirects. Codebase
        // pervasively uses `*_token_env`, `*_password_env`,
        // `*_secret_env`, `*_client_secret_env`, `*_api_key_env`,
        // `bot_token_env`, `access_token_env`, `cdp_auth_token_env`.
        // The original SCRUB only blocked literal `.api_key` etc., so
        // an attacker could repoint `<section>.api_key_env` at any env
        // var the daemon has access to and force a credential rotation
        // through a logged channel. The blanket `_env` suffix catches
        // every variant the workspace currently uses (verified by grep
        // against librefang-types/src/config/types.rs).
        "_env",
        // OAuth public identity that's safe to *display* but not safe
        // to mutate (issuer redirect / consent skipping). External
        // auth sections are mostly off the prefix list now, but defense
        // in depth in case anything slips through a writable section.
        ".client_id",
        ".client_secret",
    ];
    if SCRUB_SUFFIXES.iter().any(|s| path.ends_with(s)) {
        return false;
    }
    true
}

/// Convert a serde_json::Value to a toml_edit::Value (format-preserving).
fn json_to_toml_edit_value(value: &serde_json::Value) -> toml_edit::Value {
    match value {
        serde_json::Value::String(s) => s.as_str().into(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else if let Some(f) = n.as_f64() {
                f.into()
            } else {
                n.to_string().into()
            }
        }
        serde_json::Value::Bool(b) => (*b).into(),
        serde_json::Value::Array(arr) => {
            let mut a = toml_edit::Array::new();
            for item in arr {
                a.push(json_to_toml_edit_value(item));
            }
            toml_edit::Value::Array(a)
        }
        serde_json::Value::Object(map) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, v) in map {
                t.insert(k, json_to_toml_edit_value(v));
            }
            toml_edit::Value::InlineTable(t)
        }
        // null is handled by the caller (remove key) — fallback to empty string
        serde_json::Value::Null => "".into(),
    }
}

/// Convert a serde_json::Value to a toml::Value.
#[doc(hidden)]
pub fn json_to_toml_value(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_u64() {
                toml::Value::Integer(i as i64)
            } else if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Array(arr) => {
            toml::Value::Array(arr.iter().map(json_to_toml_value).collect())
        }
        serde_json::Value::Object(map) => {
            // Convert nested JSON objects into TOML tables. Without this, the
            // catch-all below would JSON-stringify the whole object, which is
            // how #2319 wrote `mcp_servers = ['{"name":"..."}']` into config.toml
            // and broke reload.
            let mut table = toml::map::Map::new();
            for (k, v) in map {
                table.insert(k.clone(), json_to_toml_value(v));
            }
            toml::Value::Table(table)
        }
        // Null has no TOML analogue — emit an empty string so the key still
        // round-trips; callers that care should filter before calling.
        serde_json::Value::Null => toml::Value::String(String::new()),
    }
}

/// GET /api/dashboard/snapshot — Single aggregated snapshot for the dashboard.
///
/// Replaces 7 parallel frontend requests (health, status, providers, channels,
/// skills, agents, workflows) with one round-trip, cutting poll overhead by ~7x.
pub async fn dashboard_snapshot(
    State(state): State<Arc<AppState>>,
) -> axum::Json<serde_json::Value> {
    axum::Json(dashboard_snapshot_inner(&state).await)
}

/// TTL for the [`dashboard_snapshot_inner`] memoization cache.
///
/// 900 ms is well below the dashboard's 5 s poll interval (so a polling tab
/// rebuilds on every tick and the data still feels "live"), but enough to
/// fold the burst of back-to-back polls that arrive when a user opens
/// multiple dashboard tabs, switches windows, or the page rapidly remounts
/// during a route change.
const DASHBOARD_SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_millis(900);

/// Cached aggregated payload for `/api/dashboard/snapshot`.
///
/// The payload is wrapped in an `Arc` so cache lookups clone the pointer,
/// not the (possibly large) JSON tree. The final return type of
/// [`dashboard_snapshot_inner`] is still `serde_json::Value`, so we pay one
/// `(*payload).clone()` per cache hit — still 10–100× cheaper than
/// re-running per-agent manifest enrichment + provider/channel probes +
/// memory queries.
struct CachedDashboardSnapshot {
    generated_at: std::time::Instant,
    payload: Arc<serde_json::Value>,
}

/// Process-wide cache for [`dashboard_snapshot_inner`], keyed by
/// `AppState` pointer identity.
///
/// We deliberately do **not** store the cache on `AppState` itself —
/// adding a field there ripples into `librefang-testing` and every
/// inline test that constructs an `AppState` literal (5+ call sites).
/// Keying by `Arc::as_ptr(state) as usize` instead gives every test its
/// own cache slot for free, while production (one long-lived `AppState`)
/// gets exactly one slot.
///
/// Entries are evicted opportunistically: on each lookup we discard the
/// caller's expired entry; on each insert we drop any entries older than
/// 60× the TTL, which is enough to prevent the test process from
/// accumulating slots over time without paying a global scan on the hot
/// path.
static DASHBOARD_SNAPSHOT_CACHE: std::sync::OnceLock<
    dashmap::DashMap<usize, CachedDashboardSnapshot>,
> = std::sync::OnceLock::new();

fn dashboard_snapshot_cache() -> &'static dashmap::DashMap<usize, CachedDashboardSnapshot> {
    DASHBOARD_SNAPSHOT_CACHE.get_or_init(dashmap::DashMap::new)
}

async fn dashboard_snapshot_inner(state: &Arc<AppState>) -> serde_json::Value {
    // Fast path: serve the memoized payload if it's still within TTL.
    // Keyed by the `AppState` Arc pointer so concurrent tests with
    // distinct kernels don't poison each other's cache.
    let cache_key = Arc::as_ptr(state) as usize;
    let cache = dashboard_snapshot_cache();
    if let Some(entry) = cache.get(&cache_key) {
        if entry.generated_at.elapsed() < DASHBOARD_SNAPSHOT_TTL {
            return (*entry.payload).clone();
        }
    }

    let payload = dashboard_snapshot_compute(state).await;
    let payload = Arc::new(payload);
    cache.insert(
        cache_key,
        CachedDashboardSnapshot {
            generated_at: std::time::Instant::now(),
            payload: Arc::clone(&payload),
        },
    );
    // Opportunistic prune so test processes that construct many
    // short-lived `AppState`s don't accumulate dead cache slots
    // indefinitely. The threshold is 60× TTL so production's single
    // long-lived state is never pruned, and we only walk the (tiny)
    // table when we're already taking the write path.
    let prune_threshold = DASHBOARD_SNAPSHOT_TTL * 60;
    cache.retain(|_, v| v.generated_at.elapsed() < prune_threshold);
    (*payload).clone()
}

async fn dashboard_snapshot_compute(state: &Arc<AppState>) -> serde_json::Value {
    // Health (same logic as /api/health)
    let shared_id = librefang_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]));
    let db_ok = state
        .kernel
        .memory_substrate()
        .structured_get(shared_id, "__health_check__")
        .is_ok();
    let health_status = if db_ok { "ok" } else { "degraded" };
    let fts_only = state.kernel.config_ref().memory.fts_only.unwrap_or(false);
    let embedding_ok = fts_only || state.kernel.embedding().is_some();
    let health = serde_json::json!({
        "status": health_status,
        "version": env!("CARGO_PKG_VERSION"),
        "checks": [
            { "name": "database", "status": if db_ok { "ok" } else { "error" } },
            { "name": "embedding", "status": if embedding_ok { "ok" } else { "warn" } },
        ],
    });

    // Status (same logic as /api/status, without the heavy per-agent list).
    // Read-only iteration; cheap Arc clones over full manifest deep-copy (#3569).
    let agent_entries = state.kernel.agent_registry().list_arcs();
    let agent_count = agent_entries.iter().filter(|e| !e.is_hand).count();
    let active_agent_count = agent_entries
        .iter()
        .filter(|e| !e.is_hand && matches!(e.state, librefang_types::agent::AgentState::Running))
        .count();
    // Same fix as `/api/status` above — indexed COUNT instead of
    // decoding every session blob just to call `.len()`. This is the
    // dashboard snapshot path (`/api/dashboard/snapshot`), hit on
    // every 5 s poll, so the cost compounded.
    let session_count = state
        .kernel
        .memory_substrate()
        .count_sessions()
        .unwrap_or(0);
    let cfg = state.kernel.config_snapshot();
    // Runtime stats shared with `/api/status` — the dashboard RuntimePage
    // reads these out of the snapshot for its info panel and KPI tiles.
    // Anything missing here renders as "-" on the page.
    let uptime_seconds = state.started_at.elapsed().as_secs();
    let memory_used_mb = current_process_rss_mb();
    let status = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "agent_count": agent_count,
        "active_agent_count": active_agent_count,
        "session_count": session_count,
        "uptime_seconds": uptime_seconds,
        "memory_used_mb": memory_used_mb,
        "default_provider": cfg.default_model.provider,
        "default_model": cfg.default_model.model,
        "config_exists": state.kernel.home_dir().join("config.toml").exists(),
        "api_listen": cfg.api_listen,
        "home_dir": state.kernel.home_dir().display().to_string(),
        "log_level": cfg.log_level,
        "hostname": system_hostname(),
        "network_enabled": cfg.network_enabled,
        "terminal_enabled": cfg.terminal.enabled,
    });

    // Agents list — fully enriched (same fields as /api/agents) so AgentsPage
    // can use this snapshot directly instead of polling /api/agents separately.
    let agents: Vec<serde_json::Value> = {
        let catalog_guard = state.kernel.model_catalog_ref().load();
        let catalog: Option<&librefang_kernel::model_catalog::ModelCatalog> = Some(&catalog_guard);
        let dm = {
            let dm_override = state
                .kernel
                .default_model_override_ref()
                .read()
                .unwrap_or_else(|e| e.into_inner());
            super::agents::effective_default_model(&cfg.default_model, dm_override.as_ref())
        };
        let mut agent_entries_visible: Vec<&std::sync::Arc<librefang_types::agent::AgentEntry>> =
            agent_entries.iter().collect();
        // Sort by last_active descending — matches AgentsPage default query order.
        agent_entries_visible.sort_by_key(|b| std::cmp::Reverse(b.last_active));
        agent_entries_visible
            .iter()
            // `e` here is &&Arc<AgentEntry>; deref through the ref + Arc to
            // hand `enrich_agent_json` the `&AgentEntry` it expects.
            .map(|e| super::agents::enrich_agent_json(e.as_ref(), &dm, catalog, None))
            .collect()
    };

    // Skills count — cached behind a 30s TTL to avoid scanning the skills
    // directory on every poll cycle.
    static SKILL_COUNT_CACHE: std::sync::Mutex<Option<(usize, std::time::Instant)>> =
        std::sync::Mutex::new(None);
    let skill_count = {
        let cached = SKILL_COUNT_CACHE
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(|(n, t)| {
                if t.elapsed() < std::time::Duration::from_secs(30) {
                    Some(*n)
                } else {
                    None
                }
            });
        match cached {
            Some(n) => n,
            None => {
                // Use the kernel's LIVE registry so `skills.disabled` and
                // `skills.extra_dirs` from config are honoured. The old
                // fresh-registry path showed disabled skills in the count
                // and missed extra_dirs entries.
                let n = state
                    .kernel
                    .skill_registry_ref()
                    .read()
                    .map(|r| r.list().len())
                    .unwrap_or(0);
                *SKILL_COUNT_CACHE.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some((n, std::time::Instant::now()));
                n
            }
        }
    };

    // Workflows, providers, channels — run concurrently with a 5s timeout on
    // providers/channels in case a local provider probe stalls.
    const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    let (workflow_result, providers_result, channels_result) = tokio::join!(
        state.kernel.workflow_engine().list_workflows(),
        tokio::time::timeout(PROBE_TIMEOUT, super::providers::providers_snapshot(state)),
        tokio::time::timeout(PROBE_TIMEOUT, super::channels::channels_snapshot(state)),
    );
    let workflow_count = workflow_result.len();
    let providers = providers_result.unwrap_or_default();
    let channels = channels_result.unwrap_or_default();

    let web_search_available = is_web_search_configured(&cfg.web);

    serde_json::json!({
        "health": health,
        "status": status,
        "agents": agents,
        "providers": providers,
        "channels": channels,
        "skillCount": skill_count,
        "workflowCount": workflow_count,
        "webSearchAvailable": web_search_available,
    })
}

#[cfg(test)]
mod config_key_path_validation_tests {
    // Duplicate of the inline `validate_config_key_path` logic so the tests
    // can exercise it without making it a public function.
    fn validate(p: &str) -> Result<(), String> {
        // Inline the same logic to avoid making the helper pub.
        if p.is_empty() {
            return Err("config path must not be empty".to_string());
        }
        if p.starts_with('/') || p.starts_with('\\') || p.contains("..") {
            return Err(format!(
                "config path '{p}' is not a valid key path (no filesystem separators allowed)"
            ));
        }
        for part in p.split('.') {
            if part.is_empty() {
                return Err(format!("config path '{p}' contains an empty segment"));
            }
            if !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!(
                    "config path segment '{part}' contains disallowed characters \
                     (only ASCII alphanumeric, '_', and '-' are permitted)"
                ));
            }
        }
        Ok(())
    }

    /// #3458 regression: valid key paths must pass validation.
    #[test]
    fn valid_paths_accepted() {
        assert!(validate("api_key").is_ok());
        assert!(validate("section.key").is_ok());
        assert!(validate("section.sub.key").is_ok());
        assert!(validate("llm.model_alias").is_ok());
        assert!(validate("queue.concurrency.trigger_lane").is_ok());
        assert!(validate("key-with-dash").is_ok());
    }

    /// #3458 regression: filesystem-like paths must be rejected.
    #[test]
    fn traversal_paths_rejected() {
        assert!(validate("").is_err(), "empty path");
        assert!(validate("../secret").is_err(), "traversal with ..");
        assert!(validate("a..b").is_err(), "double dot in segment");
        assert!(validate("/etc/passwd").is_err(), "absolute unix path");
        assert!(
            validate("\\Windows\\System32").is_err(),
            "absolute windows path"
        );
    }

    /// #3458 regression: special characters that could inject TOML structure
    /// must be rejected.
    #[test]
    fn special_chars_rejected() {
        assert!(validate("section[0]").is_err(), "bracket injection");
        assert!(validate("section = evil").is_err(), "equals sign");
        assert!(validate("section\nkey").is_err(), "newline");
        assert!(validate("section\0key").is_err(), "null byte");
        assert!(validate("section key").is_err(), "space");
    }

    /// Empty segment (double dot) must be rejected.
    #[test]
    fn empty_segment_rejected() {
        assert!(validate("a..b").is_err());
        assert!(validate(".a").is_err());
        assert!(validate("a.").is_err());
    }
}

#[cfg(test)]
mod web_search_configured_tests {
    use super::is_web_search_configured;
    use librefang_types::config::WebConfig;

    /// Point every API-key env-var lookup at a unique never-set name so the
    /// helper's only path to "configured" in these tests is via SearXNG. This
    /// keeps the assertions stable even on hosts that happen to export
    /// `TAVILY_API_KEY` / `BRAVE_API_KEY` / etc. for unrelated reasons.
    fn web_with_unset_keys(suffix: &str) -> WebConfig {
        let mut web = WebConfig::default();
        web.tavily.api_key_env = format!("LF_TEST_TAVILY_UNSET_{suffix}");
        web.brave.api_key_env = format!("LF_TEST_BRAVE_UNSET_{suffix}");
        web.jina.api_key_env = format!("LF_TEST_JINA_UNSET_{suffix}");
        web.perplexity.api_key_env = format!("LF_TEST_PERPLEXITY_UNSET_{suffix}");
        web.searxng.url = String::new();
        web
    }

    #[test]
    fn searxng_url_alone_counts_as_configured() {
        let mut web = web_with_unset_keys("searxng_alone");
        web.searxng.url = "https://search.example.com".to_string();
        assert!(
            is_web_search_configured(&web),
            "non-empty SearXNG URL must satisfy the configured check — it does not need an API key"
        );
    }

    #[test]
    fn empty_searxng_and_unset_keys_is_unconfigured() {
        let web = web_with_unset_keys("all_empty");
        assert!(
            !is_web_search_configured(&web),
            "no SearXNG URL and no API keys must report unconfigured"
        );
    }

    #[test]
    fn whitespace_only_searxng_url_does_not_count() {
        let mut web = web_with_unset_keys("whitespace");
        web.searxng.url = "   ".to_string();
        assert!(
            !is_web_search_configured(&web),
            "whitespace-only SearXNG URL must not satisfy the configured check"
        );
    }
}

#[cfg(test)]
mod redacted_web_tests {
    use super::redacted_web;
    use librefang_types::config::WebConfig;

    #[test]
    fn redacted_web_includes_searxng_url_round_trip() {
        let mut web = WebConfig::default();
        web.searxng.url = "https://search.example.com".to_string();
        let v = redacted_web(&web);
        let searxng = v
            .get("searxng")
            .expect("redacted_web must include `searxng` (issue #4016)");
        assert_eq!(
            searxng.get("url").and_then(|u| u.as_str()),
            Some("https://search.example.com"),
            "searxng.url written by the dashboard must round-trip through GET /api/config"
        );
    }

    #[test]
    fn redacted_web_includes_jina_subtable() {
        let mut web = WebConfig::default();
        web.jina.api_key_env = "MY_JINA_KEY".to_string();
        web.jina.use_eu_endpoint = true;
        let v = redacted_web(&web);
        let jina = v
            .get("jina")
            .expect("redacted_web must include `jina` (issue #4016)");
        assert_eq!(
            jina.get("api_key_env").and_then(|u| u.as_str()),
            Some("MY_JINA_KEY"),
        );
        assert_eq!(
            jina.get("use_eu_endpoint").and_then(|u| u.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn redacted_web_lists_all_provider_subtables() {
        let v = redacted_web(&WebConfig::default());
        // `duck_duck_go` and `auto` are stateless — no fields to surface.
        for key in &["brave", "tavily", "perplexity", "jina", "searxng", "fetch"] {
            assert!(
                v.get(key).is_some(),
                "redacted_web is missing the `{key}` sub-table; adding a new SearchProvider without surfacing its config here silently breaks the dashboard save flow (see #4016)",
            );
        }
    }
}

#[cfg(test)]
mod searxng_config_parse_tests {
    use librefang_types::config::KernelConfig;

    #[test]
    fn issue_4016_minimal_searxng_section_parses() {
        let toml_src = r#"[web.searxng]
url = "https://search.example.com"
"#;
        let cfg: KernelConfig = toml::from_str(toml_src)
            .expect("config with bare `[web.searxng]` table must parse (issue #4016)");
        assert_eq!(cfg.web.searxng.url, "https://search.example.com");
    }

    #[test]
    fn issue_4016_local_searxng_url_parses() {
        let toml_src = r#"[web.searxng]
url = "http://192.168.10.21:8888"
"#;
        let cfg: KernelConfig =
            toml::from_str(toml_src).expect("local SearXNG URL must parse (issue #4016)");
        assert_eq!(cfg.web.searxng.url, "http://192.168.10.21:8888");
    }

    #[test]
    fn issue_4016_searxng_alongside_init_template_layout_parses() {
        let toml_src = r#"
log_level = "info"
api_listen = "127.0.0.1:4545"

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"

[web]
search_provider = "auto"

[web.fetch]
max_chars = 50000
timeout_secs = 30

[web.searxng]
url = "https://search.example.com"
"#;
        let cfg: KernelConfig = toml::from_str(toml_src)
            .expect("init-template layout + appended [web.searxng] must parse (issue #4016)");
        assert_eq!(cfg.web.searxng.url, "https://search.example.com");
    }

    #[test]
    fn issue_3458_writable_path_allowlist() {
        // User-tunable scalars are accepted.
        assert!(super::is_writable_config_path("ui.theme"));
        assert!(super::is_writable_config_path("ui.locale"));
        assert!(super::is_writable_config_path("max_history_messages"));
        assert!(super::is_writable_config_path("log_level"));
        assert!(super::is_writable_config_path("approval.auto_approve"));
        assert!(super::is_writable_config_path(
            "approval.totp_grace_period_secs"
        ));

        // Sectioned tunables — single leaf and one nested level both allowed.
        assert!(super::is_writable_config_path("web.search_provider"));
        assert!(super::is_writable_config_path("rate_limit.max_ws_per_ip"));
        assert!(super::is_writable_config_path("channels.telegram.enabled"));

        // Account / credential paths MUST be rejected.
        assert!(!super::is_writable_config_path("default_model.api_key"));
        assert!(!super::is_writable_config_path("api_key"));
        assert!(!super::is_writable_config_path("users.alice.role"));
        assert!(!super::is_writable_config_path("auth.bypass"));
        assert!(!super::is_writable_config_path("approval.second_factor"));

        // Secret-suffix scrub catches accidentally-exposed leaves inside
        // an otherwise-allowed section.
        assert!(!super::is_writable_config_path("channels.telegram.token"));
        assert!(!super::is_writable_config_path("web.searxng.api_key"));
        assert!(!super::is_writable_config_path("queue.shared_secret"));

        // Unknown sections fall through to deny by default.
        assert!(!super::is_writable_config_path("network.shared_secret"));
        assert!(!super::is_writable_config_path("migration_state"));
        assert!(!super::is_writable_config_path("nonsense.key"));

        // ── Round-4 review of #4678 ──────────────────────────────────
        // Sections that are intentionally NOT in SECTION_PREFIXES
        // because their fields control auth redirect / observability
        // export / outbound traffic interception. Owner-role still
        // edits these on disk; the API write path stays closed.
        assert!(!super::is_writable_config_path("external_auth.issuer_url"));
        assert!(!super::is_writable_config_path(
            "external_auth.allowed_domains"
        ));
        assert!(!super::is_writable_config_path(
            "external_auth.redirect_url"
        ));
        assert!(!super::is_writable_config_path(
            "external_auth.require_email_verified"
        ));
        assert!(!super::is_writable_config_path("oauth.google_client_id"));
        assert!(!super::is_writable_config_path("audit.anchor_path"));
        assert!(!super::is_writable_config_path("audit.retention_days"));
        assert!(!super::is_writable_config_path("telemetry.otlp_endpoint"));
        assert!(!super::is_writable_config_path("telemetry.sample_rate"));
        assert!(!super::is_writable_config_path("proxy.http_proxy"));
        assert!(!super::is_writable_config_path("proxy.https_proxy"));

        // ── _env / client_id / client_secret SCRUB ────────────────────
        // The original SCRUB only blocked `.api_key` etc. literally;
        // the codebase pervasively names env-var-name fields with the
        // `_env` suffix (bot_token_env, client_secret_env, …). All of
        // those now reject regardless of which section they're in.
        assert!(!super::is_writable_config_path(
            "channels.telegram.bot_token_env"
        ));
        assert!(!super::is_writable_config_path("default_model.api_key_env"));
        assert!(!super::is_writable_config_path(
            "channels.whatsapp.access_token_env"
        ));
        assert!(!super::is_writable_config_path("default_model.client_id"));
        assert!(!super::is_writable_config_path(
            "default_model.client_secret"
        ));

        // ── Collection paths: primitive maps allowed, Vec<Struct> rejected ──
        // BTreeMap<String, String|u64> sections accept whole-blob writes
        // because their value type is primitive — no nested credential
        // surface. Vec<Struct> sections (sidecar_channels,
        // fallback_providers, taint_rules) reject whole-blob writes:
        // their items have nested fields (env maps, api_key_env) that
        // SCRUB can't police inside a wholesale JSON payload.
        // `sidecar_channels` has its own typed write endpoint
        // (`POST /api/channels/sidecar/{name}/configure`); the bare
        // path stays closed here.
        assert!(super::is_writable_config_path("provider_urls"));
        assert!(super::is_writable_config_path("provider_regions"));
        assert!(super::is_writable_config_path(
            "provider_request_timeout_secs"
        ));
        assert!(super::is_writable_config_path("tool_timeouts"));
        assert!(!super::is_writable_config_path("sidecar_channels"));
        assert!(!super::is_writable_config_path("fallback_providers"));
        assert!(!super::is_writable_config_path("taint_rules"));

        // ── Round-5 review of #4678 ──────────────────────────────────
        // `channels.<vendor>` (depth-1 wholesale-replace) MUST reject;
        // depth-2 leaves under the same vendor stay open (per-field
        // toggles via the dashboard).
        assert!(!super::is_writable_config_path("channels.telegram"));
        assert!(!super::is_writable_config_path("channels.whatsapp"));
        assert!(!super::is_writable_config_path("channels.email"));
        assert!(super::is_writable_config_path("channels.telegram.enabled"));
        assert!(super::is_writable_config_path("channels.whatsapp.enabled"));

        // `network.bootstrap_peers` MUST reject (DHT MITM via post-auth
        // peer redirect, threat model parallel to the round-4 removal
        // of `proxy.http_proxy`). Display knobs stay open via EXACT.
        assert!(!super::is_writable_config_path("network.bootstrap_peers"));
        assert!(!super::is_writable_config_path("network.listen_addresses"));
        assert!(super::is_writable_config_path("network.mdns_enabled"));
        assert!(super::is_writable_config_path("network.max_peers"));
        assert!(super::is_writable_config_path(
            "network.max_messages_per_peer_per_minute"
        ));
    }

    #[test]
    fn issue_4016_inline_table_form_from_dashboard_save_parses() {
        let toml_src = r#"
[web]
search_provider = "auto"
searxng = { url = "https://search.example.com" }
"#;
        let cfg: KernelConfig = toml::from_str(toml_src)
            .expect("inline-table shape produced by /api/config/set must parse (issue #4016)");
        assert_eq!(cfg.web.searxng.url, "https://search.example.com");
    }
}

#[cfg(test)]
mod migrate_roots_tests {
    use super::migrate_source_roots;
    use std::path::Path;

    #[test]
    fn includes_only_existing_known_source_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let os_home = tmp.path();
        std::fs::create_dir_all(os_home.join(".openclaw")).unwrap();
        std::fs::create_dir_all(os_home.join(".langchain")).unwrap();
        let lf_home = os_home.join(".librefang");
        std::fs::create_dir_all(&lf_home).unwrap();

        let roots = migrate_source_roots(&lf_home, Some(os_home));

        assert!(roots.contains(&lf_home), "librefang home is always a root");
        assert!(
            roots.contains(&os_home.join(".openclaw")),
            "existing source dir must be included"
        );
        assert!(
            roots.contains(&os_home.join(".langchain")),
            "existing source dir must be included"
        );
        // A non-existent root must NOT be added: validate_path_containment
        // returns a 500 on a root it cannot canonicalize.
        assert!(!roots.contains(&os_home.join(".autogpt")));
        assert!(!roots.contains(&os_home.join(".openfang")));
    }

    #[test]
    fn no_os_home_yields_librefang_home_only() {
        let tmp = tempfile::tempdir().unwrap();
        let lf_home = tmp.path().join(".librefang");
        std::fs::create_dir_all(&lf_home).unwrap();
        assert_eq!(migrate_source_roots(&lf_home, None), vec![lf_home]);
    }

    #[test]
    fn source_under_known_root_passes_but_target_stays_confined() {
        // Reproduces the #5577 regression: a source under `~/.openclaw` must be
        // accepted again, while writes (target) stay confined to the librefang
        // home.
        let tmp = tempfile::tempdir().unwrap();
        let os_home = tmp.path();
        let openclaw = os_home.join(".openclaw");
        std::fs::create_dir_all(&openclaw).unwrap();
        let lf_home = os_home.join(".librefang");
        std::fs::create_dir_all(&lf_home).unwrap();

        let source_roots = migrate_source_roots(&lf_home, Some(os_home));
        let source_allowed: Vec<&Path> = source_roots.iter().map(|p| p.as_path()).collect();

        // source_dir under ~/.openclaw is accepted (the regression case).
        assert!(crate::validation::validate_path_containment(
            "source_dir",
            &openclaw,
            &source_allowed,
            true,
        )
        .is_ok());

        // A sibling dir NOT on the allow-list is still rejected (containment held).
        let outside = os_home.join(".evil");
        std::fs::create_dir_all(&outside).unwrap();
        assert!(crate::validation::validate_path_containment(
            "source_dir",
            &outside,
            &source_allowed,
            true,
        )
        .is_err());

        // Target writes stay confined to the librefang home: `~/.openclaw` is a
        // valid *source* root but must NOT be a valid *target* root.
        let target_allowed: Vec<&Path> = vec![lf_home.as_path()];
        assert!(crate::validation::validate_path_containment(
            "target_dir",
            &openclaw,
            &target_allowed,
            false,
        )
        .is_err());
    }
}
