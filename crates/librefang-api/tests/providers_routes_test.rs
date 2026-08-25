//! Integration tests for the model-catalog & provider-management endpoints.
//!
//! Refs #3571 — "~80% of registered HTTP routes have no integration test."
//! This file targets the providers/models slice (`crates/librefang-api/src/
//! routes/providers.rs`). It mounts the real `providers::router()` against a
//! `MockKernel`-backed `AppState` and exercises happy + error paths through
//! `tower::ServiceExt::oneshot` — same harness pattern as `users_test.rs`.
//!
//! Out of scope (not exercised here, by design):
//!   - `POST /api/providers/github-copilot/oauth/*` — outbound device-flow HTTP
//!   - `GET  /api/providers/ollama/detect`          — outbound HTTP probe
//!   - `POST /api/catalog/update`                   — outbound network sync
//!   - Most `POST /api/providers/{name}/test` success paths — outbound HTTP / CLI probe.
//!     OpenRouter key-save and provider-test paths are covered with local mock servers.
//!
//! These would either flake on CI (real network) or contaminate other test
//! binaries running in parallel via `std::env::set_var`. Per CLAUDE.md
//! "no global env mutation, no fs writes outside tempfile."
//!
//! `DELETE /api/providers/{name}/key` IS exercised — only for providers
//! whose env var name (`CLAUDE_CODE_API_KEY`, `OLLAMA_API_KEY`) is not
//! referenced by any other test in this workspace, so the
//! `std::env::remove_var` call inside the handler is a no-op on shared
//! state. The assertion is on the catalog's `auth_status` flip, which is
//! the regression surface for #4803.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{test_catalog_baseline, MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

/// Boots a kernel with a sane default-model provider so handlers that fall
/// back to `config.default_model.provider` (notably `add_custom_model`)
/// don't end up tagging entries with the placeholder `"auto"` provider.
///
/// Seeds the model catalog with [`test_catalog_baseline`] so tests that
/// reference specific ids (notably `openai:gpt-4o-mini` for the
/// capability-override flow) don't depend on the network-fed
/// `sync_registry` baseline that flakes on CI when GitHub rate-limits or
/// the runner is partitioned. Validation/error-path tests in this file
/// either target unknown ids (404 paths) or only inspect envelope shape,
/// so a non-empty deterministic catalog leaves them unaffected.
fn boot() -> Harness {
    let test = TestAppState::with_builder(
        MockKernelBuilder::new()
            .with_config(|cfg| {
                cfg.default_model = librefang_types::config::DefaultModelConfig {
                    provider: "openai".to_string(),
                    model: "gpt-4o-mini".to_string(),
                    api_key_env: "OPENAI_API_KEY".to_string(),
                    base_url: None,
                    message_timeout_secs: 300,
                    extra_params: std::collections::BTreeMap::new(),
                    cli_profile_dirs: Vec::new(),
                };
            })
            .with_catalog_seed(test_catalog_baseline()),
    );

    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::providers::router())
        .with_state(state.clone());

    Harness {
        app,
        _state: state,
        _test: test,
    }
}

