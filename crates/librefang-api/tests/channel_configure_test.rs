//! Integration coverage for `POST /api/channels/sidecar/{name}/configure`.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
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

async fn boot_router() -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: API_KEY.to_string(),
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
    let home = config.home_dir.clone();
    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;
    RouterHarness {
        app,
        home,
        _tmp: tmp,
        state,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_rejects_included_sidecar_without_partial_writes() {
    let harness = boot_router().await;
    let config_path = harness.home.join("config.toml");
    let included_path = harness.home.join("channels.toml");
    std::fs::write(&config_path, "include = [\"channels.toml\"]\n").unwrap();
    std::fs::write(
        &included_path,
        "[[sidecar_channels]]\nname = \"telegram\"\ncommand = \"python3\"\n",
    )
    .unwrap();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/channels/sidecar/telegram/configure")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"values": {"TELEGRAM_BOT_TOKEN": "secret"}}).to_string(),
        ))
        .unwrap();
    let response = harness.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        "include = [\"channels.toml\"]\n"
    );
    assert!(!harness.home.join("secrets.env").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_rolls_back_secrets_when_config_write_fails() {
    let harness = boot_router().await;
    let config_path = harness.home.join("config.toml");
    let secrets_path = harness.home.join("secrets.env");
    let original_config = "sidecar_channels = \"not-an-array\"\n";
    let original_secrets = "TELEGRAM_BOT_TOKEN=old-secret\nKEEP_ME=unchanged\n";
    std::fs::write(&config_path, original_config).unwrap();
    std::fs::write(&secrets_path, original_secrets).unwrap();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/channels/sidecar/telegram/configure")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"values": {"TELEGRAM_BOT_TOKEN": "new-secret"}}).to_string(),
        ))
        .unwrap();
    let response = harness.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let response_text = String::from_utf8_lossy(&body);
    assert!(!response_text.contains("not-an-array"));
    assert!(!response_text.contains(&harness.home.display().to_string()));
    assert_eq!(
        std::fs::read_to_string(config_path).unwrap(),
        original_config
    );
    assert_eq!(
        std::fs::read_to_string(secrets_path).unwrap(),
        original_secrets
    );
}

async fn get_channels(app: &axum::Router) -> serde_json::Value {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/channels")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).expect("channels payload is JSON")
}

fn channel_row<'a>(payload: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    payload["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("no row named {name} in {payload}"))
}

