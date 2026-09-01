//! Integration tests for the config-domain HTTP routes registered via
//! `routes::config::router()` (see `crates/librefang-api/src/routes/config.rs`).
//!
//! Coverage per #3571 — config slice only:
//!   - GET  /api/config            (happy path + auth gate)
//!   - GET  /api/config/schema     (happy path; public, no auth gate)
//!   - GET  /api/config/export     (happy path with on-disk file + fallback to in-memory)
//!   - POST /api/config/set        (allowlisted round-trip; rejects empty path,
//!     traversal, non-allowlisted key, missing fields)
//!   - POST /api/config/reload     (no-op reload returns 200 with status field)
//!
//! Out of scope (intentionally skipped):
//!   - POST /api/migrate, /api/migrate/scan, GET /api/migrate/detect — touches
//!     real on-disk migration state outside the tempdir.
//!   - POST /api/shutdown / /api/init — would tear down the harness kernel.
//!   - GET  /api/health, /api/version, /api/status — covered elsewhere or trivial.
//!
//! All tests use a tempdir-backed kernel (config.home_dir = tempdir) so any
//! write-through to `config.toml` lands in the test sandbox, never the real
//! `~/.librefang/config.toml`.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, SessionId,
};
use librefang_types::config::{DefaultModelConfig, ExternalAuthConfig, KernelConfig, OidcProvider};
use librefang_types::message::TokenUsage;
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

struct RouterHarness {
    app: axum::Router,
    home: std::path::PathBuf,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot_router_with_api_key(api_key: &str) -> RouterHarness {
    boot_router_with_config(api_key, |_| {}).await
}

/// Same harness, with a hook to set config fields the default does not exercise.
/// Used by the `external_auth` read test, which has to distinguish "the value came from config" from "the value is the default that happens to match".
async fn boot_router_with_config(
    api_key: &str,
    customize: impl FnOnce(&mut KernelConfig),
) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let mut config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };
    customize(&mut config);

    let home = config.home_dir.clone();
    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    RouterHarness {
        app,
        home,
        _tmp: tmp,
        state,
    }
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

fn auth_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap()
}

fn anon_get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

fn register_metrics_agent(
    state: &librefang_api::routes::AppState,
    name: &str,
    provider: &str,
    model: &str,
) {
    let id = AgentId::new();
    let mut manifest = AgentManifest {
        name: name.to_string(),
        source_template: None,
        description: "metrics escaping test".to_string(),
        author: "test".to_string(),
        module: "builtin:chat".to_string(),
        ..Default::default()
    };
    manifest.model.provider = provider.to_string();
    manifest.model.model = model.to_string();
    let resources = manifest.resources.clone();
    let entry = AgentEntry {
        id,
        name: name.to_string(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        session_id: SessionId::new(),
        ..Default::default()
    };

    state.kernel.agent_registry().register(entry).unwrap();
    state.kernel.scheduler_ref().register(id, resources);
    state.kernel.scheduler_ref().record_usage(
        id,
        &TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            ..Default::default()
        },
    );
}

fn auth_post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// GET /api/config
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_config_returns_redacted_view() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    // Spot-check some fields the redacted view always includes.
    assert!(json.is_object(), "expected object, got {json}");
    for key in ["channels", "mcp_servers", "fallback_providers"] {
        assert!(
            json.get(key).is_some(),
            "missing redacted field '{key}' in /api/config response: {json}"
        );
    }
}

/// Walk a dotted path through a JSON body.
/// A `null` at the end still counts as found — an unset `Option` renders as `null` and the dashboard needs the key to exist in order to bind an empty input to it.
fn dig<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cursor = value;
    for segment in path.split('.') {
        cursor = cursor.as_object()?.get(segment)?;
    }
    Some(cursor)
}

/// #6596: `GET /api/config` used to omit fields that `POST /api/config/set` accepts, so the dashboard rendered them blank and reported them as "not configured" even when `config.toml` set them.
/// The unit-level guard in `routes::config::manage` enumerates the full set; this test pins the paths from the report plus one whole section (`terminal`) that was absent entirely, over the real router.
#[tokio::test(flavor = "multi_thread")]
async fn get_config_exposes_writable_fields_that_used_to_be_write_only() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");

    for path in [
        // Reported by the issue.
        "browser.cdp_endpoint",
        "browser.enabled",
        "media.image_model",
        "media.custom_stt",
        "media.transcription_timeout_secs",
        "media.ffmpeg_timeout_secs",
        "tts.custom",
        "channels.file_download_dir",
        // Found alongside them while auditing every writable section.
        "browser.cdp_auth_token_env",
        "approval.totp_grace_period_secs",
        "web.timeout_secs",
        "web.fetch.ssrf_allowed_hosts",
        "exec_policy.allowed_env_vars",
        "exec_policy.safe_bins_skip_approval",
        "exec_policy.full_mode_skips_approval",
        "session.reset",
        "session.context_injection",
        "memory.decay",
        "memory.chunking",
        "memory.pool_size",
        "proactive_memory.format_context_max_chars",
        "queue.task_queue_retention_days",
        "queue.concurrency.trigger_fire_timeout_secs",
        "compaction.strip_reasoning_after_turns",
        "registry.registry_host",
        "vault.use_os_keyring",
        "pairing.public_base_url",
        "default_model.message_timeout_secs",
        "budget.default_burst_ratio",
        "network.max_messages_per_peer_per_minute",
        "tts.elevenlabs.output_format",
    ] {
        assert!(
            dig(&json, path).is_some(),
            "GET /api/config must expose writable path '{path}' (#6596); body: {json}"
        );
    }

    // `terminal` was missing as a whole section even though `ui_sections_overlay` declares it and `terminal.` is a writable section prefix.
    let terminal = json
        .get("terminal")
        .expect("GET /api/config must include the `terminal` section (#6596)");
    for key in [
        "enabled",
        "allowed_origins",
        "allow_remote",
        "require_proxy_headers",
        "allow_unauthenticated_remote",
        "tmux_enabled",
        "max_windows",
        "tmux_binary_path",
    ] {
        assert!(
            terminal.get(key).is_some(),
            "`terminal.{key}` missing from GET /api/config: {terminal}"
        );
    }
}