/// Boots a harness whose `default_model` wires claude-code CLI-profile
/// rotation at `profile_dir`, so `list_models` / `get_model` resolve the
/// model from `<profile_dir>/settings.json` deterministically — no process
/// env mutation, no shared FS, safe under parallel test execution.
fn boot_with_claude_profile(profile_dir: &str) -> Harness {
    let dir = profile_dir.to_string();
    let test = TestAppState::with_builder(
        MockKernelBuilder::new()
            .with_config(move |cfg| {
                cfg.default_model = librefang_types::config::DefaultModelConfig {
                    provider: "claude-code".to_string(),
                    model: "sonnet".to_string(),
                    api_key_env: String::new(),
                    base_url: None,
                    message_timeout_secs: 300,
                    extra_params: std::collections::BTreeMap::new(),
                    cli_profile_dirs: vec![dir.clone()],
                };
            })
            .with_catalog_seed(test_catalog_baseline()),
    );
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::providers::router())
        .with_state(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

/// The model the detector resolves: `ANTHROPIC_MODEL` env wins over the profile
/// `settings.json` (matching the CLI's own precedence), so the assertion stays
/// deterministic whether or not the test runner exports `ANTHROPIC_MODEL`.
fn expected_claude_model(settings_model: &str) -> String {
    std::env::var("ANTHROPIC_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| settings_model.to_string())
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

// ---------------------------------------------------------------------------
// GET /api/models
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_models_returns_well_formed_envelope() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("models").and_then(|v| v.as_array()).is_some());
    assert!(body.get("total").and_then(|v| v.as_u64()).is_some());
    assert!(body.get("available").and_then(|v| v.as_u64()).is_some());
    // Built-in catalog has at least one entry from the registry.
    assert!(body["total"].as_u64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_models_filters_by_unknown_provider_yields_empty() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models?provider=__no_such_provider__",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["models"].as_array().unwrap().len(), 0);
}

/// Regression #7780, end to end through `GET /api/models`.
///
/// A model discovered behind an OpenAI-compatible gateway used to enter the catalog with `context_window: 131_072` regardless of what the gateway said, so the route served a fabricated number that neither an operator nor the budget math behind `resolve_context_window` could tell from a measured one.
/// This asserts both halves of the fix on the wire: a reported capacity survives to the response, and an unreported one is served as the catalog's `unknown` encoding with `limits_known: false` rather than as the old literal.
#[tokio::test(flavor = "multi_thread")]
async fn list_models_serves_probed_capacity_and_never_the_hardcoded_literal() {
    let h = boot();
    h._state.kernel.model_catalog_update(&mut |catalog| {
        catalog.merge_discovered_models(
            "openai",
            &[
                // LiteLLM in front of a build that answers /model/info.
                librefang_kernel::provider_health::DiscoveredModelInfo {
                    context_window: Some(8_192),
                    max_output_tokens: Some(2_048),
                    ..librefang_kernel::provider_health::DiscoveredModelInfo::bare(
                        "gw-reports-capacity",
                    )
                },
                // The reproduction from the issue: the bare OpenAI shape, which carries `id` / `object` / `created` / `owned_by` and nothing else.
                librefang_kernel::provider_health::DiscoveredModelInfo::bare("gw-reports-nothing"),
            ],
        );
    });

    let (status, body) = json_request(&h, Method::GET, "/api/models?provider=openai", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let rows = body["models"].as_array().expect("models array");
    let by_id = |id: &str| {
        rows.iter()
            .find(|m| m["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("{id} missing from {body}"))
    };

    let reported = by_id("gw-reports-capacity");
    assert_eq!(reported["context_window"], 8_192);
    assert_eq!(reported["max_output_tokens"], 2_048);
    assert_eq!(reported["limits_known"], true);

    let silent = by_id("gw-reports-nothing");
    assert_ne!(
        silent["context_window"], 131_072,
        "the fabricated literal must not reach any surface: {silent}"
    );
    assert_eq!(silent["context_window"], 0);
    assert_eq!(silent["max_output_tokens"], 0);
    assert_eq!(
        silent["limits_known"], false,
        "the route must say the numbers have no source, so a reader can tell \
         `unknown` apart from an image model's `not applicable`"
    );
}

// ---------------------------------------------------------------------------
// GET /api/models/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_model_unknown_id_returns_404() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/models/__no_such_model__", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.get("error").is_some() || body.get("message").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn list_models_surfaces_cli_profile_configured_model() {
    // A claude-code profile dir pinning a distinctive model in its settings.json.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("settings.json"),
        r#"{"model": "test-cli-detected-model-xyz"}"#,
    )
    .unwrap();
    let h = boot_with_claude_profile(&tmp.path().to_string_lossy());

    let (status, body) =
        json_request(&h, Method::GET, "/api/models?provider=claude-code", None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["models"].as_array().unwrap();
    // Wiring guard: a provider-filtered response must contain only that
    // provider's rows — i.e. the synthesized-row loop honours `provider_filter`
    // and does not leak codex/gemini/qwen rows into a claude-code query.
    assert!(
        rows.iter().all(|m| m["provider"] == "claude-code"),
        "every row under ?provider=claude-code must be claude-code: {rows:?}"
    );
    // The configured model is surfaced as a cli_config-sourced row.
    let expected = expected_claude_model("test-cli-detected-model-xyz");
    let synth = rows
        .iter()
        .find(|m| m["source"] == "cli_config")
        .expect("a cli_config-sourced claude-code row must be present");
    assert_eq!(synth["id"], format!("claude-code/{expected}"));
    assert_eq!(synth["tier"], "custom");
    // Shape parity with catalog rows: image-cost keys present (null).
    assert!(synth.get("image_input_cost_per_m").is_some());
    assert!(synth.get("image_output_cost_per_m").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_model_resolves_cli_detected_id() {
    // The id list_models advertises for a CLI-detected model must also resolve
    // via GET /api/models/{id} — list and detail agree on advertised ids.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("settings.json"),
        r#"{"model": "test-cli-detected-model-xyz"}"#,
    )
    .unwrap();
    let h = boot_with_claude_profile(&tmp.path().to_string_lossy());

    let expected = expected_claude_model("test-cli-detected-model-xyz");
    let (status, body) = json_request(
        &h,
        Method::GET,
        &format!("/api/models/claude-code/{expected}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], format!("claude-code/{expected}"));
    assert_eq!(body["source"], "cli_config");
    // get_model rows carry an `overrides` object like catalog rows.
    assert!(body.get("overrides").is_some());
}

// ---------------------------------------------------------------------------
// Aliases — list / create / delete round-trip
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn aliases_list_starts_with_envelope() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/models/aliases", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("aliases").and_then(|v| v.as_array()).is_some());
    assert!(body.get("total").and_then(|v| v.as_u64()).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn create_alias_rejects_missing_alias_field() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/aliases",
        Some(serde_json::json!({ "model_id": "gpt-4o" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_alias_rejects_missing_model_id_field() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/aliases",
        Some(serde_json::json!({ "alias": "fast" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_alias_then_list_then_delete_round_trips() {
    let h = boot();

    // Create
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/models/aliases",
        Some(serde_json::json!({
            "alias": "Test-Alias-3571",
            "model_id": "gpt-4o-mini",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    // Handler lowercases the alias name on return.
    assert_eq!(body["alias"].as_str().unwrap(), "test-alias-3571");
    assert_eq!(body["model_id"].as_str().unwrap(), "gpt-4o-mini");

    // List should include it.
    let (status, body) = json_request(&h, Method::GET, "/api/models/aliases", None).await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["aliases"].as_array().unwrap();
    let found = entries.iter().any(|e| {
        e["alias"].as_str() == Some("test-alias-3571")
            && e["model_id"].as_str() == Some("gpt-4o-mini")
    });
    assert!(found, "newly created alias must appear in /models/aliases");

    // Duplicate should return 409.
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/aliases",
        Some(serde_json::json!({
            "alias": "Test-Alias-3571",
            "model_id": "gpt-4o-mini",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Delete
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/aliases/test-alias-3571",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Second delete -> 404.
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/aliases/test-alias-3571",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Custom models — POST /api/models/custom + DELETE /api/models/custom/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn add_custom_model_rejects_missing_id() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/custom",
        Some(serde_json::json!({ "display_name": "no id" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn add_custom_model_then_get_then_delete_round_trips() {
    let h = boot();

    // Create
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/models/custom",
        Some(serde_json::json!({
            "id": "test-custom-3571",
            "provider": "openai",
            "display_name": "Test Custom 3571",
            "context_window": 64_000,
            "max_output_tokens": 4_096,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["id"].as_str().unwrap(), "test-custom-3571");
    assert_eq!(body["status"].as_str().unwrap(), "added");

    // Duplicate -> 409.
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/models/custom",
        Some(serde_json::json!({
            "id": "test-custom-3571",
            "provider": "openai",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // GET via /api/models/{id}
    let (status, body) = json_request(&h, Method::GET, "/api/models/test-custom-3571", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"].as_str().unwrap(), "test-custom-3571");
    assert_eq!(body["provider"].as_str().unwrap(), "openai");

    // Delete
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/custom/test-custom-3571",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Second delete -> 404.
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/custom/test-custom-3571",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Per-model overrides — GET / PUT / DELETE /api/models/overrides/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn model_overrides_unset_returns_empty_object() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Handler returns `{}` when no overrides exist for the key.
    assert!(body.is_object());
    assert!(body.as_object().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn model_overrides_set_then_get_then_delete_round_trips() {
    let h = boot();

    // PUT
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/models/overrides/openai:gpt-4o-mini",
        Some(serde_json::json!({ "temperature": 0.42 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // PUT now returns the persisted ModelOverrides entity (Refs #3832), not
    // an ack envelope — the value we just wrote should be reflected back.
    assert_eq!(
        body["temperature"].as_f64(),
        Some(0.42_f32 as f64),
        "PUT response should echo the persisted override, got {body}"
    );

    // GET — overrides now present.
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.is_object() && !body.as_object().unwrap().is_empty(),
        "overrides body should be a non-empty object after PUT, got {body}"
    );

    // DELETE
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // GET again -> empty object.
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_object() && body.as_object().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Capability overrides (refs #4745)
// User overrides on `supports_tools / vision / streaming / thinking` must
// surface in the GET /api/models/{id}, GET /api/models, and
// GET /api/providers/{name} responses, and revert when the override is
// deleted. Tests pin behaviour for both directions (force-on, force-off) so
// the catalog default never has to be hardcoded — we capture it first.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn capability_override_flips_effective_value_in_get_model() {
    let h = boot();
    let model_id = "gpt-4o-mini";
    let key = "openai:gpt-4o-mini";

    // Capture the catalog defaults so we can pick override values that are
    // guaranteed to differ.
    let (status, base) =
        json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let base_tools = base["supports_tools"].as_bool().unwrap();
    let base_vision = base["supports_vision"].as_bool().unwrap();
    let base_thinking = base["supports_thinking"].as_bool().unwrap();
    let base_streaming = base["supports_streaming"].as_bool().unwrap();

    // PUT the negation of every capability.
    let payload = serde_json::json!({
        "supports_tools": !base_tools,
        "supports_vision": !base_vision,
        "supports_streaming": !base_streaming,
        "supports_thinking": !base_thinking,
    });
    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/models/overrides/{key}"),
        Some(payload),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["supports_tools"].as_bool(), Some(!base_tools));
    assert_eq!(body["supports_vision"].as_bool(), Some(!base_vision));
    assert_eq!(body["supports_streaming"].as_bool(), Some(!base_streaming));
    assert_eq!(body["supports_thinking"].as_bool(), Some(!base_thinking));

    // GET /api/models/{id} now reports the overridden values.
    let (status, body) =
        json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["supports_tools"].as_bool(), Some(!base_tools));
    assert_eq!(body["supports_vision"].as_bool(), Some(!base_vision));
    assert_eq!(body["supports_streaming"].as_bool(), Some(!base_streaming));
    assert_eq!(body["supports_thinking"].as_bool(), Some(!base_thinking));
    // The raw `overrides` envelope still echoes the user's intent.
    assert_eq!(
        body["overrides"]["supports_tools"].as_bool(),
        Some(!base_tools)
    );
    // `capabilities_catalog` is the unmerged catalog default — it must NOT
    // shift when an override is active, otherwise the override-editor UI
    // can't render "Auto = revert to catalog" correctly.
    let cat = &body["capabilities_catalog"];
    assert_eq!(cat["supports_tools"].as_bool(), Some(base_tools));
    assert_eq!(cat["supports_vision"].as_bool(), Some(base_vision));
    assert_eq!(cat["supports_streaming"].as_bool(), Some(base_streaming));
    assert_eq!(cat["supports_thinking"].as_bool(), Some(base_thinking));

    // GET /api/models?provider=openai also reflects the override.
    let (status, listed) = json_request(&h, Method::GET, "/api/models?provider=openai", None).await;
    assert_eq!(status, StatusCode::OK);
    let entry = listed["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"].as_str() == Some(model_id))
        .expect("gpt-4o-mini should be in the openai catalog slice");
    assert_eq!(entry["supports_tools"].as_bool(), Some(!base_tools));
    assert_eq!(entry["supports_vision"].as_bool(), Some(!base_vision));
    // `capabilities_catalog` must also be present and unaffected by override.
    assert_eq!(
        entry["capabilities_catalog"]["supports_tools"].as_bool(),
        Some(base_tools)
    );

    // GET /api/providers/openai also surfaces the effective values for the
    // single-provider drilldown.
    let (status, prov) = json_request(&h, Method::GET, "/api/providers/openai", None).await;
    assert_eq!(status, StatusCode::OK);
    let prov_entry = prov["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"].as_str() == Some(model_id))
        .expect("gpt-4o-mini should be in /api/providers/openai");
    assert_eq!(prov_entry["supports_tools"].as_bool(), Some(!base_tools));
    assert_eq!(
        prov_entry["supports_thinking"].as_bool(),
        Some(!base_thinking)
    );
    assert_eq!(
        prov_entry["capabilities_catalog"]["supports_thinking"].as_bool(),
        Some(base_thinking),
        "capabilities_catalog in /api/providers/{{name}} must be unmerged"
    );

    // DELETE — effective values revert to catalog defaults.
    let (status, _) = json_request(
        &h,
        Method::DELETE,
        &format!("/api/models/overrides/{key}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) =
        json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["supports_tools"].as_bool(), Some(base_tools));
    assert_eq!(body["supports_vision"].as_bool(), Some(base_vision));
    assert_eq!(body["supports_streaming"].as_bool(), Some(base_streaming));
    assert_eq!(body["supports_thinking"].as_bool(), Some(base_thinking));
}

#[tokio::test(flavor = "multi_thread")]
async fn capability_override_partial_only_flips_set_fields() {
    let h = boot();
    let model_id = "gpt-4o-mini";
    let key = "openai:gpt-4o-mini";

    let (_, base) = json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    let base_tools = base["supports_tools"].as_bool().unwrap();
    let base_vision = base["supports_vision"].as_bool().unwrap();

    // Override only `supports_vision`. `supports_tools` must stay at the
    // catalog default — partial overrides don't touch other fields.
    let (status, _) = json_request(
        &h,
        Method::PUT,
        &format!("/api/models/overrides/{key}"),
        Some(serde_json::json!({ "supports_vision": !base_vision })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    assert_eq!(
        body["supports_tools"].as_bool(),
        Some(base_tools),
        "supports_tools must keep its catalog default when not in override payload"
    );
    assert_eq!(body["supports_vision"].as_bool(), Some(!base_vision));
}

// ---------------------------------------------------------------------------
// Capacity-limit overrides (refs #7774)
// `context_window` / `max_output_tokens` are operator-editable at any time
// through PUT /api/models/overrides/{id}, and the effective value must show up
// on every surface that reports a model's limits, with the raw catalog value
// kept alongside under `limits_catalog` so a revert-target UI can render it.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn context_window_override_flips_effective_value_on_every_model_surface() {
    let h = boot();
    let model_id = "gpt-4o-mini";
    let key = "openai:gpt-4o-mini";

    // Baseline: no override, so every surface reports the catalog value and
    // `limits_catalog` agrees with it.
    let (status, base) =
        json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let catalog_window = base["context_window"].as_u64().unwrap();
    let catalog_max_out = base["max_output_tokens"].as_u64().unwrap();
    assert!(catalog_window > 0, "fixture sanity: {base}");
    assert_eq!(
        base["limits_catalog"]["context_window"].as_u64(),
        Some(catalog_window)
    );
    assert_eq!(
        base["limits_catalog"]["max_output_tokens"].as_u64(),
        Some(catalog_max_out)
    );

    // The correction an operator makes when the gateway under-reports: half
    // the catalog window, and a distinct output cap.
    let corrected_window = catalog_window / 2;
    let corrected_max_out = catalog_max_out / 2;
    let (status, body) = json_request(
        &h,
        Method::PUT,
        &format!("/api/models/overrides/{key}"),
        Some(serde_json::json!({
            "context_window": corrected_window,
            "max_output_tokens": corrected_max_out,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["context_window"].as_u64(),
        Some(corrected_window),
        "PUT must echo the persisted context_window: {body}"
    );
    assert_eq!(body["max_output_tokens"].as_u64(), Some(corrected_max_out));

    // GET the override back — the field round-trips through
    // `model_overrides.json`, which is what makes it editable at any time.
    let (status, stored) = json_request(
        &h,
        Method::GET,
        &format!("/api/models/overrides/{key}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["context_window"].as_u64(), Some(corrected_window));

    // GET /api/models/{id} — effective value shifts, catalog value does not.
    let (status, detail) =
        json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["context_window"].as_u64(), Some(corrected_window));
    assert_eq!(
        detail["max_output_tokens"].as_u64(),
        Some(corrected_max_out)
    );
    assert_eq!(
        detail["limits_catalog"]["context_window"].as_u64(),
        Some(catalog_window),
        "limits_catalog must stay unmerged so the UI can offer a revert: {detail}"
    );
    assert_eq!(
        detail["limits_catalog"]["max_output_tokens"].as_u64(),
        Some(catalog_max_out)
    );

    // GET /api/models — the list surface agrees with the detail surface.
    let (status, listed) = json_request(&h, Method::GET, "/api/models?provider=openai", None).await;
    assert_eq!(status, StatusCode::OK);
    let entry = listed["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"].as_str() == Some(model_id))
        .expect("gpt-4o-mini should be in the openai catalog slice");
    assert_eq!(entry["context_window"].as_u64(), Some(corrected_window));
    assert_eq!(
        entry["limits_catalog"]["context_window"].as_u64(),
        Some(catalog_window)
    );

    // GET /api/providers/{name} — the drilldown surface too.
    let (status, prov) = json_request(&h, Method::GET, "/api/providers/openai", None).await;
    assert_eq!(status, StatusCode::OK);
    let prov_entry = prov["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"].as_str() == Some(model_id))
        .expect("gpt-4o-mini should be in /api/providers/openai");
    assert_eq!(
        prov_entry["context_window"].as_u64(),
        Some(corrected_window)
    );
    assert_eq!(
        prov_entry["limits_catalog"]["context_window"].as_u64(),
        Some(catalog_window)
    );

    // DELETE — every surface reverts to the catalog value.
    let (status, _) = json_request(
        &h,
        Method::DELETE,
        &format!("/api/models/overrides/{key}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, reverted) =
        json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reverted["context_window"].as_u64(), Some(catalog_window));
    assert_eq!(
        reverted["max_output_tokens"].as_u64(),
        Some(catalog_max_out)
    );
}

/// Refs #7774. Backward compatibility: an overrides document that carries only
/// inference parameters must leave the reported limits exactly where they were.
/// This is the guard against the new field quietly changing what an existing
/// install reports.
#[tokio::test(flavor = "multi_thread")]
async fn an_overrides_document_without_limits_leaves_reported_limits_unchanged() {
    let h = boot();
    let model_id = "gpt-4o-mini";
    let key = "openai:gpt-4o-mini";

    let (_, base) = json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    let catalog_window = base["context_window"].as_u64().unwrap();
    let catalog_max_out = base["max_output_tokens"].as_u64().unwrap();

    let (status, _) = json_request(
        &h,
        Method::PUT,
        &format!("/api/models/overrides/{key}"),
        Some(serde_json::json!({ "temperature": 0.3, "max_tokens": 4_096 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, after) = json_request(&h, Method::GET, &format!("/api/models/{model_id}"), None).await;
    assert_eq!(after["context_window"].as_u64(), Some(catalog_window));
    assert_eq!(after["max_output_tokens"].as_u64(), Some(catalog_max_out));
    assert!(
        after["overrides"]["context_window"].is_null(),
        "an unset limit must not be serialized: {after}"
    );
}

/// Refs #7774 / #6209. `max_tokens` — the per-request output cap — stays ahead
/// of the `max_output_tokens` capacity correction in the provider headline,
/// while the capacity correction still beats the catalog. Pins the three-way
/// order so neither field silently swallows the other.
#[tokio::test(flavor = "multi_thread")]
async fn provider_headline_ranks_max_tokens_above_the_capacity_override() {
    let h = boot();
    let key = "openai:gpt-4o-mini";
    let headline = |body: &serde_json::Value| -> Option<u64> {
        body["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"].as_str() == Some("openai"))
            .and_then(|p| p["max_output_tokens"].as_u64())
    };

    // Capacity correction alone → it wins over the catalog value.
    let (status, _) = json_request(
        &h,
        Method::PUT,
        &format!("/api/models/overrides/{key}"),
        Some(serde_json::json!({ "max_output_tokens": 6_000 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(headline(&body), Some(6_000));

    // Both set → the explicit per-request cap wins.
    let (status, _) = json_request(
        &h,
        Method::PUT,
        &format!("/api/models/overrides/{key}"),
        Some(serde_json::json!({ "max_output_tokens": 6_000, "max_tokens": 2_000 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(headline(&body), Some(2_000));
}

// ---------------------------------------------------------------------------
// GET /api/providers + GET /api/providers/{name}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_providers_returns_well_formed_envelope() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("providers").and_then(|v| v.as_array()).is_some());
    assert!(body.get("total").and_then(|v| v.as_u64()).is_some());
    let providers = body["providers"].as_array().unwrap();
    // Every entry must have the required identity fields.
    for p in providers {
        assert!(p["id"].is_string(), "provider entry missing 'id': {p}");
        assert!(
            p["display_name"].is_string(),
            "provider entry missing 'display_name': {p}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_provider_unknown_returns_404() {
    let h = boot();
    let (status, _body) =
        json_request(&h, Method::GET, "/api/providers/__no_such_provider__", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Issue #6209 — the provider list and detail endpoints surface the
/// representative model's max-output-token limit so the dashboard can show
/// (and edit) it. Without an override, the value is the catalog
/// `max_output_tokens` of the provider's default/first model; setting a
/// `max_tokens` override changes the headline value the dashboard renders.
#[tokio::test(flavor = "multi_thread")]
async fn provider_max_output_tokens_reflects_catalog_then_override() {
    let h = boot();

    // The baseline seeds openai → gpt-4o-mini with max_output_tokens 16_384
    // and no override, so the headline value is the catalog default.
    let find_openai = |body: &serde_json::Value| -> serde_json::Value {
        body["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"].as_str() == Some("openai"))
            .cloned()
            .expect("openai provider present in baseline catalog")
    };

    let (status, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    let openai = find_openai(&body);
    assert_eq!(
        openai["max_output_tokens"].as_u64(),
        Some(16_384),
        "list should expose the catalog max_output_tokens before any override: {openai}"
    );

    // The single-provider detail endpoint exposes the same value.
    let (status, detail) = json_request(&h, Method::GET, "/api/providers/openai", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["max_output_tokens"].as_u64(), Some(16_384));

    // Set a max_tokens override on the representative model.
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        "/api/models/overrides/openai:gpt-4o-mini",
        Some(serde_json::json!({ "max_tokens": 8_000 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The headline value now reflects the override on both endpoints.
    let (status, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    let openai = find_openai(&body);
    assert_eq!(
        openai["max_output_tokens"].as_u64(),
        Some(8_000),
        "list should reflect the max_tokens override after PUT: {openai}"
    );

    let (status, detail) = json_request(&h, Method::GET, "/api/providers/openai", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["max_output_tokens"].as_u64(), Some(8_000));

    // Clearing the override reverts the headline to the catalog default.
    let (status, _body) = json_request(
        &h,
        Method::DELETE,
        "/api/models/overrides/openai:gpt-4o-mini",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    let openai = find_openai(&body);
    assert_eq!(
        openai["max_output_tokens"].as_u64(),
        Some(16_384),
        "list should revert to catalog default after the override is deleted: {openai}"
    );
}

// ---------------------------------------------------------------------------
// POST /api/providers/{name}/test — only verify unknown-provider 404
// (the success branch performs outbound HTTP/CLI probes — see file header).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_provider_unknown_returns_404() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/providers/__no_such_provider__/test",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// PUT /api/providers/{name}/url — input validation
// (value-side path persists into config.toml under the temp-dir home,
// so it stays inside the harness sandbox.)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_url_rejects_missing_base_url() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/openai/url",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_url_rejects_invalid_scheme() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/openai/url",
        Some(serde_json::json!({ "base_url": "ftp://example.com" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_url_rejects_invalid_proxy_scheme() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/openai/url",
        Some(serde_json::json!({
            "base_url": "https://api.openai.com/v1",
            "proxy_url": "gopher://nope",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// POST /api/providers/{name}/default
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn set_default_provider_unknown_returns_404() {
    let h = boot();
    let (status, _body) = json_request(
        &h,
        Method::POST,
        "/api/providers/__no_such_provider__/default",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_default_openrouter_rejects_model_missing_from_live_catalog() {
    let h = boot_with_provider(ProviderInfo {
        id: "openrouter".to_string(),
        display_name: "OpenRouter".to_string(),
        api_key_env: "OPENROUTER_API_KEY".to_string(),
        base_url: "https://openrouter.ai/api/v1".to_string(),
        key_required: true,
        auth_status: AuthStatus::ValidatedKey,
        ..ProviderInfo::default()
    });
    let live_model = ModelCatalogEntry {
        id: "openrouter/acme/current:free".to_string(),
        display_name: "Current Free".to_string(),
        provider: "openrouter".to_string(),
        tier: ModelTier::Balanced,
        context_window: 32_768,
        max_output_tokens: 4_096,
        supports_streaming: true,
        ..ModelCatalogEntry::default()
    };
    h._state.kernel.model_catalog_update(&mut |catalog| {
        catalog.reconcile_live_provider_models(
            "openrouter",
            vec!["acme/current:free".to_string()],
            vec![live_model.clone()],
        );
    });

    let (list_status, list_body) =
        json_request(&h, Method::GET, "/api/models?provider=openrouter", None).await;
    assert_eq!(list_status, StatusCode::OK);
    let listed_ids: Vec<&str> = list_body["models"]
        .as_array()
        .expect("models array")
        .iter()
        .filter_map(|model| model["id"].as_str())
        .collect();
    assert!(listed_ids.contains(&"openrouter/acme/current:free"));
    assert!(!listed_ids.contains(&"openrouter/qwen/deprecated:free"));

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/openrouter/default",
        Some(serde_json::json!({
            "model": "openrouter/qwen/deprecated:free"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.to_string().contains("not available"),
        "rejection should explain that the model is absent from the live list: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_default_openrouter_uses_free_snapshot_when_live_refresh_fails() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    librefang_api::openrouter_catalog::clear_refresh_attempts(&server.uri());
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;
    let h = boot_with_provider(ProviderInfo {
        id: "openrouter".to_string(),
        display_name: "OpenRouter".to_string(),
        api_key_env: "OPENROUTER_API_KEY".to_string(),
        base_url: String::new(),
        key_required: true,
        auth_status: AuthStatus::ValidatedKey,
        ..ProviderInfo::default()
    });
    let snapshot_model = ModelCatalogEntry {
        id: "openrouter/acme/snapshot:free".to_string(),
        display_name: "Snapshot Free".to_string(),
        provider: "openrouter".to_string(),
        tier: ModelTier::Balanced,
        context_window: 32_768,
        max_output_tokens: 4_096,
        supports_streaming: true,
        ..ModelCatalogEntry::default()
    };
    h._state.kernel.model_catalog_update(&mut |catalog| {
        catalog.set_provider_url("openrouter", &server.uri());
        catalog.reconcile_live_provider_models(
            "openrouter",
            vec!["acme/snapshot:free".to_string()],
            vec![snapshot_model.clone()],
        );
        catalog.clear_provider_available_models("openrouter");
    });

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/openrouter/default",
        Some(serde_json::json!({})),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["model"], "openrouter/acme/snapshot:free");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_default_non_openrouter_accepts_model_missing_from_probe_snapshot() {
    let h = boot();
    h._state.kernel.model_catalog_update(&mut |catalog| {
        catalog.set_provider_available_models("openai", vec!["gpt-4o-mini".to_string()]);
    });

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/openai/default",
        Some(serde_json::json!({
            "model": "ft:gpt-4o:new-after-startup"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_openrouter_refreshes_live_models() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    librefang_api::openrouter_catalog::clear_refresh_attempts(&server.uri());
    Mock::given(method("GET"))
        .and(path("/key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"label": "test"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "id": "acme/live:free",
                "name": "Live Free",
                "context_length": 65536,
                "pricing": {"prompt": "0", "completion": "0"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let h = boot_with_provider(ProviderInfo {
        id: "openrouter".to_string(),
        display_name: "OpenRouter".to_string(),
        api_key_env: String::new(),
        base_url: server.uri(),
        key_required: false,
        auth_status: AuthStatus::Configured,
        ..ProviderInfo::default()
    });

    let (status, body) =
        json_request(&h, Method::POST, "/api/providers/openrouter/test", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["models_refreshed"], 1);

    let catalog = h._state.kernel.model_catalog_ref().load();
    assert!(catalog.has_live_provider_models("openrouter"));
    assert!(catalog
        .find_model_for_provider("openrouter", "openrouter/acme/live:free")
        .is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_first_openrouter_key_uses_free_snapshot_when_live_models_are_rate_limited() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const OPENROUTER_TEST_ENV: &str = "LIBREFANG_TEST_OPENROUTER_KEY_6384";
    const CURRENT_TEST_ENV: &str = "LIBREFANG_TEST_CURRENT_KEY_6384";

    librefang_api::secrets_env::remove_env_var_guarded(CURRENT_TEST_ENV).await;
    librefang_api::secrets_env::remove_env_var_guarded("OPENAI_API_KEY").await;

    let server = MockServer::start().await;
    librefang_api::openrouter_catalog::clear_refresh_attempts(&server.uri());
    Mock::given(method("GET"))
        .and(path("/key"))
        .and(header("authorization", "Bearer test-openrouter-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"label": "test"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;

    let provider = ProviderInfo {
        id: "openrouter".to_string(),
        display_name: "OpenRouter".to_string(),
        api_key_env: OPENROUTER_TEST_ENV.to_string(),
        base_url: server.uri(),
        key_required: true,
        auth_status: AuthStatus::Missing,
        ..ProviderInfo::default()
    };
    let snapshot_model = ModelCatalogEntry {
        id: "openrouter/acme/snapshot:free".to_string(),
        display_name: "Snapshot Free".to_string(),
        provider: "openrouter".to_string(),
        tier: ModelTier::Balanced,
        context_window: 32_768,
        max_output_tokens: 4_096,
        supports_streaming: true,
        ..ModelCatalogEntry::default()
    };
    let test = TestAppState::with_builder(
        MockKernelBuilder::new()
            .with_config(|cfg| {
                cfg.default_model = librefang_types::config::DefaultModelConfig {
                    provider: "openai".to_string(),
                    model: "gpt-4o-mini".to_string(),
                    api_key_env: CURRENT_TEST_ENV.to_string(),
                    ..Default::default()
                };
            })
            .with_catalog_seed((vec![provider], vec![snapshot_model])),
    );
    let state = test.state.clone();
    let h = Harness {
        app: Router::new()
            .nest("/api", routes::providers::router())
            .with_state(state.clone()),
        _state: state,
        _test: test,
    };

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/openrouter/key",
        Some(serde_json::json!({"key": "test-openrouter-key"})),
    )
    .await;
    librefang_api::secrets_env::remove_env_var_guarded(OPENROUTER_TEST_ENV).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["switched_default"], true);
    let default_override = h
        ._state
        .kernel
        .default_model_override_ref()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .expect("default override");
    assert_eq!(default_override.provider, "openrouter");
    assert_eq!(default_override.model, "openrouter/acme/snapshot:free");
}

#[tokio::test(flavor = "multi_thread")]
async fn resaving_openrouter_key_reports_same_provider_model_migration_separately() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const OPENROUTER_TEST_ENV: &str = "LIBREFANG_TEST_OPENROUTER_KEY_6384_RESAVE";

    let server = MockServer::start().await;
    librefang_api::openrouter_catalog::clear_refresh_attempts(&server.uri());
    Mock::given(method("GET"))
        .and(path("/key"))
        .and(header("authorization", "Bearer test-openrouter-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": {"label": "test"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "id": "acme/replacement:free",
                "name": "Replacement Free",
                "context_length": 65536,
                "pricing": {"prompt": "0", "completion": "0"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider = ProviderInfo {
        id: "openrouter".to_string(),
        display_name: "OpenRouter".to_string(),
        api_key_env: OPENROUTER_TEST_ENV.to_string(),
        base_url: server.uri(),
        key_required: true,
        auth_status: AuthStatus::Missing,
        ..ProviderInfo::default()
    };
    let stale_model = ModelCatalogEntry {
        id: "openrouter/acme/old:free".to_string(),
        display_name: "Old Free".to_string(),
        provider: "openrouter".to_string(),
        tier: ModelTier::Balanced,
        context_window: 32_768,
        max_output_tokens: 4_096,
        supports_streaming: true,
        ..ModelCatalogEntry::default()
    };
    let test = TestAppState::with_builder(
        MockKernelBuilder::new()
            .with_config(|cfg| {
                cfg.default_model = librefang_types::config::DefaultModelConfig {
                    provider: "openrouter".to_string(),
                    model: "openrouter/acme/old:free".to_string(),
                    api_key_env: OPENROUTER_TEST_ENV.to_string(),
                    ..Default::default()
                };
            })
            .with_catalog_seed((vec![provider], vec![stale_model])),
    );
    let state = test.state.clone();
    let h = Harness {
        app: Router::new()
            .nest("/api", routes::providers::router())
            .with_state(state.clone()),
        _state: state,
        _test: test,
    };
    {
        let mut guard = h
            ._state
            .kernel
            .default_model_override_ref()
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *guard = Some(librefang_types::config::DefaultModelConfig {
            provider: "openrouter".to_string(),
            model: "openrouter/acme/old:free".to_string(),
            api_key_env: OPENROUTER_TEST_ENV.to_string(),
            ..Default::default()
        });
    }

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/openrouter/key",
        Some(serde_json::json!({"key": "test-openrouter-key"})),
    )
    .await;
    librefang_api::secrets_env::remove_env_var_guarded(OPENROUTER_TEST_ENV).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_ne!(body["switched_default"], true, "body: {body}");
    assert_eq!(body["model_migrated"], true, "body: {body}");
    assert_eq!(body["old_model"], "openrouter/acme/old:free");
    assert_eq!(body["new_model"], "openrouter/acme/replacement:free");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("default model migrated")),
        "body: {body}"
    );

    let default_override = h
        ._state
        .kernel
        .default_model_override_ref()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .expect("default override");
    assert_eq!(default_override.provider, "openrouter");
    assert_eq!(default_override.model, "openrouter/acme/replacement:free");
}

#[tokio::test(flavor = "multi_thread")]
async fn first_openrouter_model_request_populates_live_cache() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    librefang_api::openrouter_catalog::clear_refresh_attempts(&server.uri());
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "id": "acme/lazy:free",
                "name": "Lazy Free",
                "context_length": 65536,
                "pricing": {"prompt": "0", "completion": "0"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let h = boot_with_provider(ProviderInfo {
        id: "openrouter".to_string(),
        display_name: "OpenRouter".to_string(),
        api_key_env: "OPENROUTER_API_KEY".to_string(),
        base_url: server.uri(),
        key_required: true,
        auth_status: AuthStatus::Configured,
        ..ProviderInfo::default()
    });

    let (first_status, first_body) =
        json_request(&h, Method::GET, "/api/models?provider=openrouter", None).await;
    assert_eq!(first_status, StatusCode::OK);
    assert!(first_body["models"]
        .as_array()
        .expect("models array")
        .iter()
        .any(|model| model["id"] == "openrouter/acme/lazy:free"));

    let (second_status, second_body) =
        json_request(&h, Method::GET, "/api/models?provider=openrouter", None).await;
    assert_eq!(second_status, StatusCode::OK);
    assert!(second_body["models"]
        .as_array()
        .expect("models array")
        .iter()
        .any(|model| model["id"] == "openrouter/acme/lazy:free"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_openrouter_model_populates_live_cache_before_lookup() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    librefang_api::openrouter_catalog::clear_refresh_attempts(&server.uri());
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "id": "acme/lazy-detail:free",
                "name": "Lazy Detail Free",
                "context_length": 65536,
                "pricing": {"prompt": "0", "completion": "0"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let h = boot_with_provider(ProviderInfo {
        id: "openrouter".to_string(),
        display_name: "OpenRouter".to_string(),
        api_key_env: "OPENROUTER_API_KEY".to_string(),
        base_url: server.uri(),
        key_required: true,
        auth_status: AuthStatus::Configured,
        ..ProviderInfo::default()
    });

    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/models/openrouter/acme/lazy-detail:free",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["id"], "openrouter/acme/lazy-detail:free");
    assert!(h
        ._state
        .kernel
        .model_catalog_ref()
        .load()
        .has_live_provider_models("openrouter"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unconfigured_openrouter_model_request_does_not_fetch_live_catalog() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    librefang_api::openrouter_catalog::clear_refresh_attempts(&server.uri());
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "id": "acme/unexpected:free",
                "name": "Unexpected Free",
                "context_length": 65536,
                "pricing": {"prompt": "0", "completion": "0"}
            }]
        })))
        .expect(0)
        .mount(&server)
        .await;
    let h = boot_with_provider(ProviderInfo {
        id: "openrouter".to_string(),
        display_name: "OpenRouter".to_string(),
        api_key_env: "OPENROUTER_API_KEY".to_string(),
        base_url: server.uri(),
        key_required: true,
        auth_status: AuthStatus::Missing,
        ..ProviderInfo::default()
    });

    for path in [
        "/api/models?provider=openrouter",
        "/api/providers",
        "/api/providers/openrouter",
    ] {
        let (status, _) = json_request(&h, Method::GET, path, None).await;
        assert_eq!(status, StatusCode::OK, "unexpected status for {path}");
    }
    assert!(!h
        ._state
        .kernel
        .model_catalog_ref()
        .load()
        .has_live_provider_models("openrouter"));
}

// ---------------------------------------------------------------------------
// GET /api/catalog/status — purely reads filesystem state (none in tempdir).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn catalog_status_returns_last_sync_field() {
    let h = boot();
    let (status, body) = json_request(&h, Method::GET, "/api/catalog/status", None).await;
    assert_eq!(status, StatusCode::OK);
    // Field is always present; value may be null when no sync has run.
    assert!(
        body.get("last_sync").is_some(),
        "catalog status should always include 'last_sync' key, got {body}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/providers/github-copilot/oauth/poll/{poll_id} — unknown id branch
// (the start endpoint hits GitHub; we only verify the lookup-failure path.)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn copilot_oauth_poll_unknown_id_returns_404() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/providers/github-copilot/oauth/poll/this-poll-id-does-not-exist",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["status"].as_str().unwrap(), "not_found");
}

// ---------------------------------------------------------------------------
// DELETE /api/providers/{name}/key — regression coverage for #4803.
//
// Pre-fix, pressing "remove key" on a CLI or local-HTTP provider suppressed
// it in `suppressed_providers.json` but `detect_auth` ignored suppression
// for those branches and re-promoted the provider to Configured /
// NotRequired on the same call, so the provider never left the configured
// grid. These tests boot a seeded catalog, hit the route, and assert the
// catalog flips `auth_status` to Missing — the state the dashboard filter
// (`isProviderAvailable`) treats as unconfigured.
// ---------------------------------------------------------------------------

use librefang_types::model_catalog::{
    AuthStatus, Modality, ModelCatalogEntry, ModelTier, ProviderInfo, ReasoningEchoPolicy,
};

/// Boot a harness seeded with a single named provider in the given
/// initial `auth_status`. Lets each test stage the "configured" state
/// the pre-fix bug failed to leave.
///
/// Intentionally single-provider. Multi-provider scenarios (e.g.
/// "suppressing A does not affect B") should build a custom seed via
/// `MockKernelBuilder::with_catalog_seed` rather than extending this
/// helper — keeping it 1-to-1 with the test intent makes the asserts
/// easy to read.
fn boot_with_provider(provider: ProviderInfo) -> Harness {
    let id = provider.id.clone();
    let model = ModelCatalogEntry {
        id: format!("{id}-test-model"),
        display_name: format!("{id} test model"),
        provider: id,
        tier: ModelTier::Custom,
        modality: Modality::default(),
        context_window: 8_192,
        max_output_tokens: 2_048,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        pricing_known: true,
        limits_known: true,
        image_input_cost_per_m: None,
        image_output_cost_per_m: None,
        supports_tools: false,
        supports_vision: false,
        supports_streaming: false,
        supports_thinking: false,
        aliases: Vec::new(),
        reasoning_echo_policy: ReasoningEchoPolicy::default(),
    };
    let test = TestAppState::with_builder(
        MockKernelBuilder::new()
            .with_config(|cfg| {
                cfg.default_model = librefang_types::config::DefaultModelConfig {
                    provider: "openai".to_string(),
                    model: "gpt-4o-mini".to_string(),
                    api_key_env: "OPENAI_API_KEY".to_string(),
                    base_url: None,
                    message_timeout_secs: 300,
                    extra_params: std::collections::BTreeMap::new(),
                    cli_profile_dirs: Vec::new(),
                };
            })
            .with_catalog_seed((vec![provider], vec![model])),
    );

    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::providers::router())
        .with_state(state.clone());

    Harness {
        app,
        _state: state,
        _test: test,
    }
}

fn find_provider<'a>(body: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    body["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .find(|p| p["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("provider '{id}' missing from /api/providers"))
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_provider_key_flips_cli_provider_to_missing() {
    // claude-code is a CLI provider (`is_cli_provider("claude-code") = true`,
    // `key_required = false`, `api_key_env = ""`). Pre-fix `detect_auth`
    // re-set its status from the cli_provider_available probe, leaving the
    // provider in the configured grid no matter what the dashboard did.
    let h = boot_with_provider(ProviderInfo {
        id: "claude-code".to_string(),
        display_name: "Claude Code".to_string(),
        api_key_env: String::new(),
        base_url: String::new(),
        key_required: false,
        auth_status: AuthStatus::Configured,
        model_count: 1,
        ..ProviderInfo::default()
    });

    let (status, _) =
        json_request(&h, Method::DELETE, "/api/providers/claude-code/key", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    let claude = find_provider(&body, "claude-code");
    assert_eq!(
        claude["auth_status"].as_str(),
        Some("missing"),
        "suppressed CLI provider must report `missing` so the dashboard moves it out of the configured grid; got {claude}",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_provider_key_flips_local_provider_to_missing() {
    // ollama is a local HTTP provider (`is_local_provider("ollama") = true`,
    // `key_required = false`). Pre-fix `detect_auth` re-promoted it from
    // Missing to NotRequired on the same call that suppressed it.
    let h = boot_with_provider(ProviderInfo {
        id: "ollama".to_string(),
        display_name: "Ollama".to_string(),
        api_key_env: "OLLAMA_API_KEY".to_string(),
        base_url: "http://127.0.0.1:11434".to_string(),
        key_required: false,
        auth_status: AuthStatus::NotRequired,
        model_count: 1,
        ..ProviderInfo::default()
    });

    let (status, _) = json_request(&h, Method::DELETE, "/api/providers/ollama/key", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    let ollama = find_provider(&body, "ollama");
    assert_eq!(
        ollama["auth_status"].as_str(),
        Some("missing"),
        "suppressed local provider must report `missing`; got {ollama}",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_url_unsuppresses_after_delete() {
    // The re-enable counterpart of the above: after suppressing a local
    // provider, pointing it at a new URL must un-suppress so it appears
    // in the configured grid again. Without `set_provider_url` clearing
    // the suppression flag (#4803), the local provider would stay
    // Missing forever after the user removed and re-configured it.
    //
    // We assert against the on-disk `suppressed_providers.json` rather
    // than `/api/providers` because the list endpoint additionally
    // overrides `auth_status` to "missing" when a fresh probe finds the
    // local port closed — that branch fires here (nothing is listening
    // in the test process), masking the un-suppression we want to
    // verify. The file is the persistence layer that survives restarts,
    // so it is the right surface for this regression.
    let h = boot_with_provider(ProviderInfo {
        id: "ollama".to_string(),
        display_name: "Ollama".to_string(),
        api_key_env: "OLLAMA_API_KEY".to_string(),
        base_url: "http://127.0.0.1:11434".to_string(),
        key_required: false,
        auth_status: AuthStatus::NotRequired,
        model_count: 1,
        ..ProviderInfo::default()
    });

    let suppressed_path = h
        ._state
        .kernel
        .home_dir()
        .join("data")
        .join("suppressed_providers.json");

    // Suppress via DELETE first to set up the regression scenario.
    let (status, _) = json_request(&h, Method::DELETE, "/api/providers/ollama/key", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let suppressed_after_delete: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(&suppressed_path).unwrap()).unwrap();
    assert!(
        suppressed_after_delete.iter().any(|s| s == "ollama"),
        "DELETE should add ollama to suppressed_providers.json; got {suppressed_after_delete:?}",
    );

    // PUT a new URL — this must un-suppress (#4803). The probe inside
    // the handler fires against the new URL; in this test environment
    // nothing is listening so the probe fails, but that does not block
    // the suppression flip.
    let (status, _) = json_request(
        &h,
        Method::PUT,
        "/api/providers/ollama/url",
        Some(serde_json::json!({ "base_url": "http://127.0.0.1:11999" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // `save_suppressed` removes the file when the set is empty.
    let still_suppressed: Option<Vec<String>> = std::fs::read_to_string(&suppressed_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let any_suppressed_after_put = still_suppressed
        .as_ref()
        .map(|v| v.iter().any(|s| s == "ollama"))
        .unwrap_or(false);
    assert!(
        !any_suppressed_after_put,
        "PUT /api/providers/ollama/url must drop ollama from the suppressed list; on-disk content: {still_suppressed:?}",
    );
}

// ---------------------------------------------------------------------------
// POST /api/providers/{name}/enable — explicit re-enable for suppressed
// providers, the CLI-shape counterpart to `set_provider_url` un-suppression
// (#4803 follow-up). CLI providers (`claude-code`, `codex-cli`, …) have no
// key or URL to set, so they can only leave the suppressed bucket via this
// endpoint. The tests assert on the on-disk suppression file rather than
// `/api/providers` for the same reason `set_provider_url_unsuppresses_after_delete`
// does: the list endpoint's local-probe override would mask the flip for
// the ollama row, and the on-disk file is the persistence layer that
// survives restart anyway.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn enable_provider_unsuppresses_cli_provider() {
    // claude-code can only escape suppression via this endpoint: no key to
    // POST, no URL to PUT. Pre-fix the user had to hand-edit
    // suppressed_providers.json.
    let h = boot_with_provider(ProviderInfo {
        id: "claude-code".to_string(),
        display_name: "Claude Code".to_string(),
        api_key_env: String::new(),
        base_url: String::new(),
        key_required: false,
        auth_status: AuthStatus::Configured,
        model_count: 1,
        ..ProviderInfo::default()
    });

    let suppressed_path = h
        ._state
        .kernel
        .home_dir()
        .join("data")
        .join("suppressed_providers.json");

    // Suppress first — same setup as `delete_provider_key_flips_cli_provider_to_missing`.
    let (status, _) =
        json_request(&h, Method::DELETE, "/api/providers/claude-code/key", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let suppressed_after_delete: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(&suppressed_path).unwrap()).unwrap();
    assert!(
        suppressed_after_delete.iter().any(|s| s == "claude-code"),
        "DELETE should add claude-code to suppressed_providers.json; got {suppressed_after_delete:?}",
    );

    // Re-enable via the new endpoint.
    let (status, body) =
        json_request(&h, Method::POST, "/api/providers/claude-code/enable", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"].as_str(), Some("enabled"));
    assert_eq!(body["provider"].as_str(), Some("claude-code"));

    let still_suppressed: Option<Vec<String>> = std::fs::read_to_string(&suppressed_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let any_suppressed_after_enable = still_suppressed
        .as_ref()
        .map(|v| v.iter().any(|s| s == "claude-code"))
        .unwrap_or(false);
    assert!(
        !any_suppressed_after_enable,
        "POST /api/providers/claude-code/enable must drop claude-code from the suppressed list; on-disk content: {still_suppressed:?}",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_provider_unsuppresses_local_provider() {
    // ollama already has the `set_provider_url` un-suppress path; this
    // covers the "user wants to re-enable without changing the URL"
    // shortcut, which is the natural one-click re-enable from a
    // dashboard list of suppressed providers.
    let h = boot_with_provider(ProviderInfo {
        id: "ollama".to_string(),
        display_name: "Ollama".to_string(),
        api_key_env: "OLLAMA_API_KEY".to_string(),
        base_url: "http://127.0.0.1:11434".to_string(),
        key_required: false,
        auth_status: AuthStatus::NotRequired,
        model_count: 1,
        ..ProviderInfo::default()
    });

    let suppressed_path = h
        ._state
        .kernel
        .home_dir()
        .join("data")
        .join("suppressed_providers.json");

    let (status, _) = json_request(&h, Method::DELETE, "/api/providers/ollama/key", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = json_request(&h, Method::POST, "/api/providers/ollama/enable", None).await;
    assert_eq!(status, StatusCode::OK);

    let still_suppressed: Option<Vec<String>> = std::fs::read_to_string(&suppressed_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let any_suppressed_after_enable = still_suppressed
        .as_ref()
        .map(|v| v.iter().any(|s| s == "ollama"))
        .unwrap_or(false);
    assert!(
        !any_suppressed_after_enable,
        "POST /api/providers/ollama/enable must drop ollama from the suppressed list; on-disk content: {still_suppressed:?}",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_provider_is_idempotent_on_already_enabled_row() {
    // Calling enable on a provider that was never suppressed must not
    // touch the suppressed_providers.json file (the handler skips the
    // disk write when nothing is suppressed) and must return 200. This
    // guards the "dashboard double-clicks Re-enable" UX from spuriously
    // recreating the file every call.
    let h = boot_with_provider(ProviderInfo {
        id: "claude-code".to_string(),
        display_name: "Claude Code".to_string(),
        api_key_env: String::new(),
        base_url: String::new(),
        key_required: false,
        auth_status: AuthStatus::Configured,
        model_count: 1,
        ..ProviderInfo::default()
    });

    let suppressed_path = h
        ._state
        .kernel
        .home_dir()
        .join("data")
        .join("suppressed_providers.json");

    let (status, _) =
        json_request(&h, Method::POST, "/api/providers/claude-code/enable", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !suppressed_path.exists(),
        "idempotent enable on a never-suppressed provider must not create suppressed_providers.json",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_providers_exposes_suppression_state() {
    // Dashboard discriminates "user-suppressed CLI provider" from
    // "missing because never configured" by reading `suppressed: bool`
    // on each provider entry. Pre-fix this flag was not exposed and the
    // dashboard could only guess from `auth_status: "missing"`.
    let h = boot_with_provider(ProviderInfo {
        id: "claude-code".to_string(),
        display_name: "Claude Code".to_string(),
        api_key_env: String::new(),
        base_url: String::new(),
        key_required: false,
        auth_status: AuthStatus::Configured,
        model_count: 1,
        ..ProviderInfo::default()
    });

    let (_, body_before) = json_request(&h, Method::GET, "/api/providers", None).await;
    let claude_before = find_provider(&body_before, "claude-code");
    assert_eq!(
        claude_before["suppressed"].as_bool(),
        Some(false),
        "pristine catalog must report `suppressed: false`; got {claude_before}",
    );

    let (status, _) =
        json_request(&h, Method::DELETE, "/api/providers/claude-code/key", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, body_after) = json_request(&h, Method::GET, "/api/providers", None).await;
    let claude_after = find_provider(&body_after, "claude-code");
    assert_eq!(
        claude_after["suppressed"].as_bool(),
        Some(true),
        "after DELETE /key, suppressed must flip to true; got {claude_after}",
    );

    let (status, _) =
        json_request(&h, Method::POST, "/api/providers/claude-code/enable", None).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body_final) = json_request(&h, Method::GET, "/api/providers", None).await;
    let claude_final = find_provider(&body_final, "claude-code");
    assert_eq!(
        claude_final["suppressed"].as_bool(),
        Some(false),
        "after POST /enable, suppressed must flip back to false; got {claude_final}",
    );
}

// ---------------------------------------------------------------------------
// POST /api/providers/{name}/default — regression coverage for #5116.
//
// Pre-fix, `persist_default_model` read config.toml with
// `unwrap_or_default()` and then rewrote the file from a fresh TOML tree
// containing only `[default_model]`, destroying every operator-authored
// section (e.g. `[email]`, `[telegram]`, `[proxy]`) on rewrite. The
// regression here is the data-loss path itself — pre-seed config.toml
// with a sibling section, switch the default provider through the route,
// then assert the sibling section survives the rewrite.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn set_default_provider_preserves_other_config_sections() {
    let h = boot();

    // Seed config.toml with `[default_model]` + sibling `[email]` and
    // `[proxy]` sections that the pre-fix `unwrap_or_default()` / rewrite
    // path would have silently wiped. The home dir is the kernel's
    // tempdir, so writing here doesn't escape the harness sandbox.
    let config_path = h._state.kernel.home_dir().join("config.toml");
    let seeded = r#"# Seeded by integration test for #5116

[default_model]
provider = "openai"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"

[email]
smtp_host = "smtp.example.com"
smtp_port = 587
username = "alice@example.com"

[proxy]
http = "http://127.0.0.1:8118"
"#;
    std::fs::write(&config_path, seeded).expect("seed config.toml");

    // Switch default provider via the route. The catalog seeded by `boot()`
    // only has `openai`, but switching from openai -> openai still exercises
    // the same persist_default_model path that wipes other sections.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/openai/default",
        Some(serde_json::json!({ "model": "gpt-4o-mini" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected body: {body}");
    assert_eq!(
        body["persisted"].as_bool(),
        Some(true),
        "config.toml should have been persisted; got {body}",
    );

    // Reload and confirm both sibling sections survived the rewrite.
    let after = std::fs::read_to_string(&config_path).expect("read config.toml back");
    let parsed: toml::Value = toml::from_str(&after).expect("post-write config.toml parses");

    let dm = parsed
        .get("default_model")
        .and_then(|v| v.as_table())
        .expect("default_model section must still exist");
    assert_eq!(
        dm.get("provider").and_then(|v| v.as_str()),
        Some("openai"),
        "default_model.provider should reflect the PATCH; full toml:\n{after}",
    );

    let email = parsed
        .get("email")
        .and_then(|v| v.as_table())
        .unwrap_or_else(|| {
            panic!("[email] section was wiped — regression of #5116; full toml:\n{after}")
        });
    assert_eq!(
        email.get("smtp_host").and_then(|v| v.as_str()),
        Some("smtp.example.com"),
        "[email].smtp_host must survive default-model rewrite; full toml:\n{after}",
    );
    assert_eq!(
        email.get("smtp_port").and_then(|v| v.as_integer()),
        Some(587),
        "[email].smtp_port must survive default-model rewrite; full toml:\n{after}",
    );

    let proxy = parsed
        .get("proxy")
        .and_then(|v| v.as_table())
        .unwrap_or_else(|| {
            panic!("[proxy] section was wiped — regression of #5116; full toml:\n{after}")
        });
    assert_eq!(
        proxy.get("http").and_then(|v| v.as_str()),
        Some("http://127.0.0.1:8118"),
        "[proxy].http must survive default-model rewrite; full toml:\n{after}",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_default_provider_when_config_toml_absent_creates_it_with_default_model() {
    // The companion happy path for the read-then-write contract: when
    // config.toml is missing entirely (fresh daemon, no operator config),
    // the route MUST create it and seed `[default_model]`. The bug fix
    // discriminates `NotFound` from other read errors — make sure the
    // NotFound branch still produces a usable file.
    let h = boot();
    let config_path = h._state.kernel.home_dir().join("config.toml");
    // Sanity: the boot helper does not pre-write config.toml.
    assert!(
        !config_path.exists(),
        "boot() should not pre-write config.toml; harness assumption broken",
    );

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/openai/default",
        Some(serde_json::json!({ "model": "gpt-4o-mini" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected body: {body}");
    assert_eq!(body["persisted"].as_bool(), Some(true));

    let after = std::fs::read_to_string(&config_path).expect("config.toml created");
    let parsed: toml::Value = toml::from_str(&after).expect("new config.toml parses");
    let dm = parsed
        .get("default_model")
        .and_then(|v| v.as_table())
        .expect("[default_model] missing from freshly-created config.toml");
    assert_eq!(dm.get("provider").and_then(|v| v.as_str()), Some("openai"));
    assert_eq!(
        dm.get("model").and_then(|v| v.as_str()),
        Some("gpt-4o-mini")
    );
}

/// #5137: `set_default_provider` now surfaces a per-agent partial-failure
/// list from `sync_default_model_agents` and returns 207 Multi-Status when
/// any agent could not be migrated. On the happy path (every eligible
/// agent migrates cleanly) it MUST still return 200 OK and MUST NOT
/// include a `sync_failures` key — proving the new partial-failure branch
/// is correctly gated and did not regress the success contract.
#[tokio::test(flavor = "multi_thread")]
async fn set_default_provider_happy_path_has_no_sync_failures_and_is_200() {
    let h = boot();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/openai/default",
        Some(serde_json::json!({ "model": "gpt-4o-mini" })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "happy-path provider switch must stay 200, not 207; body: {body}"
    );
    assert!(
        body.get("sync_failures").is_none(),
        "no sync_failures key when every eligible agent migrated cleanly (#5137); body: {body}"
    );
}

// ---------------------------------------------------------------------------
// POST / DELETE /api/providers/{name}/key — path-name validation.
//
// Refs `docs/issues/set-provider-key-arbitrary-names.md`. Pre-fix, an admin
// could plant arbitrary env vars (`STRIPE_API_KEY`, …) into the live
// `std::env` + persisted `secrets.env`, or submit `name = "a".repeat(N)` to
// plant a giant env var. The handlers now shape-check `name` against
// `^[a-z0-9-]{1,64}$` BEFORE touching the catalog or env, and shape-check
// the derived env var against `^[A-Z][A-Z0-9_]{0,63}_API_KEY$` when the
// provider is not in the catalog.
//
// These tests only exercise the REJECTION paths — the 400 is returned
// before the handler reaches `set_env_var_guarded` / `remove_env_var_guarded`,
// so they do not mutate the process-shared `std::env` (which would violate
// the "no global env mutation" rule documented at the top of this file).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_key_rejects_oversize_name() {
    let h = boot();
    let name = "a".repeat(1000);
    let path = format!("/api/providers/{name}/key");
    let (status, body) = json_request(
        &h,
        Method::POST,
        &path,
        Some(serde_json::json!({ "key": "sk-test" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "1000-char provider name must be rejected by the shape gate; body: {body}"
    );
    // ApiErrorResponse envelope: `{"error": {"message": "..."}, "message": "...", ...}`.
    let err = body["error"]["message"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or_default();
    assert!(
        err.contains("too long") || err.contains("64"),
        "rejection must mention the length cap; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_key_rejects_uppercase_name() {
    // Uppercase is outside `[a-z0-9-]` — also closes the "plant a known
    // third-party env var via a name like `STRIPE`" surface, because the
    // shape gate trips before the derive step.
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/STRIPE/key",
        Some(serde_json::json!({ "key": "sk-test" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "uppercase provider name must be rejected; body: {body}"
    );
    let err = body["error"]["message"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or_default();
    assert!(
        err.contains("invalid characters"),
        "rejection must mention invalid characters; body: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_key_rejects_dotted_name() {
    // `.` is not in `[a-z0-9-]`. Also covers any attempt to smuggle a
    // path-traversal-ish shape into the env-var derivation.
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/ab.cd/key",
        Some(serde_json::json!({ "key": "sk-test" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "dotted provider name must be rejected; body: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_provider_key_rejects_oversize_name() {
    let h = boot();
    let name = "a".repeat(1000);
    let path = format!("/api/providers/{name}/key");
    let (status, body) = json_request(&h, Method::DELETE, &path, None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "1000-char provider name must be rejected on DELETE too; body: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_provider_key_rejects_uppercase_name() {
    let h = boot();
    let (status, body) = json_request(&h, Method::DELETE, "/api/providers/STRIPE/key", None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "uppercase provider name must be rejected on DELETE; body: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_provider_key_rejects_dotted_name() {
    let h = boot();
    let (status, body) = json_request(&h, Method::DELETE, "/api/providers/ab.cd/key", None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "dotted provider name must be rejected on DELETE; body: {body}"
    );
}

// ---------------------------------------------------------------------------
// EveryAPI live-catalog refresh (`everyapi_catalog.rs`)
//
// `librefang models connect everyapi` registers a one-shot snapshot of the
// gateway's models. These tests drive the TTL refresh end-to-end against a
// local mock gateway: `GET {base_url}/models` for the authoritative id list
// and the public `GET {origin}/api/pricing` for context window + billing
// ratios, proving that the `/v1`-stripped pricing origin is derived correctly
// and that metadata the gateway publishes nowhere survives the merge.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn everyapi_refresh_merges_both_gateway_endpoints() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let base_url = format!("{}/v1", server.uri());
    // The retry window is keyed by the provider `base_url` (the `/v1` form),
    // not the bare server origin — sequential tests reuse ephemeral ports.
    librefang_api::everyapi_catalog::clear_refresh_attempts(&base_url);

    // Distinct env var per test: `std::env::set_var` is process-global and
    // nextest runs test threads concurrently.
    let key_env = "LIBREFANG_TEST_EVERYAPI_RELAY_KEY_MERGE";
    std::env::set_var(key_env, "relay-secret-must-not-leak");

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"id": "claude-sonnet-5", "owned_by": "anthropic", "supported_endpoint_types": ["openai", "anthropic"]},
                {"id": "tts-1", "owned_by": "openai", "supported_endpoint_types": ["audio-speech"]},
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/pricing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": [
                {"model_name": "claude-sonnet-5", "quota_type": 0, "model_ratio": 1.0, "completion_ratio": 5.0, "context_window": 200000, "billing_mode": "per_token"},
                {"model_name": "tts-1", "quota_type": 0, "model_ratio": 7.5, "completion_ratio": 0.0, "context_window": 0, "billing_mode": "per_token"},
                // Published by the pricing feed but absent from `/v1/models`;
                // must never be registered.
                {"model_name": "claude-fable-5", "quota_type": 0, "model_ratio": 5.0, "completion_ratio": 5.0, "context_window": 200000, "billing_mode": "per_token"},
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let h = boot_with_provider(ProviderInfo {
        id: "everyapi".to_string(),
        display_name: "EveryAPI".to_string(),
        api_key_env: key_env.to_string(),
        base_url: base_url.clone(),
        key_required: true,
        auth_status: AuthStatus::Configured,
        ..ProviderInfo::default()
    });

    // Stand in for what `models connect everyapi` wrote: `max_output_tokens`
    // and the capability flags appear on neither gateway endpoint, so they
    // exist only here. `clear_provider_available_models` then rewinds the
    // freshness stamp so the handler sees a stale catalog.
    let registered = ModelCatalogEntry {
        id: "claude-sonnet-5".to_string(),
        display_name: "claude-sonnet-5".to_string(),
        provider: "everyapi".to_string(),
        tier: ModelTier::Balanced,
        modality: Modality::Text,
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_per_m: 2.0,
        output_cost_per_m: 10.0,
        pricing_known: true,
        supports_tools: true,
        supports_vision: true,
        supports_streaming: true,
        supports_thinking: true,
        ..ModelCatalogEntry::default()
    };
    let delisted = ModelCatalogEntry {
        id: "gemini-3-flash".to_string(),
        display_name: "gemini-3-flash".to_string(),
        provider: "everyapi".to_string(),
        tier: ModelTier::Balanced,
        modality: Modality::Text,
        context_window: 1_000_000,
        max_output_tokens: 8_192,
        supports_streaming: true,
        ..ModelCatalogEntry::default()
    };
    h._state.kernel.model_catalog_update(&mut |catalog| {
        catalog.reconcile_live_provider_models(
            "everyapi",
            vec!["claude-sonnet-5".to_string(), "gemini-3-flash".to_string()],
            vec![registered.clone(), delisted.clone()],
        );
        catalog.clear_provider_available_models("everyapi");
    });

    let (status, body) = json_request(&h, Method::GET, "/api/providers/everyapi", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let models = body["models"].as_array().expect("models array");
    let by_id = |id: &str| models.iter().find(|m| m["id"].as_str() == Some(id));

    let sonnet = by_id("claude-sonnet-5").unwrap_or_else(|| panic!("sonnet missing: {body}"));
    // Pricing feed ratios: model_ratio 1 x $2 in, x completion_ratio 5 out.
    assert_eq!(sonnet["input_cost_per_m"], 2.0);
    assert_eq!(sonnet["output_cost_per_m"], 10.0);
    assert_eq!(sonnet["pricing_known"], true);
    assert_eq!(sonnet["context_window"], 200_000);
    // Published by neither endpoint — proof the merge carries it forward
    // instead of letting `reconcile_live_provider_models` delete it.
    assert_eq!(sonnet["max_output_tokens"], 64_000);
    assert_eq!(sonnet["supports_tools"], true);

    // Newly listed by the gateway, non-Text so it needs no token metadata.
    let tts = by_id("tts-1").unwrap_or_else(|| panic!("tts-1 missing: {body}"));
    assert_eq!(tts["modality"], "audio");
    assert_eq!(tts["input_cost_per_m"], 15.0);

    assert!(
        by_id("gemini-3-flash").is_none(),
        "a model the gateway delisted must disappear: {body}"
    );
    assert!(
        by_id("claude-fable-5").is_none(),
        "the pricing feed must never introduce a model id: {body}"
    );

    // Relay key reached the Authorization header only.
    assert!(
        !body.to_string().contains("relay-secret-must-not-leak"),
        "the relay key must never appear in a response payload"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn everyapi_refresh_keeps_the_registered_catalog_when_the_gateway_is_down() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let base_url = format!("{}/v1", server.uri());
    librefang_api::everyapi_catalog::clear_refresh_attempts(&base_url);

    let key_env = "LIBREFANG_TEST_EVERYAPI_RELAY_KEY_DOWN";
    std::env::set_var(key_env, "relay-secret-must-not-leak");

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let h = boot_with_provider(ProviderInfo {
        id: "everyapi".to_string(),
        display_name: "EveryAPI".to_string(),
        api_key_env: key_env.to_string(),
        base_url: base_url.clone(),
        key_required: true,
        auth_status: AuthStatus::Configured,
        ..ProviderInfo::default()
    });
    let registered = ModelCatalogEntry {
        id: "claude-sonnet-5".to_string(),
        display_name: "claude-sonnet-5".to_string(),
        provider: "everyapi".to_string(),
        tier: ModelTier::Balanced,
        modality: Modality::Text,
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_per_m: 2.0,
        output_cost_per_m: 10.0,
        pricing_known: true,
        supports_streaming: true,
        ..ModelCatalogEntry::default()
    };
    h._state.kernel.model_catalog_update(&mut |catalog| {
        catalog.reconcile_live_provider_models(
            "everyapi",
            vec!["claude-sonnet-5".to_string()],
            vec![registered.clone()],
        );
        catalog.clear_provider_available_models("everyapi");
    });

    // A failed refresh is warn-logged, never fatal to the request.
    let (status, body) = json_request(&h, Method::GET, "/api/providers/everyapi", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let models = body["models"].as_array().expect("models array");
    let sonnet = models
        .iter()
        .find(|m| m["id"].as_str() == Some("claude-sonnet-5"))
        .unwrap_or_else(|| panic!("registered catalog must survive: {body}"));
    assert_eq!(sonnet["max_output_tokens"], 64_000);
}

// ---------------------------------------------------------------------------
// Model discovery for custom OpenAI-compatible providers (#6702) and API keys
// for keyless local providers (#6703).
// ---------------------------------------------------------------------------

/// #6702: a custom provider that opted into discovery is probed by
/// `POST /api/providers/{name}/test`, which merges the live `/models` listing
/// into the catalog. Pre-fix the handler gated on the hard-coded local id
/// allowlist, so this provider fell through to the generic reachability check
/// and reported no `discovered_models` at all.
#[tokio::test(flavor = "multi_thread")]
async fn opted_in_custom_provider_is_probed_and_discovers_models() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                { "id": "qwen3-32b" },
                { "id": "llama-3.3-70b" },
            ]
        })))
        .mount(&server)
        .await;

    let h = boot_with_provider(ProviderInfo {
        id: "acme-vllm".to_string(),
        display_name: "ACME vLLM".to_string(),
        api_key_env: "LIBREFANG_TEST_ACME_VLLM_API_KEY".to_string(),
        base_url: server.uri(),
        key_required: false,
        auth_status: AuthStatus::NotRequired,
        discover_models: true,
        ..ProviderInfo::default()
    });

    let (status, body) =
        json_request(&h, Method::POST, "/api/providers/acme-vllm/test", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"].as_str(), Some("ok"), "body: {body}");
    assert_eq!(
        body["discovered_models"].as_u64(),
        Some(2),
        "an opted-in custom provider must report the models its /models endpoint served; body: {body}"
    );

    // The discovered ids reach the catalog, which is what makes them selectable
    // from the Models page rather than a number in a test response.
    let (_, provider) = json_request(&h, Method::GET, "/api/providers/acme-vllm", None).await;
    let ids: Vec<&str> = provider["models"]
        .as_array()
        .expect("models array")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(
        ids.contains(&"qwen3-32b"),
        "discovered model must be merged into the catalog; got {ids:?}"
    );
}

/// The other half of the #6702 contract: a custom provider that did NOT opt in
/// keeps the pre-change behaviour exactly — reachability only, no discovery,
/// no catalog mutation. This is the guard on "an existing install sees no
/// difference".
#[tokio::test(flavor = "multi_thread")]
async fn custom_provider_without_the_flag_is_not_probed_for_models() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{ "id": "qwen3-32b" }]
        })))
        .mount(&server)
        .await;

    let h = boot_with_provider(ProviderInfo {
        id: "acme-plain".to_string(),
        display_name: "ACME Plain".to_string(),
        api_key_env: "LIBREFANG_TEST_ACME_PLAIN_API_KEY".to_string(),
        base_url: server.uri(),
        key_required: false,
        auth_status: AuthStatus::Configured,
        ..ProviderInfo::default()
    });

    let (status, body) =
        json_request(&h, Method::POST, "/api/providers/acme-plain/test", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.get("discovered_models").is_none(),
        "a provider that never opted in must not enter the discovery path; body: {body}"
    );

    let (_, provider) = json_request(&h, Method::GET, "/api/providers/acme-plain", None).await;
    let ids: Vec<&str> = provider["models"]
        .as_array()
        .expect("models array")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&"qwen3-32b"),
        "no live model should have been merged; got {ids:?}"
    );
}

/// #7775: the models a self-hosted OpenAI-compatible gateway serves must reach `GET /api/models`, which is the list every surface picks a model from.
///
/// The ids belong to the operator, so no checked-in catalogue can ship them — and before this fix `list_models` refreshed a live catalogue for OpenRouter and EveryAPI only, then merely *read* the probe cache to filter static entries.
/// A gateway had nothing static to filter and nothing live to add, so the list came back with only whatever the operator had hand-registered.
#[tokio::test(flavor = "multi_thread")]
async fn gateway_served_models_reach_the_model_list() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                { "id": "sensor-model-generic" },
                { "id": "sensor-model-generic-high" },
            ]
        })))
        .mount(&server)
        .await;

    let h = boot_with_provider(ProviderInfo {
        id: "acme-gateway".to_string(),
        display_name: "ACME Gateway".to_string(),
        api_key_env: "LIBREFANG_TEST_ACME_GATEWAY_API_KEY".to_string(),
        base_url: server.uri(),
        key_required: false,
        auth_status: AuthStatus::NotRequired,
        discover_models: true,
        ..ProviderInfo::default()
    });

    let (status, body) =
        json_request(&h, Method::GET, "/api/models?provider=acme-gateway", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let ids: Vec<&str> = body["models"]
        .as_array()
        .expect("models array")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(
        ids.contains(&"sensor-model-generic") && ids.contains(&"sensor-model-generic-high"),
        "every model the gateway lists must be selectable from /api/models; got {ids:?}"
    );
    // The hand-registered entry is not collateral: the live filter (#3191) runs against the same probe result and must not drop a custom-tier model the operator added themselves.
    assert!(
        ids.contains(&"acme-gateway-test-model"),
        "discovery must not evict an operator-registered model; got {ids:?}"
    );
}

/// The counterpart guard: `/api/models` must not turn into a probe of every configured provider.
/// A provider that never opted into discovery is left alone entirely — no request, not merely no merge — so enabling this on one gateway does not start billing round-trips against the others.
#[tokio::test(flavor = "multi_thread")]
async fn the_model_list_does_not_probe_a_provider_that_never_opted_in() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{ "id": "should-never-be-listed" }]
        })))
        .mount(&server)
        .await;

    let h = boot_with_provider(ProviderInfo {
        id: "acme-quiet".to_string(),
        display_name: "ACME Quiet".to_string(),
        api_key_env: "LIBREFANG_TEST_ACME_QUIET_API_KEY".to_string(),
        base_url: server.uri(),
        key_required: true,
        auth_status: AuthStatus::Configured,
        ..ProviderInfo::default()
    });

    let (status, body) =
        json_request(&h, Method::GET, "/api/models?provider=acme-quiet", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let ids: Vec<&str> = body["models"]
        .as_array()
        .expect("models array")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&"should-never-be-listed"),
        "a provider without the opt-in must not gain live models; got {ids:?}"
    );
    let received = server
        .received_requests()
        .await
        .expect("wiremock records requests");
    assert!(
        received.is_empty(),
        "no listing request should have been sent at all; got {:?}",
        received
            .iter()
            .map(|r| r.url.to_string())
            .collect::<Vec<_>>()
    );
}

/// `PUT /api/providers/{name}/discovery` flips the flag, reports it back on
/// `/api/providers`, and persists it into the provider's own TOML so the
/// opt-in survives a daemon restart.
#[tokio::test(flavor = "multi_thread")]
async fn set_provider_discovery_flips_flag_and_persists_to_the_provider_file() {
    let h = boot_with_provider(ProviderInfo {
        id: "acme-toggle".to_string(),
        display_name: "ACME Toggle".to_string(),
        api_key_env: "LIBREFANG_TEST_ACME_TOGGLE_API_KEY".to_string(),
        base_url: "http://127.0.0.1:59999/v1".to_string(),
        key_required: true,
        auth_status: AuthStatus::Configured,
        ..ProviderInfo::default()
    });

    let (_, before) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(
        find_provider(&before, "acme-toggle")["discover_models"].as_bool(),
        Some(false),
        "discovery is off until the operator opts in; body: {before}"
    );

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/acme-toggle/discovery",
        Some(serde_json::json!({ "discover_models": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["discover_models"].as_bool(), Some(true));

    let (_, after) = json_request(&h, Method::GET, "/api/providers", None).await;
    assert_eq!(
        find_provider(&after, "acme-toggle")["discover_models"].as_bool(),
        Some(true),
        "the list endpoint must report the new setting; body: {after}"
    );

    let provider_file = h
        ._state
        .kernel
        .home_dir()
        .join("providers")
        .join("acme-toggle.toml");
    let persisted = std::fs::read_to_string(&provider_file).unwrap_or_else(|e| {
        panic!(
            "provider file must exist at {}: {e}",
            provider_file.display()
        )
    });
    assert!(
        persisted.contains("discover_models = true"),
        "the opt-in must survive a restart; file content:\n{persisted}"
    );

    // The assertion above is not enough on its own: the file used to be written
    // with `id` + `discover_models` and nothing else, which failed the catalog
    // loader's required fields, so the loader discarded it whole and the flag
    // reverted on every boot (#7776). Reload the way the daemon does at boot and
    // assert the record actually comes back.
    let reloaded = librefang_runtime::model_catalog::ModelCatalog::new_from_dir(
        &h._state.kernel.home_dir().join("providers"),
    );
    let round_tripped = reloaded.get_provider("acme-toggle").unwrap_or_else(|| {
        panic!("the persisted file must load back as a provider; file content:\n{persisted}")
    });
    assert!(
        round_tripped.discover_models,
        "the reloaded catalog must carry the opt-in; file content:\n{persisted}"
    );
    assert_eq!(
        round_tripped.base_url, "http://127.0.0.1:59999/v1",
        "the endpoint has to be written too, or the probe loop has nothing to poll"
    );
    assert_eq!(round_tripped.display_name, "ACME Toggle");
    assert_eq!(
        round_tripped.api_key_env,
        "LIBREFANG_TEST_ACME_TOGGLE_API_KEY"
    );

    // Turning it back off rewrites the same key rather than appending a second one.
    let (status, _) = json_request(
        &h,
        Method::PUT,
        "/api/providers/acme-toggle/discovery",
        Some(serde_json::json!({ "discover_models": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let persisted = std::fs::read_to_string(&provider_file).expect("provider file still readable");
    assert!(persisted.contains("discover_models = false"), "{persisted}");
    assert_eq!(
        persisted.matches("discover_models").count(),
        1,
        "the key is updated in place, not appended; file content:\n{persisted}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_discovery_rejects_unknown_provider() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/no-such-provider/discovery",
        Some(serde_json::json!({ "discover_models": true })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_provider_discovery_rejects_non_boolean_body() {
    let h = boot();
    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/providers/openai/discovery",
        Some(serde_json::json!({ "discover_models": "yes" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
}

/// #6703: a provider that declares `key_required = false` — every built-in
/// local one does — can still be given an API key, and the provider list says
/// so. The dashboard hid its key field on the strength of `key_required`, so a
/// self-hosted vLLM behind auth could not be configured from the UI even though
/// the runtime forwards the key as `Authorization: Bearer` whenever it is set.
#[tokio::test(flavor = "multi_thread")]
async fn keyless_provider_accepts_a_key_and_reports_it_as_present() {
    const KEY_ENV: &str = "LIBREFANG_TEST_VLLM_KEYLESS_6703_API_KEY";

    let h = boot_with_provider(ProviderInfo {
        id: "acme-keyless".to_string(),
        display_name: "ACME Keyless".to_string(),
        api_key_env: KEY_ENV.to_string(),
        base_url: "http://127.0.0.1:59998/v1".to_string(),
        key_required: false,
        auth_status: AuthStatus::NotRequired,
        ..ProviderInfo::default()
    });

    let (_, before) = json_request(&h, Method::GET, "/api/providers", None).await;
    let entry = find_provider(&before, "acme-keyless");
    assert_eq!(entry["key_required"].as_bool(), Some(false));
    assert_eq!(
        entry["key_present"].as_bool(),
        Some(false),
        "no key is stored yet; body: {before}"
    );

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/acme-keyless/key",
        Some(serde_json::json!({ "key": "vllm-secret" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a keyless provider must still accept a key; body: {body}"
    );

    let (_, after) = json_request(&h, Method::GET, "/api/providers", None).await;
    let entry = find_provider(&after, "acme-keyless");
    assert_eq!(
        entry["key_present"].as_bool(),
        Some(true),
        "the dashboard needs this to offer replace/remove for a keyless provider; body: {after}"
    );
    assert_eq!(
        entry["key_required"].as_bool(),
        Some(false),
        "storing a key must not make the key mandatory; body: {after}"
    );

    // This test is the one place in the file that plants an env var; the name
    // is unique to it, and it is removed before returning so no sibling test
    // observes it.
    librefang_api::secrets_env::remove_env_var_guarded(KEY_ENV).await;
}

/// #6702 regression: opting a custom provider into model discovery must not
/// make it *look* unconfigured when its `/models` listing is unreachable.
///
/// Before the guard, every probed provider had `auth_status` forced to
/// `missing` on an unreachable probe. That rule was written when only the four
/// built-in local ids were probed, where it is correct — they need no key, so
/// reachability is the whole availability story. Opting a keyed provider in
/// (#6702) put it on the same path, and a gateway that proxies
/// `/chat/completions` without serving `/models` is an ordinary shape, so a
/// working provider with a valid key would report as needing setup purely
/// because the operator turned discovery on.
#[tokio::test(flavor = "multi_thread")]
async fn discovery_opt_in_does_not_mark_a_keyed_provider_unconfigured() {
    const KEY_ENV: &str = "LIBREFANG_TEST_ACME_GW_6702_API_KEY";

    let h = boot_with_provider(ProviderInfo {
        id: "acme-gw".to_string(),
        display_name: "ACME Gateway".to_string(),
        api_key_env: KEY_ENV.to_string(),
        // Deliberately closed: stands in for a gateway that proxies
        // /chat/completions but serves no /models listing.
        base_url: "http://127.0.0.1:59321/v1".to_string(),
        key_required: true,
        auth_status: AuthStatus::Configured,
        discover_models: true,
        ..ProviderInfo::default()
    });

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/providers/acme-gw/key",
        Some(serde_json::json!({ "key": "gateway-secret" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (_, body) = json_request(&h, Method::GET, "/api/providers", None).await;
    let entry = find_provider(&body, "acme-gw");
    assert_eq!(
        entry["reachable"].as_bool(),
        Some(false),
        "the probe must genuinely have failed, or this test proves nothing; body: {body}"
    );
    assert_eq!(
        entry["auth_status"].as_str(),
        Some("configured"),
        "a keyed provider must keep 'configured' when only its /models probe fails; body: {body}"
    );

    librefang_api::secrets_env::remove_env_var_guarded(KEY_ENV).await;
}