async fn configure(
    app: &axum::Router,
    channel_type: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/channels/sidecar/{channel_type}/configure"))
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Multi-instance support (#8xxx): two `[[sidecar_channels]]` of the same
/// catalog type, distinguished by `instance_name`, must both configure
/// successfully, keep independent secrets and default agents, and both
/// surface on `GET /api/channels` with the shared `channel_type` and their
/// own `agent`.
#[tokio::test(flavor = "multi_thread")]
async fn configure_supports_a_second_named_instance_of_the_same_type() {
    let harness = boot_router().await;

    let (status, _) = configure(
        &harness.app,
        "telegram",
        serde_json::json!({
            "values": {"TELEGRAM_BOT_TOKEN": "token-default"},
            "agent": "ops-bot",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = configure(
        &harness.app,
        "telegram",
        serde_json::json!({
            "values": {"TELEGRAM_BOT_TOKEN": "token-support"},
            "instance_name": "telegram-support",
            "agent": "support-bot",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let payload = get_channels(&harness.app).await;
    let default_row = channel_row(&payload, "telegram");
    let support_row = channel_row(&payload, "telegram-support");

    assert_eq!(default_row["channel_type"], "telegram");
    assert_eq!(support_row["channel_type"], "telegram");
    assert_eq!(default_row["agent"], "ops-bot");
    assert_eq!(support_row["agent"], "support-bot");
    assert!(default_row["configured"].as_bool().unwrap());
    assert!(support_row["configured"].as_bool().unwrap());

    // Each instance's own secret namespace, not a shared bare key —
    // otherwise the second save would have clobbered the first bot's token.
    let secrets =
        std::fs::read_to_string(harness.home.join("secrets.env")).expect("secrets.env exists");
    assert!(secrets.contains("TELEGRAM_BOT_TOKEN=token-default"));
    assert!(secrets.contains("TELEGRAM_SUPPORT__TELEGRAM_BOT_TOKEN=token-support"));

    // The catalog picker entry for "telegram" must still be present so a
    // third instance can be added — it must not disappear once any
    // instance is configured.
    let discovery_row = payload["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "telegram" && row["configured"] == false);
    assert!(
        discovery_row.is_some(),
        "telegram catalog row must remain pickable after being configured: {payload}"
    );
}

/// A second catalog type cannot steal an instance name already used by a
/// different type — see `find_conflicting_channel_type`.
#[tokio::test(flavor = "multi_thread")]
async fn configure_rejects_name_conflict_with_a_different_channel_type() {
    let harness = boot_router().await;

    let (status, _) = configure(
        &harness.app,
        "telegram",
        serde_json::json!({
            "values": {"TELEGRAM_BOT_TOKEN": "token"},
            "instance_name": "shared-bot",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = configure(
        &harness.app,
        "ntfy",
        serde_json::json!({
            "values": {"NTFY_TOPIC": "alerts"},
            "instance_name": "shared-bot",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let payload = get_channels(&harness.app).await;
    let row = channel_row(&payload, "shared-bot");
    assert_eq!(
        row["channel_type"], "telegram",
        "rejected save must not have reassigned the existing instance's type: {payload}"
    );
}

/// Editing one non-secret field on an already-configured instance must not
/// require re-pasting every required secret (#8063).
///
/// The configure drawer never echoes a stored secret back as plaintext — it
/// shows "•••• (set — leave blank to keep)" and submits nothing for that field —
/// and the write path has always honoured that: a key absent from `values` never
/// reaches `upsert_secret`, so the stored secret survives. Only the required-field
/// pre-check disagreed, rejecting the save with `required field ... is missing or
/// empty`, which made the drawer's own promise unkeepable and left an operator no
/// way to change an allowlist without first hunting down their workspace tokens.
#[tokio::test(flavor = "multi_thread")]
async fn configure_keeps_a_stored_secret_when_the_form_omits_it() {
    let harness = boot_router().await;

    let (status, body) = configure(
        &harness.app,
        "telegram",
        serde_json::json!({"values": {"TELEGRAM_BOT_TOKEN": "token-original"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Second save: exactly what the drawer sends when the operator edits an
    // allowlist and leaves the secret field blank.
    let (status, body) = configure(
        &harness.app,
        "telegram",
        serde_json::json!({"values": {"ALLOWED_USERS": "111,222"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let secrets =
        std::fs::read_to_string(harness.home.join("secrets.env")).expect("secrets.env exists");
    assert!(
        secrets.contains("TELEGRAM_BOT_TOKEN=token-original"),
        "the omitted secret must be left exactly as it was: {secrets}"
    );
    let config =
        std::fs::read_to_string(harness.home.join("config.toml")).expect("config.toml exists");
    assert!(
        config.contains("ALLOWED_USERS"),
        "the edited non-secret field must be persisted: {config}"
    );

    // And the row still reports the secret as set, so the next drawer keeps
    // showing the "leave blank to keep" placeholder rather than an empty
    // required field.
    let payload = get_channels(&harness.app).await;
    let row = channel_row(&payload, "telegram");
    let token = row["fields"]
        .as_array()
        .expect("fields is an array")
        .iter()
        .find(|f| f["key"] == "TELEGRAM_BOT_TOKEN")
        .unwrap_or_else(|| panic!("no TELEGRAM_BOT_TOKEN field in {row}"));
    assert_eq!(token["has_value"], true, "{row}");
}

/// The relaxation above is "has a value after this save", not "required fields
/// are optional now": a first save for an instance with nothing stored is still
/// a 400, and still writes neither file.
///
/// Uses a named second instance so the outcome is decided by `secrets.env` alone.
/// The required-secret check accepts the daemon's own environment as a source for the bare-key path only, so a `TELEGRAM_BOT_TOKEN` exported into the test process cannot make this pass for the wrong reason.
/// (That gate is deliberately narrower than what the child actually sees — the supervisor never calls `Command::env_clear`, so an exported bare key reaches every sidecar child — because a namespaced instance must name its own secret rather than silently running on the first instance's token.)
#[tokio::test(flavor = "multi_thread")]
async fn configure_still_rejects_a_required_secret_that_was_never_stored() {
    let harness = boot_router().await;

    let (status, body) = configure(
        &harness.app,
        "telegram",
        serde_json::json!({
            "values": {"ALLOWED_USERS": "111"},
            "instance_name": "telegram-hr",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("TELEGRAM_BOT_TOKEN"),
        "the 400 must name the field that has no value: {body}"
    );
    assert!(
        !harness.home.join("secrets.env").exists(),
        "a rejected save must not have written secrets.env"
    );
    let config = std::fs::read_to_string(harness.home.join("config.toml")).unwrap_or_default();
    assert!(
        !config.contains("telegram-hr"),
        "a rejected save must not have written the instance block: {config}"
    );
}
/// `instance_secret_prefix` uppercases and maps every non-alphanumeric byte to `_`, so it is many-to-one.
/// Two instances that collapse onto the same prefix share one `<PREFIX>__KEY` namespace in secrets.env, and the second save would overwrite the first one's token — which is the opposite of the isolation multi-instance support is for, and what the feature's own changelog promises ("so two bots never share a token").
///
/// `warn_secret_prefix_collisions` only reports this from the boot / reload loop, after the damage.
/// The write path has to refuse it.
#[tokio::test(flavor = "multi_thread")]
async fn configure_refuses_an_instance_name_that_shares_a_secret_namespace() {
    let harness = boot_router().await;

    let (status, body) = configure(
        &harness.app,
        "telegram",
        serde_json::json!({
            "values": {"TELEGRAM_BOT_TOKEN": "first-token"},
            "instance_name": "bot-1",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // `bot.1` normalizes to `BOT_1`, exactly where `bot-1` keeps its secret.
    let (status, body) = configure(
        &harness.app,
        "telegram",
        serde_json::json!({
            "values": {"TELEGRAM_BOT_TOKEN": "second-token"},
            "instance_name": "bot.1",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "sidecar_secret_prefix_conflict", "{body}");

    // The refusal must land before the first mutation: the original token is still there and no second instance was created.
    let secrets = std::fs::read_to_string(harness.home.join("secrets.env")).unwrap_or_default();
    assert!(
        secrets.contains("BOT_1__TELEGRAM_BOT_TOKEN=first-token"),
        "the rejected save overwrote the first instance's secret: {secrets}"
    );
    assert!(
        !secrets.contains("second-token"),
        "the rejected save wrote its own token anyway: {secrets}"
    );
    let payload = get_channels(&harness.app).await;
    assert!(
        payload.to_string().contains("bot-1") && !payload.to_string().contains("bot.1"),
        "the rejected instance must not have been created: {payload}"
    );
}

/// Re-saving the same instance is `upsert_sidecar_block` editing it in place, not a second instance moving into its namespace, so it must still work.
#[tokio::test(flavor = "multi_thread")]
async fn configure_still_allows_reconfiguring_the_same_named_instance() {
    let harness = boot_router().await;

    for token in ["first-token", "second-token"] {
        let (status, body) = configure(
            &harness.app,
            "telegram",
            serde_json::json!({
                "values": {"TELEGRAM_BOT_TOKEN": token},
                "instance_name": "bot-1",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let secrets = std::fs::read_to_string(harness.home.join("secrets.env")).unwrap_or_default();
    assert!(
        secrets.contains("BOT_1__TELEGRAM_BOT_TOKEN=second-token"),
        "re-saving an instance must update its own secret: {secrets}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn registry_returns_typed_metadata_array() {
    let harness = boot_router().await;
    let channels_dir = harness.home.join("channels");
    std::fs::create_dir_all(&channels_dir).unwrap();
    std::fs::write(
        channels_dir.join("audit-test.toml"),
        "id = \"audit-test\"\nname = \"Audit Test\"\ndescription = \"typed response\"\n",
    )
    .unwrap();

    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/channels/registry")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    let response = harness.app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let entry = metadata
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "audit-test")
        .unwrap();
    assert_eq!(entry["name"], "Audit Test");
    assert_eq!(entry["description"], "typed response");
}