/// #6605: `external_auth` fields that are deliberately non-writable were also unreadable, which is the wrong asymmetry.
/// `require_email_verified` is the #3703 mitigation — it rejects logins whose ID token lacks `email_verified = true`, which is what stops an unverified address in an `allowed_domains` domain from inheriting that domain's authorization.
/// Keeping it out of the write allowlist is correct (an Owner-role caller with a leaked API key must not be able to turn it off); hiding it from `GET /api/config` left an operator no way to confirm the protection is on without shell access to read `config.toml`.
/// The `OidcProvider` endpoint overrides had the same problem.
///
/// Every asserted value is one the harness set explicitly, and `require_email_verified` is set to the non-default `false` on purpose — asserting its default `true` would also pass against a hardcoded literal in the response builder, which proves nothing about where the value came from.
#[tokio::test(flavor = "multi_thread")]
async fn get_config_exposes_non_writable_external_auth_fields() {
    let h = boot_router_with_config(API_KEY, |config| {
        config.external_auth = ExternalAuthConfig {
            // Non-default: proves the response reads the configured value rather than emitting a constant.
            require_email_verified: false,
            providers: vec![OidcProvider {
                id: "corp".to_string(),
                display_name: "Corporate SSO".to_string(),
                issuer_url: "https://issuer.example.invalid".to_string(),
                auth_url: "https://issuer.example.invalid/authorize".to_string(),
                token_url: "https://issuer.example.invalid/token".to_string(),
                userinfo_url: "https://issuer.example.invalid/userinfo".to_string(),
                jwks_uri: "https://issuer.example.invalid/jwks".to_string(),
                client_id: "corp-client".to_string(),
                client_secret_env: "LIBREFANG_CONFIG_ROUTES_TEST_DOES_NOT_EXIST".to_string(),
                redirect_url: "http://127.0.0.1:4545/api/auth/callback".to_string(),
                scopes: vec!["openid".to_string()],
                allowed_domains: vec!["example.invalid".to_string()],
                audience: "corp-audience".to_string(),
                // `Some(false)` rather than `None`: an explicit per-provider override must stay distinguishable from "inherit the global setting", which renders as `null`.
                require_email_verified: Some(false),
            }],
            ..Default::default()
        };
    })
    .await;

    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");

    assert_eq!(
        dig(&json, "external_auth.require_email_verified").and_then(|v| v.as_bool()),
        Some(false),
        "GET /api/config must report the configured value of the #3703 email-verification gate \
         (#6605); body: {json}"
    );

    let Some(provider) = dig(&json, "external_auth.providers")
        .and_then(|v| v.as_array())
        .and_then(|providers| providers.first())
    else {
        panic!("configured OIDC provider missing from GET /api/config: {json}");
    };

    for (key, expected) in [
        ("auth_url", "https://issuer.example.invalid/authorize"),
        ("token_url", "https://issuer.example.invalid/token"),
        ("userinfo_url", "https://issuer.example.invalid/userinfo"),
        ("jwks_uri", "https://issuer.example.invalid/jwks"),
        ("audience", "corp-audience"),
    ] {
        assert_eq!(
            provider.get(key).and_then(|v| v.as_str()),
            Some(expected),
            "`external_auth.providers[].{key}` must round-trip its configured value (#6605); \
             provider: {provider}"
        );
    }
    assert_eq!(
        provider
            .get("require_email_verified")
            .and_then(|v| v.as_bool()),
        Some(false),
        "the per-provider `require_email_verified` override must be readable and must not collapse \
         an explicit `false` into `null` (#6605); provider: {provider}"
    );

    // The read side is additive only — these paths stay closed to `POST /api/config/set`.
    for path in [
        "external_auth.require_email_verified",
        "external_auth.audience",
    ] {
        let (status, _) = send(
            h.app.clone(),
            auth_post_json(
                "/api/config/set",
                serde_json::json!({"path": path, "value": true}),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "`{path}` became readable in #6605 but MUST stay non-writable — flipping it post-auth \
             is the #3703 regression vector"
        );
    }
}

/// #6636 observation (e): the approval section's non-writable fields must still read back.
///
/// The `approval` section declares no explicit `fields` list, so `ConfigPage` renders a control for every `ApprovalPolicy` field the derived schema knows about, and seven of the fourteen were absent from the response builder.
/// `cache_approvals_per_session` is the one an operator noticed: it defaults to `true`, so the dashboard showed it off even when `config.toml` said `true`, and there was no way to tell whether per-session approval caching was actually on.
///
/// Every asserted value is set to the non-default here, so a response builder emitting a hardcoded literal cannot pass.
/// The read side is additive: none of these becomes writable, which is the point — an Owner-role caller with a leaked API key must not be able to relax approval policy over HTTP.
#[tokio::test(flavor = "multi_thread")]
async fn get_config_exposes_non_writable_approval_fields() {
    let h = boot_router_with_config(API_KEY, |config| {
        config.approval.cache_approvals_per_session = false;
        config.approval.audit_retention_days = 7;
        config.approval.trusted_senders = vec!["operator-1".to_string()];
        config.approval.totp_tools = vec!["shell_exec".to_string()];
    })
    .await;

    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");

    assert_eq!(
        dig(&json, "approval.cache_approvals_per_session").and_then(|v| v.as_bool()),
        Some(false),
        "GET /api/config must report the configured value of `cache_approvals_per_session` \
         (#6636); body: {json}"
    );
    assert_eq!(
        dig(&json, "approval.audit_retention_days").and_then(|v| v.as_u64()),
        Some(7),
        "`approval.audit_retention_days` must round-trip; body: {json}"
    );
    assert_eq!(
        dig(&json, "approval.trusted_senders").and_then(|v| v.as_array()),
        Some(&vec![serde_json::json!("operator-1")]),
        "`approval.trusted_senders` must round-trip; body: {json}"
    );
    assert_eq!(
        dig(&json, "approval.totp_tools").and_then(|v| v.as_array()),
        Some(&vec![serde_json::json!("shell_exec")]),
        "`approval.totp_tools` must round-trip; body: {json}"
    );
    // Present even when empty / at their default, so the dashboard can bind an input to the key.
    for path in ["approval.channel_rules", "approval.routing"] {
        assert!(
            dig(&json, path).is_some_and(|v| v.is_array()),
            "`{path}` must be present as an array; body: {json}"
        );
    }
    assert_eq!(
        dig(&json, "approval.timeout_fallback").and_then(|v| v.as_str()),
        Some("deny"),
        "`approval.timeout_fallback` must use the serde encoding, not `Debug`; body: {json}"
    );

    // Newly readable, still closed to writes.
    for path in [
        "approval.cache_approvals_per_session",
        "approval.trusted_senders",
        "approval.audit_retention_days",
    ] {
        let (status, _) = send(
            h.app.clone(),
            auth_post_json(
                "/api/config/set",
                serde_json::json!({"path": path, "value": true}),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "`{path}` became readable in #6636 but MUST stay non-writable — approval policy is not \
             adjustable over HTTP"
        );
    }
}

/// `[registry] auto_sync` must round-trip through both halves of the config surface.
///
/// It is the operator's only durable way to stop the daemon fast-forwarding `~/.librefang/registry/` with `git reset --hard origin/main`, which destroys local modifications under that checkout — including the ones `PUT /api/hands/{id}/manifest` writes for a hand that shipped with the registry (#6636 observation (a)).
/// A knob that reads back as its default would leave an operator unable to confirm the freeze took, which is the same class of gap #6605 and #6618 closed elsewhere in this file.
#[tokio::test(flavor = "multi_thread")]
async fn registry_auto_sync_round_trips_through_get_and_set() {
    let h = boot_router_with_config(API_KEY, |config| {
        // Non-default, so a hardcoded literal in the response builder cannot pass.
        config.registry.auto_sync = false;
    })
    .await;

    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(
        dig(&json, "registry.auto_sync").and_then(|v| v.as_bool()),
        Some(false),
        "GET /api/config must report the configured value; body: {json}"
    );

    // Writable too — `registry.` is a writable section prefix, and an operator who
    // froze the registry from the dashboard has to be able to unfreeze it there.
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "registry.auto_sync", "value": true}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "registry.auto_sync must be writable"
    );

    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(
        dig(&json, "registry.auto_sync").and_then(|v| v.as_bool()),
        Some(true),
        "the write must be readable back; body: {json}"
    );
}

/// #6596: enum-valued config fields were rendered with `format!("{:?}", …)`, which emits the Rust variant name (`"Allowlist"`, `"DuckDuckGo"`).
/// The write path, `config.toml`, and the schema's `select` options all use serde's rename form, so the dashboard dropdown matched none of its own options.
/// Asserting the absence of upper-case characters catches any new field that reintroduces `Debug` formatting, not just the seven that were fixed.
#[tokio::test(flavor = "multi_thread")]
async fn get_config_enum_values_use_serde_encoding() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");

    for path in [
        "mode",
        "reload.mode",
        "exec_policy.mode",
        "broadcast.strategy",
        "docker.mode",
        "docker.scope",
        "web.search_provider",
        "privacy.mode",
        "sanitize.mode",
    ] {
        let value = dig(&json, path)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("'{path}' missing or not a string in {json}"));
        assert!(
            !value.chars().any(|c| c.is_ascii_uppercase()),
            "'{path}' = \"{value}\" looks like Debug output; emit the serde encoding the \
             write path and the schema's select options use (#6596)"
        );
    }
}

/// #6596 end to end: write a field that used to be invisible on the read side, then read it back.
/// Before the fix the write succeeded and the subsequent GET still showed nothing, which is what made the dashboard report a saved setting as unconfigured.
///
/// `compaction.*` is the field family used here because `build_reload_plan` classifies it as a read-live no-op change, so its own diff satisfies `should_store_config` and the live config swap is guaranteed by this edit alone.
/// A `browser.*` write is classified restart-required and would only reach the live snapshot on the back of some unrelated diff, which would make the assertion prove the wrong thing.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_then_get_round_trips_a_previously_write_only_field() {
    let h = boot_router_with_api_key(API_KEY).await;

    // Seed the on-disk config with the harness's api_key. `config_set` reloads from this file afterwards, so without the seed the reload would replace the live config with one whose `api_key` is empty and the follow-up authenticated GET would be answered under different auth rules than the one under test.
    let seed = format!("api_key = \"{API_KEY}\"\n");
    std::fs::write(h.home.join("config.toml"), seed).expect("seed config.toml");

    // Default is 0, so 4 is an observable change.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "compaction.strip_reasoning_after_turns", "value": 4}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "compaction.strip_reasoning_after_turns is allowlisted; got {status}: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(
        dig(&json, "compaction.strip_reasoning_after_turns").and_then(|v| v.as_u64()),
        Some(4),
        "a saved value must be visible in the next GET /api/config (#6596); body: {json}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_config_is_dashboard_read_when_no_api_key() {
    // With api_key empty, dashboard reads must work without a token.
    let h = boot_router_with_api_key("").await;
    let (status, _) = send(h.app.clone(), anon_get("/api/config")).await;
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "/api/config must be reachable without auth in no-key dev mode"
    );
}

// ---------------------------------------------------------------------------
// GET /api/config/schema
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_config_schema_is_public_and_returns_json_schema() {
    // Schema is in PUBLIC_ROUTES_ALWAYS, so anonymous GET must succeed even
    // when an api_key is configured.
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), anon_get("/api/config/schema")).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    // Schemars-generated draft-07 output, plus our two extension keys.
    assert!(
        json.get("x-sections").is_some(),
        "schema missing x-sections overlay"
    );
    assert!(
        json.get("x-ui-options").is_some(),
        "schema missing x-ui-options overlay"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_config_schema_starts_configured_openrouter_catalog_refresh() {
    use librefang_types::model_catalog::AuthStatus;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "id": "acme/schema:free",
                "name": "Schema Free",
                "context_length": 65536,
                "pricing": {"prompt": "0", "completion": "0"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let h = boot_router_with_api_key(API_KEY).await;
    h.state.kernel.model_catalog_update(&mut |catalog| {
        assert!(catalog.set_provider_url("openrouter", &server.uri()));
        catalog.set_provider_auth_status("openrouter", AuthStatus::Configured);
        catalog.clear_provider_available_models("openrouter");
    });

    let (status, _) = send(h.app.clone(), anon_get("/api/config/schema")).await;
    assert_eq!(status, StatusCode::OK);

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if h.state
                .kernel
                .model_catalog_ref()
                .load()
                .has_live_provider_models("openrouter")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("config schema should trigger OpenRouter refresh");
}

// ---------------------------------------------------------------------------
// GET /api/config/export
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_config_export_falls_back_to_in_memory_when_no_file() {
    // Tempdir has no config.toml — handler must serialize the in-memory config.
    let h = boot_router_with_api_key(API_KEY).await;
    assert!(!h.home.join("config.toml").exists());

    let (status, body) = send(h.app.clone(), auth_get("/api/config/export")).await;
    assert_eq!(status, StatusCode::OK);
    let toml_text = String::from_utf8(body).expect("toml is utf-8");
    // Must parse as TOML and include at least a top-level table marker.
    let _: toml::Value = toml::from_str(&toml_text).expect("export body is valid TOML");
    assert!(!toml_text.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_config_export_reads_disk_file_when_present() {
    let h = boot_router_with_api_key(API_KEY).await;
    let on_disk = "# sentinel-marker-3571\nlog_level = \"debug\"\n";
    std::fs::write(h.home.join("config.toml"), on_disk).expect("write config.toml");

    let (status, body) = send(h.app.clone(), auth_get("/api/config/export")).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    assert!(
        text.contains("sentinel-marker-3571"),
        "export should pass through the on-disk file verbatim, got: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_export_requires_auth_when_key_set() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(h.app.clone(), anon_get("/api/config/export")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// POST /api/config/set
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn config_set_writes_allowlisted_path_to_tempdir_toml() {
    let h = boot_router_with_api_key(API_KEY).await;
    // `log_level` is a real top-level KernelConfig field on the allowlist; it round-trips through the schema validator AND survives the post-write kernel reload (which re-serializes the in-memory config).
    // `ui.theme` used to be named here as the counter-example of an allowlisted path the kernel does not model; #6605 removed those four `ui.*` entries from the allowlist, so the write path no longer accepts any such path.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "log_level", "value": "debug"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 for allowlisted log_level write, got {status}: {}",
        String::from_utf8_lossy(&body)
    );
    let response: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let response_status = response["status"].as_str().expect("status string");

    // Verify the write landed in the tempdir's config.toml — NOT the user's
    // real ~/.librefang/config.toml. (kernel.home_dir is the tempdir.)
    let written = std::fs::read_to_string(h.home.join("config.toml")).expect("toml exists");
    let parsed: toml::Value = toml::from_str(&written).expect("valid toml");
    let log_level = parsed.get("log_level").and_then(|v| v.as_str());
    assert_eq!(log_level, Some("debug"), "wrote: {written}");

    // And the in-memory kernel config reflects it (post-reload).
    assert_eq!(h.state.kernel.config_ref().log_level, "debug");

    let audit = h
        .state
        .kernel
        .audit()
        .recent(20)
        .into_iter()
        .find(|entry| entry.detail == "config set: log_level")
        .expect("config set audit entry");
    assert_eq!(audit.outcome, response_status);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_non_allowlisted_path() {
    let h = boot_router_with_api_key(API_KEY).await;
    // `api_key` is excluded from the allowlist for security.
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "api_key", "value": "stolen"}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "api_key write must be 403, got {status}: {}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_path_traversal() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "../etc/passwd", "value": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_empty_path() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "", "value": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_missing_path_field() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json("/api/config/set", serde_json::json!({"value": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_missing_value_field() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        // A real allowlisted path, so the 400 can only come from the missing `value` field.
        // Was `ui.theme` until #6605 dropped it from the allowlist.
        auth_post_json("/api/config/set", serde_json::json!({"path": "log_level"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// POST /api/config/set — collection-typed sections (#4678)
//
// Round-trips for the BTreeMap<String, String|u64> sections that the
// dashboard's StringMapEditor / NumberMapEditor save as a whole-blob
// payload at the section's bare path. Vec<Struct> sections (sidecar_channels,
// fallback_providers, taint_rules) and tightened-out section prefixes
// (external_auth, oauth, audit, telemetry, proxy) must be rejected.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn config_set_writes_provider_urls_collection_to_toml() {
    let h = boot_router_with_api_key(API_KEY).await;
    let payload = serde_json::json!({
        "openai": "https://api.openai.com/v1",
        "ollama": "http://127.0.0.1:11434/v1",
    });
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "provider_urls", "value": payload.clone()}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 for whole-collection provider_urls write, got {status}: {}",
        String::from_utf8_lossy(&body)
    );

    let written = std::fs::read_to_string(h.home.join("config.toml")).expect("toml exists");
    let parsed: toml::Value = toml::from_str(&written).expect("valid toml");
    let urls = parsed
        .get("provider_urls")
        .and_then(|v| v.as_table())
        .expect("provider_urls table present");
    assert_eq!(
        urls.get("openai").and_then(|v| v.as_str()),
        Some("https://api.openai.com/v1"),
        "wrote: {written}"
    );
    assert_eq!(
        urls.get("ollama").and_then(|v| v.as_str()),
        Some("http://127.0.0.1:11434/v1"),
        "wrote: {written}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_writes_tool_timeouts_number_map_to_toml() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "tool_timeouts", "value": {"shell": 60, "fetch": 30}}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "tool_timeouts whole-collection write should round-trip; got {status}: {}",
        String::from_utf8_lossy(&body)
    );

    let written = std::fs::read_to_string(h.home.join("config.toml")).expect("toml exists");
    let parsed: toml::Value = toml::from_str(&written).expect("valid toml");
    let timeouts = parsed
        .get("tool_timeouts")
        .and_then(|v| v.as_table())
        .expect("tool_timeouts table");
    assert_eq!(timeouts.get("shell").and_then(|v| v.as_integer()), Some(60));
    assert_eq!(timeouts.get("fetch").and_then(|v| v.as_integer()), Some(30));
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_sidecar_channels_whole_blob_write() {
    // Vec<Struct> sections cannot be whole-blob-written — their items
    // contain nested env maps that the path-string SCRUB cannot police
    // when they arrive as a JSON payload at the section's bare path.
    let h = boot_router_with_api_key(API_KEY).await;
    let evil_payload = serde_json::json!([
        {
            "name": "evil",
            "command": "/bin/cat",
            "channel_type": "telegram",
            "env": {"AWS_SECRET_ACCESS_KEY": "stolen"}
        }
    ]);
    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "sidecar_channels", "value": evil_payload}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "sidecar_channels whole-blob write must 403; got {status}: {}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_fallback_providers_whole_blob_write() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "fallback_providers",
                "value": [{"provider": "openai", "model": "gpt-4o", "api_key_env": "STOLEN"}]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_external_auth_issuer_url() {
    // external_auth.* is intentionally NOT in SECTION_PREFIXES — flipping
    // issuer_url post-auth would let an Owner-role attacker redirect login
    // to an attacker IDP (regression vector for #3703).
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "external_auth.issuer_url",
                "value": "https://attacker.example/"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_audit_anchor_path() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "audit.anchor_path", "value": "/tmp/evil-anchor"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_telemetry_otlp_endpoint() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "telemetry.otlp_endpoint",
                "value": "https://attacker.example:4317"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_proxy_http_proxy() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "proxy.http_proxy", "value": "http://attacker:8080"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_rejects_env_suffix_redirect_inside_writable_section() {
    // SCRUB_SUFFIXES extension catches `<anything>_env` so an attacker
    // can't repoint `default_model.api_key_env` (or any *.token_env /
    // *.client_secret_env / *.password_env) at an arbitrary daemon env var.
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "default_model.api_key_env", "value": "HOME"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_set_requires_auth_when_key_set() {
    let h = boot_router_with_api_key(API_KEY).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/set")
        .header(header::CONTENT_TYPE, "application/json")
        // Any well-formed body will do — the 401 comes from the auth layer before the handler runs.
        // Was `ui.theme` until #6605 dropped it from the allowlist.
        .body(Body::from(
            serde_json::json!({"path": "log_level", "value": "debug"}).to_string(),
        ))
        .unwrap();
    let (status, _) = send(h.app.clone(), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// POST /api/config/reload
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn config_reload_returns_no_changes_when_disk_matches_memory() {
    let h = boot_router_with_api_key(API_KEY).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(h.app.clone(), req).await;
    // Reload may return 200 (no changes / applied) or 400 (no on-disk file
    // depending on kernel impl). Either way the body must be JSON with a
    // `status` field — the route must be wired and not 404 / 500-stack-trace.
    assert!(
        status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
        "unexpected status {status}: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).expect("reload body is JSON");
    assert!(
        json.get("status").is_some(),
        "missing 'status' field: {json}"
    );
    let expected_audit_outcome = if status == StatusCode::OK {
        json["status"].as_str().expect("reload status")
    } else {
        "failed"
    };
    let audit = h
        .state
        .kernel
        .audit()
        .recent(20)
        .into_iter()
        .find(|entry| entry.detail == "config reload requested via API")
        .expect("config reload audit entry");
    assert_eq!(audit.outcome, expected_audit_outcome);
    assert_ne!(audit.outcome, "pending");
}

#[tokio::test(flavor = "multi_thread")]
async fn config_reload_requires_auth_when_key_set() {
    let h = boot_router_with_api_key(API_KEY).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(h.app.clone(), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Regression for #4664: a syntactically-broken `config.toml` (the bug report
/// hit a duplicate `[web.searxng]` key) used to silently reset the live config
/// to defaults on the next hot-reload tick because `crate::config::load_config`
/// is tolerant. From the operator's POV the dashboard "stopped loading" because
/// `default_model`, `provider_api_keys`, channels, etc. all reverted to
/// defaults. The fix makes `reload_config` strict-parse the file first and
/// surface the error so the live config stays intact.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_with_invalid_toml_returns_error_and_preserves_live_config() {
    let h = boot_router_with_api_key(API_KEY).await;

    // Capture the live `default_model` BEFORE the bad reload so we can prove
    // it survived. The harness boots with `model = "test-model"`.
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let before: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let before_model = before
        .get("default_model")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    assert_eq!(
        before_model, "test-model",
        "harness must seed default_model.model = test-model"
    );

    // Write a config.toml with a TOML duplicate-key error. This mirrors the
    // exact failure shape from the user's report: two `[web.searxng]` sections.
    let bad_toml =
        "[web.searxng]\nurl = \"http://first\"\n\n[web.searxng]\nurl = \"http://second\"\n";
    std::fs::write(h.home.join("config.toml"), bad_toml).expect("write bad config.toml");

    // Reload must report a parse error (400) — NOT silently apply defaults (200).
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(h.app.clone(), req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bad TOML must produce a 400 with an explicit error, not a 200 + silent defaults; body={}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let err_str = json
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("")
        .to_string();
    assert!(
        err_str.contains("invalid TOML") && err_str.contains("live config unchanged"),
        "error must be operator-actionable; got: {err_str}"
    );
    let audit = h
        .state
        .kernel
        .audit()
        .recent(20)
        .into_iter()
        .find(|entry| entry.detail == "config reload requested via API")
        .expect("failed config reload audit entry");
    assert_eq!(audit.outcome, "failed");

    // Live config must be unchanged — the failed reload did not blow away
    // `default_model.model` (which is the symptom that broke the dashboard).
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let after: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let after_model = after
        .get("default_model")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    assert_eq!(
        after_model, before_model,
        "live default_model.model must be preserved after a failed reload"
    );
}

/// Internal helper: drop a `config.toml` into the harness's home dir,
/// POST `/api/config/reload`, and assert that it returns 400 *and* that
/// `GET /api/config` still reports the seeded `default_model.model`.
///
/// Used by the next two regressions to cover the two non-syntax failure
/// modes that `try_load_config` (introduced in #4664) refuses: a
/// deserialize-shape mismatch and a broken `include = [...]` chain. The
/// duplicate-key TOML-syntax case has its own dedicated test above
/// (preserved with its own assertion text so a regression on the syntax
/// path stays distinguishable in test output).
async fn assert_reload_rejects_and_preserves_default_model(
    h: &RouterHarness,
    bad_toml_filename: &str,
    bad_toml_contents: &str,
    extra_files: &[(&str, &str)],
    failure_label: &str,
) {
    // Capture pre-reload `default_model.model`.
    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let before: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let before_model = before
        .get("default_model")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    assert_eq!(before_model, "test-model");

    for (name, contents) in extra_files {
        std::fs::write(h.home.join(name), contents)
            .unwrap_or_else(|e| panic!("write helper file {name}: {e}"));
    }
    std::fs::write(h.home.join(bad_toml_filename), bad_toml_contents)
        .expect("write bad config.toml");

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(h.app.clone(), req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{failure_label} must produce a 400, not silent defaults; body={}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let err_str = json
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("")
        .to_string();
    // Every reload-time rejection MUST go through the strict loader's
    // `try_load_config` and be wrapped with the "live config unchanged"
    // pledge, so future failure modes can be asserted with the same
    // substring without needing to know which inner branch tripped.
    assert!(
        err_str.contains("live config unchanged"),
        "{failure_label} error must carry the reload-boundary pledge; got: {err_str}"
    );

    let (status, body) = send(h.app.clone(), auth_get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    let after: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let after_model = after
        .get("default_model")
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    assert_eq!(
        after_model, before_model,
        "{failure_label}: live default_model.model must be preserved after a failed reload"
    );
}

/// End-to-end regression for the second silent-defaults path that
/// `try_load_config` (#4664) closes: TOML parses cleanly but a field
/// has the wrong shape (`default_model = "string"` where a table is
/// expected). Pre-fix, `load_config` would warn and return defaults
/// and the reload would silently overwrite the live config; post-fix,
/// `POST /api/config/reload` must return 400.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_with_deserialize_shape_mismatch_returns_error_and_preserves_live_config() {
    let h = boot_router_with_api_key(API_KEY).await;
    assert_reload_rejects_and_preserves_default_model(
        &h,
        "config.toml",
        // TOML parses fine; deserialize fails because `default_model` is a struct.
        "default_model = \"not-a-table\"\n",
        &[],
        "deserialize-shape mismatch",
    )
    .await;
}

/// End-to-end regression for the third silent-defaults path: root
/// config is well-formed but `include = ["bad.toml"]` points at a
/// file that fails TOML parsing. Pre-fix, `resolve_config_includes`'s
/// error was swallowed by `load_config` and the reload proceeded with
/// the root only; post-fix, the reload must refuse.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_with_broken_include_returns_error_and_preserves_live_config() {
    let h = boot_router_with_api_key(API_KEY).await;
    assert_reload_rejects_and_preserves_default_model(
        &h,
        "config.toml",
        "include = [\"bad.toml\"]\nlog_level = \"debug\"\n",
        &[(
            "bad.toml",
            // Same duplicate-key shape as #4664, just inside the include.
            "[memory]\ndecay_rate = 0.1\n[memory]\ndecay_rate = 0.2\n",
        )],
        "broken include chain",
    )
    .await;
}

/// End-to-end regression locking in the `live config unchanged` reload-
/// boundary contract for the *post-loader* validation path
/// (`config_reload::validate_config_for_reload`). The strict loader
/// accepts the file (parses cleanly, deserialises into a valid
/// `KernelConfig`), but the validator rejects the result — e.g.
/// `network_enabled = true` with an empty `network.shared_secret`.
///
/// Without the contract being uniform, a future regression on this
/// branch would surface as a confusing assertion-helper diff rather
/// than the clear "wrapper missing" message. Asserting the substring
/// here means the helper covers every reload-rejection branch
/// regardless of which one trips.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_with_validation_failure_returns_error_and_preserves_live_config() {
    let h = boot_router_with_api_key(API_KEY).await;
    assert_reload_rejects_and_preserves_default_model(
        &h,
        "config.toml",
        // Parses + deserialises fine; validator refuses because
        // network_enabled requires a non-empty shared_secret.
        "network_enabled = true\n[network]\nshared_secret = \"\"\n",
        &[],
        "post-loader validation failure",
    )
    .await;
}

// ---------------------------------------------------------------------------
// GET /api/health/detail (#3776)
//
// Validates that the new operational metric sections (`budget`, `llm`) are
// wired into the response and serialize with the documented shape so that
// monitoring systems (Prometheus blackbox exporter, alerting rules) can rely
// on the field names.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn health_detail_includes_budget_and_llm_sections() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/health/detail")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");

    // Pre-existing fields must remain (regression guard).
    for key in [
        "status",
        "version",
        "uptime_seconds",
        "panic_count",
        "restart_count",
        "agent_count",
        "database",
        "memory",
        "config_warnings",
        "event_bus",
    ] {
        assert!(
            json.get(key).is_some(),
            "missing pre-existing field '{key}' in /api/health/detail: {json}"
        );
    }

    // New `budget` block — exposes already-collected MeteringEngine spend.
    let budget = json
        .get("budget")
        .expect("missing 'budget' section in /api/health/detail");
    for key in [
        "hourly_spend_usd",
        "hourly_limit_usd",
        "hourly_spend_percent",
        "daily_spend_usd",
        "daily_limit_usd",
        "daily_spend_percent",
        "monthly_spend_usd",
        "monthly_limit_usd",
        "monthly_spend_percent",
        "alert_threshold",
    ] {
        assert!(
            budget.get(key).is_some(),
            "missing budget.{key} in /api/health/detail: {budget}"
        );
    }
    // With no budget cap configured in the test kernel, the *_percent fields
    // must serialize as JSON null (operators distinguish "no cap" from "0%").
    for key in [
        "daily_spend_percent",
        "hourly_spend_percent",
        "monthly_spend_percent",
    ] {
        assert!(
            budget.get(key).expect("present").is_null(),
            "{key} must be null when no cap is configured: {budget}"
        );
    }

    // New `llm` block — sourced from query_model_performance() snapshot.
    let llm = json
        .get("llm")
        .expect("missing 'llm' section in /api/health/detail");
    for key in [
        "total_calls",
        "avg_latency_ms",
        "max_latency_ms",
        "model_count",
    ] {
        assert!(
            llm.get(key).is_some(),
            "missing llm.{key} in /api/health/detail: {llm}"
        );
    }
    // No LLM calls have been recorded in this fresh kernel.
    assert_eq!(llm["total_calls"].as_u64(), Some(0));
    assert_eq!(llm["max_latency_ms"].as_u64(), Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn health_detail_daily_spend_percent_reflects_configured_cap() {
    use librefang_types::config::BudgetConfig;

    let h = boot_router_with_api_key(API_KEY).await;

    // Set a non-zero daily cap so the *_percent fields become defined (0.0
    // for an empty kernel rather than null).
    h.state
        .kernel
        .update_budget_config(&|b: &mut BudgetConfig| {
            b.max_daily_usd = 25.0;
            b.max_hourly_usd = 5.0;
        });

    let (status, body) = send(h.app.clone(), auth_get("/api/health/detail")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let budget = &json["budget"];

    assert_eq!(budget["daily_limit_usd"].as_f64(), Some(25.0));
    assert_eq!(budget["hourly_limit_usd"].as_f64(), Some(5.0));
    assert_eq!(
        budget["daily_spend_percent"].as_f64(),
        Some(0.0),
        "daily_spend_percent must be 0.0 (not null) once a cap is set: {budget}"
    );
    assert_eq!(
        budget["hourly_spend_percent"].as_f64(),
        Some(0.0),
        "hourly_spend_percent must be 0.0 (not null) once a cap is set: {budget}"
    );
    // No monthly cap was set — must remain null.
    assert!(
        budget["monthly_spend_percent"].is_null(),
        "monthly_spend_percent must stay null when no monthly cap is set: {budget}"
    );
}

/// A `budget_status` query failure must surface `/api/health/detail` as a
/// scrubbed 500 too, not a 200 with fabricated zero-spend numbers baked into
/// the `budget` section.
/// Regression companion to `budget_status_returns_500_when_usage_query_fails`
/// in `budget_routes_test.rs`, which covers the same failure at `GET
/// /api/budget` (#7037).
#[tokio::test(flavor = "multi_thread")]
async fn health_detail_returns_500_when_budget_query_fails() {
    let h = boot_router_with_api_key(API_KEY).await;
    h.state
        .kernel
        .memory_substrate()
        .pool()
        .get()
        .unwrap()
        .execute("DROP TABLE usage_events", [])
        .unwrap();

    let (status, body) = send(h.app.clone(), auth_get("/api/health/detail")).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let rendered = String::from_utf8_lossy(&body);
    assert!(
        !rendered.contains("usage_events") && !rendered.contains("no such table"),
        "response body must not leak SQL identifiers: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// #5186 boot-path golden — stale renamed channel key fails boot loudly.
//
// Issue #5186 asked for an end-to-end guard that this class can't regress:
// when an operator's `config.toml` carries a channel-scoped field whose
// shape no longer matches the schema (the prototypical "stale renamed
// channel key" — old release accepted one shape, new release expects
// another), boot must abort with the field path in the error so the
// operator can pinpoint the offending line. The pre-#5186 behaviour
// silently substituted `KernelConfig::default()`, after which the
// daemon's downstream auth / token-resolve step would fail with a
// confusing "missing bot token" message that hid the real cause.
//
// The test goes through `librefang_kernel::config::load_config` — the
// exact entry point `LibreFangKernel::boot` uses to read `config.toml`
// from disk — and asserts:
//   1. it returns `Err` (fail-closed),
//   2. the error names the offending channel field,
//   3. the error does NOT mention authentication.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn boot_fails_on_stale_channel_output_format_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_path = tmp.path().join("config.toml");

    // A `[[sidecar_channels]]` entry where `restart_initial_backoff_ms`
    // has the wrong shape (string instead of `u64`) — the canonical
    // "stale renamed channel key" scenario the issue tracks: an older
    // release tolerated the string form, the current schema is the
    // numeric form, and the operator's config still carries the old
    // value.
    //
    // Witness rotated: whatsapp → webhook → google_chat (all
    // sidecar-migrated) → `[[sidecar_channels]]` itself, since
    // every channel now lives under this array-of-tables and
    // there are no in-process channels left to probe.
    let bad_toml = "\
[[sidecar_channels]]
name = \"probe\"
command = \"true\"
restart_initial_backoff_ms = \"eighty-eighty\"
";
    std::fs::write(&config_path, bad_toml).expect("write bad config.toml");

    let result = librefang_kernel::config::load_config(Some(&config_path));
    let err = result.expect_err(
        "stale-shape channel field must fail-close at load_config, \
         not silently substitute KernelConfig::default()",
    );

    // The error must name the offending field so the operator can fix
    // their config without guessing. The exact wording is owned by the
    // TOML deserializer; we lock the substring contract on the field
    // name and the section path.
    assert!(
        err.contains("restart_initial_backoff_ms"),
        "boot error must name the offending channel field; got: {err}"
    );
    assert!(
        err.contains("sidecar_channels"),
        "boot error must locate the field under [[sidecar_channels]]; got: {err}"
    );

    // The critical regression guard from the issue: the failure must NOT
    // be misclassified as an auth / token error downstream. Pre-#5186,
    // the load tolerated the bad value, defaults wiped the operator's
    // channel credentials, and the next layer surfaced it as an
    // authentication failure. Now we abort at parse time with the
    // field name and never reach auth.
    let lower = err.to_lowercase();
    assert!(
        !lower.contains("auth") && !lower.contains("bot token") && !lower.contains("unauthorized"),
        "boot error must not be misclassified as an auth failure (the \
         pre-#5186 downstream symptom); got: {err}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/ready (#6633)
//
// `/api/health` and `/api/ready` answer two different questions and must not
// be conflated: liveness ("is the process responsive?") never fails over a
// recoverable dependency outage, readiness ("can it accept work?") does. The
// tests below pin both halves of that contract over the real router, plus the
// public reachability a kubelet-issued probe depends on.
// ---------------------------------------------------------------------------

/// An env var name no environment sets. Pointing `embedding_api_key_env` at it
/// makes the Cohere embedding driver fail construction with `MissingApiKey`
/// deterministically, without the test mutating process-global environment
/// state that parallel tests share.
const ABSENT_EMBEDDING_KEY_ENV: &str = "LIBREFANG_TEST_ABSENT_EMBEDDING_KEY_6633";

/// Extract one named entry from the `checks` array of a health/ready payload.
fn ready_check<'a>(body: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    body.get("checks")
        .and_then(|c| c.as_array())
        .and_then(|checks| {
            checks
                .iter()
                .find(|c| c.get("name").and_then(|n| n.as_str()) == Some(name))
        })
        .unwrap_or_else(|| panic!("no '{name}' check in payload: {body}"))
}

/// The probe must be reachable with no credential — a kubelet holds none, and
/// a 401 would pin the pod out of Service endpoints permanently. The harness
/// boots with `api_key` set, so this also proves the route is in the
/// `is_public` allowlist rather than merely unauthenticated by accident.
#[tokio::test(flavor = "multi_thread")]
async fn ready_is_public_and_reports_ready_on_a_healthy_kernel() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), anon_get("/api/ready")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(json.get("status").and_then(|s| s.as_str()), Some("ready"));

    let database = ready_check(&json, "database");
    assert_eq!(database.get("status").and_then(|s| s.as_str()), Some("ok"));
    assert_eq!(
        database.get("required").and_then(|r| r.as_bool()),
        Some(true)
    );

    // No `embedding_provider` is configured in the default harness, so boot
    // takes the auto-detect path. Whether a driver materialises depends on
    // the ambient environment of the machine running the test — the point of
    // the contract is that it must not decide readiness either way.
    let embedding = ready_check(&json, "embedding");
    assert_eq!(
        embedding.get("required").and_then(|r| r.as_bool()),
        Some(false),
        "an unpinned embedding provider is an optional enhancement, not a \
         readiness requirement: {json}"
    );
}

/// The readiness payload reaches unauthenticated callers, so it must not carry
/// the reconnaissance surface that keeps `/api/health/detail` behind auth.
#[tokio::test(flavor = "multi_thread")]
async fn ready_payload_withholds_diagnostics_from_anonymous_callers() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), anon_get("/api/ready")).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    let object = json.as_object().expect("payload is an object");
    // Sorted, because `serde_json::Map` preserves insertion order here and the
    // assertion is about which keys exist, not the order they serialize in.
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["checks", "status"],
        "the readiness payload is deliberately minimal — adding a top-level \
         field means auditing it for disclosure first: {json}"
    );

    // Guard the specific values that made `/api/health/detail` auth-only.
    let rendered = String::from_utf8_lossy(&body);
    for leak in [
        env!("CARGO_PKG_VERSION"),
        "uptime",
        "panic",
        "restart",
        "agent_count",
        "provider",
        "model",
        "budget",
    ] {
        assert!(
            !rendered.contains(leak),
            "readiness payload must not disclose '{leak}': {rendered}"
        );
    }
}

/// When the operator pins a specific embedding provider and the daemon cannot
/// construct it, the deployment is not serving the memory semantics that were
/// asked for: readiness fails. Liveness must be unaffected in the same boot —
/// that separation is the whole reason `/api/ready` exists rather than
/// `/api/health` returning 503.
#[tokio::test(flavor = "multi_thread")]
async fn ready_fails_but_health_stays_200_when_a_pinned_embedding_driver_is_missing() {
    let h = boot_router_with_config(API_KEY, |config| {
        config.memory.fts_only = Some(false);
        config.memory.embedding_provider = Some("cohere".to_string());
        config.memory.embedding_api_key_env = Some(ABSENT_EMBEDDING_KEY_ENV.to_string());
    })
    .await;

    let (status, body) = send(h.app.clone(), anon_get("/api/ready")).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a pinned-but-unavailable embedding provider must fail readiness; \
         body: {}",
        String::from_utf8_lossy(&body)
    );

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(
        json.get("status").and_then(|s| s.as_str()),
        Some("not_ready")
    );
    let embedding = ready_check(&json, "embedding");
    assert_eq!(
        embedding.get("required").and_then(|r| r.as_bool()),
        Some(true)
    );
    assert_eq!(
        embedding.get("status").and_then(|s| s.as_str()),
        Some("error")
    );
    // The database is fine; only the pinned dependency failed.
    assert_eq!(
        ready_check(&json, "database")
            .get("status")
            .and_then(|s| s.as_str()),
        Some("ok")
    );

    let (health_status, health_body) = send(h.app.clone(), anon_get("/api/health")).await;
    assert_eq!(
        health_status,
        StatusCode::OK,
        "liveness must not fail for a readiness-only outage — a 503 here \
         would make Kubernetes restart-loop the pod; body: {}",
        String::from_utf8_lossy(&health_body)
    );
}

/// `fts_only` switches vector search off, so boot never builds an embedding
/// driver at all. A leftover `embedding_provider` in the same config must not
/// then be read as an unmet requirement — the mode wins.
#[tokio::test(flavor = "multi_thread")]
async fn ready_ignores_a_stale_embedding_provider_in_fts_only_mode() {
    let h = boot_router_with_config(API_KEY, |config| {
        config.memory.fts_only = Some(true);
        config.memory.embedding_provider = Some("cohere".to_string());
        config.memory.embedding_api_key_env = Some(ABSENT_EMBEDDING_KEY_ENV.to_string());
    })
    .await;

    let (status, body) = send(h.app.clone(), anon_get("/api/ready")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "fts_only is a supported mode, not a degraded one; body: {}",
        String::from_utf8_lossy(&body)
    );

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(json.get("status").and_then(|s| s.as_str()), Some("ready"));
    let embedding = ready_check(&json, "embedding");
    assert_eq!(
        embedding.get("required").and_then(|r| r.as_bool()),
        Some(false)
    );
    assert_eq!(
        embedding.get("status").and_then(|s| s.as_str()),
        Some("skipped")
    );
}

/// `"auto"` is the documented way to say "probe the environment, fall back to
/// text search" — an explicit statement that no particular provider is
/// promised. It must behave like an unset provider, not like a pin.
#[tokio::test(flavor = "multi_thread")]
async fn ready_treats_auto_embedding_provider_as_optional() {
    let h = boot_router_with_config(API_KEY, |config| {
        config.memory.fts_only = Some(false);
        config.memory.embedding_provider = Some("auto".to_string());
        config.memory.embedding_api_key_env = Some(ABSENT_EMBEDDING_KEY_ENV.to_string());
    })
    .await;

    let (status, body) = send(h.app.clone(), anon_get("/api/ready")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an 'auto' provider promises nothing specific, so its absence cannot \
         fail readiness; body: {}",
        String::from_utf8_lossy(&body)
    );

    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(
        ready_check(&json, "embedding")
            .get("required")
            .and_then(|r| r.as_bool()),
        Some(false)
    );
}

/// Readiness must answer for the process that is running, not for whatever
/// `config.toml` says right now.
///
/// `POST /api/config/reload` swaps the entire live `KernelConfig` whenever the
/// plan carries any hot action, and a `[memory]` change is classified
/// `restart_required` — the embedding driver is built once at boot and never
/// rebuilt. So an operator edit that adds `memory.embedding_provider`
/// alongside any hot-reloadable field would, if the probe read the requirement
/// from `config_ref()`, introduce a requirement against a driver that can
/// never appear. Readiness would sit at 503 forever while the daemon served
/// traffic perfectly well, and Kubernetes would hold the pod out of Service
/// endpoints with no path back except a manual restart: a config-file edit
/// turned into an outage.
///
/// `AppState::readiness_requires_embedding` snapshots the requirement at boot
/// to keep both halves of the comparison from the same point in time.
#[tokio::test(flavor = "multi_thread")]
async fn ready_ignores_an_embedding_provider_introduced_by_a_later_config_reload() {
    let h = boot_router_with_api_key(API_KEY).await;

    // Baseline: nothing pinned at boot, so readiness does not depend on an
    // embedding driver.
    let (status, body) = send(h.app.clone(), anon_get("/api/ready")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(
        ready_check(&json, "embedding")
            .get("required")
            .and_then(|r| r.as_bool()),
        Some(false),
        "precondition: the harness pins no provider at boot"
    );

    // Land a config.toml that pins an unsatisfiable embedding provider AND
    // touches a hot-reloadable field. The second part matters: `should_store_config`
    // only swaps the live config when the plan carries a hot action or a
    // no-op change, so a memory-only edit would not swap at all and the test
    // would pass for the wrong reason.
    let on_disk = format!(
        "log_level = \"debug\"\n\
         max_history_messages = 42\n\
         [memory]\n\
         fts_only = false\n\
         embedding_provider = \"cohere\"\n\
         embedding_api_key_env = \"{ABSENT_EMBEDDING_KEY_ENV}\"\n"
    );
    std::fs::write(h.home.join("config.toml"), on_disk).expect("write config.toml");

    let reload = Request::builder()
        .method(Method::POST)
        .uri("/api/config/reload")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let (reload_status, reload_body) = send(h.app.clone(), reload).await;
    assert_eq!(
        reload_status,
        StatusCode::OK,
        "reload should succeed (partially): {}",
        String::from_utf8_lossy(&reload_body)
    );
    let reload_json: serde_json::Value =
        serde_json::from_slice(&reload_body).expect("reload body is JSON");
    // Sanity-check that this edit really is the restart-required shape the
    // regression depends on. If a future reload-plan change made `[memory]`
    // hot-reloadable, this assertion fails loudly and the snapshot rationale
    // above needs revisiting — rather than the test silently going vacuous.
    assert_eq!(
        reload_json
            .get("restart_required")
            .and_then(|r| r.as_bool()),
        Some(true),
        "a [memory] edit must still be restart_required for this regression to \
         be meaningful: {reload_json}"
    );

    let (status, body) = send(h.app.clone(), anon_get("/api/ready")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a config reload must not be able to fail readiness for a driver the \
         running process was never asked to build; body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");
    assert_eq!(
        ready_check(&json, "embedding")
            .get("required")
            .and_then(|r| r.as_bool()),
        Some(false),
        "the requirement is a boot-time property: {json}"
    );
}

/// `GET /api/config/schema` must tell the dashboard which paths the write endpoint refuses.
///
/// Without it the SPA rendered an editable control for a deliberately non-writable field, the operator changed it, and the save came back 403 with no explanation — reported as "the per-field save appears to succeed, config.toml is unchanged" (#6636 observation (d)).
///
/// The server sends the resolved verdict rather than the allowlists, because writability is decided by an exact-path list, section prefixes, a depth-2-only rule and a secret-suffix scrub; re-deriving that in TypeScript would make the SPA a third place to keep in sync.
/// This test is the guard that the emitted set agrees with the write path itself.
#[tokio::test(flavor = "multi_thread")]
async fn config_schema_reports_the_paths_the_write_endpoint_refuses() {
    let h = boot_router_with_api_key(API_KEY).await;
    let (status, body) = send(h.app.clone(), auth_get("/api/config/schema")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).expect("response is JSON");

    let non_writable: std::collections::HashSet<&str> = json
        .get("x-non-writable")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Sanity floor: an empty or near-empty set would make every assertion below vacuous and would silently restore the old always-editable rendering.
    assert!(
        non_writable.len() > 20,
        "x-non-writable enumerated only {} paths — the schema walk is broken",
        non_writable.len()
    );

    // Deliberately non-writable, and the two the issue named.
    for path in [
        "approval.require_approval",
        "approval.second_factor",
        "external_auth.require_email_verified",
    ] {
        assert!(
            non_writable.contains(path),
            "`{path}` is refused by POST /api/config/set, so the dashboard must be told to \
             render it read-only; got {} entries",
            non_writable.len()
        );
    }

    // Writable paths must NOT be listed, or the dashboard would grey out fields the operator can legitimately change.
    for path in [
        "approval.auto_approve",
        "approval.totp_grace_period_secs",
        "registry.cache_ttl_secs",
        "log_level",
    ] {
        assert!(
            !non_writable.contains(path),
            "`{path}` is writable and must stay editable in the dashboard"
        );
    }

    // Cross-check the verdict against the write endpoint for one of each, so the emitted set cannot drift from the rule it is supposed to mirror.
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "approval.require_approval", "value": []}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the path reported as non-writable must actually be refused"
    );
    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "approval.auto_approve", "value": false}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the path reported as writable must actually be accepted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_escapes_untrusted_label_values() {
    let h = boot_router_with_api_key(API_KEY).await;
    register_metrics_agent(
        &h.state,
        "metrics\"agent\\path\ninjected_agent_metric 1",
        "provider\"\\\ninjected_provider_metric 1",
        "model\"\\\ninjected_model_metric 1",
    );

    let (status, body) = send(h.app.clone(), auth_get("/api/metrics")).await;
    assert_eq!(status, StatusCode::OK);
    let body = String::from_utf8(body).expect("metrics response is UTF-8");

    let expected = r#"librefang_tokens{agent="metrics\"agent\\path\ninjected_agent_metric 1",provider="provider\"\\\ninjected_provider_metric 1",model="model\"\\\ninjected_model_metric 1"} 3"#;
    assert!(body.contains(expected), "missing escaped metric: {body}");
    for injected_name in [
        "injected_agent_metric",
        "injected_provider_metric",
        "injected_model_metric",
    ] {
        assert!(
            !body.lines().any(|line| line.starts_with(injected_name)),
            "label content escaped into a metric line: {body}"
        );
    }
}

/// #8085: the SCRUB list governed only the path being assigned, so a wholesale
/// table write smuggled a credential-shaped field past it.
///
/// `is_writable_config_path` accepts a write one level below a writable section
/// prefix, and at that depth the handler assigns the submitted JSON wholesale
/// (`doc[section][key] = <value>`). `media.custom_stt` ends in neither a
/// scrubbed name nor `_env`, so the path check passed and the `api_key_env`
/// member of the posted table landed on disk — repointing the env var a
/// credential is resolved from, post-auth, over HTTP.
///
/// This is the reporter's exact payload. It must be refused, and the refusal
/// must name the offending key so the caller knows which field to edit on disk.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_refuses_a_credential_key_smuggled_inside_a_wholesale_table() {
    let h = boot_router_with_api_key(API_KEY).await;

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "media.custom_stt",
                "value": {
                    "api_key_env": "ANTHROPIC_API_KEY",
                    "base_url": "http://attacker.example/"
                }
            }),
        ),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a table carrying `api_key_env` must be refused however innocuous its path looks; \
         body: {}",
        String::from_utf8_lossy(&body)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    let message = parsed
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or_default();
    assert!(
        message.contains("api_key_env"),
        "the refusal must name the offending key so the operator knows what to edit on disk, \
         got: {message}"
    );
}

/// The same defect class, one level down and inside an array — a payload can
/// nest, so the scan has to recurse rather than inspect only the top level.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_refuses_a_credential_key_nested_below_the_top_level() {
    let h = boot_router_with_api_key(API_KEY).await;

    for value in [
        serde_json::json!({"outer": {"api_key_env": "ANTHROPIC_API_KEY"}}),
        serde_json::json!([{"token": "hunter2"}]),
        serde_json::json!({"list": [{"client_secret": "s"}]}),
    ] {
        let (status, body) = send(
            h.app.clone(),
            auth_post_json(
                "/api/config/set",
                serde_json::json!({"path": "media.custom_stt", "value": value}),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "nested credential key must be refused; payload {value}, body: {}",
            String::from_utf8_lossy(&body)
        );
    }
}

/// The fix must not close the legitimate route to the same fields.
///
/// A per-leaf write one level deeper is how the dashboard is meant to edit
/// these sections, and it is still governed by the path check exactly as
/// before: the non-secret leaf succeeds, and the credential leaf is refused by
/// the path check rather than the payload scan.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_still_allows_a_non_secret_leaf_write_under_the_same_section() {
    let h = boot_router_with_api_key(API_KEY).await;

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "media.custom_stt.base_url",
                "value": "http://localhost:9000/"
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a per-leaf non-secret write must keep working; body: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "media.custom_stt.api_key_env",
                "value": "ANTHROPIC_API_KEY"
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the credential leaf stays closed at its own path, as it was before this change"
    );
}

/// A payload with no credential-shaped key anywhere must be unaffected, so the
/// scan cannot become a blanket ban on object-valued writes.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_accepts_a_table_with_no_credential_shaped_key() {
    let h = boot_router_with_api_key(API_KEY).await;

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "provider_urls",
                "value": {"openai": "http://localhost:9001/v1"}
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a primitive-valued collection write must still be accepted; body: {}",
        String::from_utf8_lossy(&body)
    );
}

/// #8085 follow-up: `toml_edit` models `[media]` and `media = { … }` as
/// different `Item` variants, and the write path's `contains_table` /
/// `as_table_mut` guards recognised only the standard-table form.
///
/// An operator who hand-wrote a section as an inline table therefore lost it:
/// editing one leaf judged the section "missing" and replaced it with an empty
/// table, dropping every sibling key — `api_key_env` among them. That is
/// reachable precisely because #8085 recommends per-leaf writes as the safe
/// route for tables that carry credential fields.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_preserves_the_siblings_of_a_hand_written_inline_table() {
    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");

    std::fs::write(
        &config_path,
        "[media]\ncustom_stt = { base_url = \"http://old/\", api_key_env = \"MY_STT_KEY\" }\n",
    )
    .expect("seed config.toml");

    let (status, body) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({
                "path": "media.custom_stt.base_url",
                "value": "http://new/"
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "per-leaf write into an inline table must succeed; body: {}",
        String::from_utf8_lossy(&body)
    );

    let on_disk = std::fs::read_to_string(&config_path).expect("read back config.toml");
    assert!(
        on_disk.contains("http://new/"),
        "the edited leaf must be written; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("MY_STT_KEY"),
        "the sibling credential key must survive a per-leaf edit — this is the \
         data-loss bug this test exists for; got:\n{on_disk}"
    );
}

/// The same defect one level up: a top-level section written inline must not be
/// wiped by a depth-2 write.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_preserves_an_inline_section_on_a_depth_two_write() {
    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");

    std::fs::write(
        &config_path,
        "media = { audio_model = \"whisper-1\", describe_image_model = \"gpt-4o\" }\n",
    )
    .expect("seed config.toml");

    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "media.audio_model", "value": "whisper-2"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let on_disk = std::fs::read_to_string(&config_path).expect("read back config.toml");
    assert!(
        on_disk.contains("whisper-2"),
        "the edited key must be written; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("describe_image_model"),
        "the sibling key must survive; got:\n{on_disk}"
    );
}

/// Removal used `as_table_mut`, which is `None` for an inline table, so the key
/// stayed on disk while the handler still answered success — a delete that
/// silently did nothing.
#[tokio::test(flavor = "multi_thread")]
async fn config_set_null_removes_a_key_from_an_inline_table() {
    let h = boot_router_with_api_key(API_KEY).await;
    let config_path = h.home.join("config.toml");

    std::fs::write(
        &config_path,
        "media = { audio_model = \"whisper-1\", describe_image_model = \"gpt-4o\" }\n",
    )
    .expect("seed config.toml");

    let (status, _) = send(
        h.app.clone(),
        auth_post_json(
            "/api/config/set",
            serde_json::json!({"path": "media.audio_model", "value": null}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let on_disk = std::fs::read_to_string(&config_path).expect("read back config.toml");
    assert!(
        !on_disk.contains("audio_model"),
        "a reported-successful removal must actually remove the key; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("describe_image_model"),
        "the sibling key must survive the removal; got:\n{on_disk}"
    );
}
