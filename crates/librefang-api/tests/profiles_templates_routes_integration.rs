//! Integration tests for `/api/profiles` and `/api/templates` sub-routes
//! inside `crates/librefang-api/src/routes/system.rs` (refs #3571 — "~80%
//! of registered HTTP routes have no integration test").
//!
//! These exercise the real `system::router()` via `tower::oneshot`, with a
//! `TestAppState` + `MockKernelBuilder` boot. The auth middleware is not
//! mounted in this slice — same approach as `users_test.rs` — because the
//! profile/template handlers are pure (profiles) or filesystem-bound
//! (templates) and the goal is to catch the "compiles but routes are dead /
//! return wrong shape" class of bug called out in the issue.
//!
//! ### Templates and per-harness home directories
//!
//! `list_agent_templates` / `get_agent_template` / `get_agent_template_toml`
//! resolve `agent-types/` and `workspaces/agents/` against
//! `state.kernel.config_ref().home_dir` (#8112) — NOT the process-wide
//! `LIBREFANG_HOME` env var, which is what an embedder's `KernelConfig` can
//! point somewhere else entirely (crates/librefang-kernel/src/config.rs:686).
//! Each [`boot`] call gets a fresh `MockKernelBuilder` tempdir as its own
//! `home_dir`, so template fixtures are written under *that* harness's own
//! directory rather than into one directory shared by the whole test binary
//! — which also means no cross-test locking is needed: every harness is
//! already isolated.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        // Minimal default model so kernel boot is happy. Same shape as
        // `users_test.rs::boot`.
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::system::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

/// This harness kernel's own `workspaces/agents/` — where [`write_template`]
/// / [`remove_template`] operate, matching what `get_agent_template` etc.
/// actually resolve against (#8112).
fn agents_dir(h: &Harness) -> PathBuf {
    librefang_types::agent_type_store::workspace_agents_dir_in(
        &h.state.kernel.config_ref().home_dir,
    )
}

async fn get(h: &Harness, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

async fn get_json(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let (status, _hdr, bytes) = get(h, path).await;
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

// ---------------------------------------------------------------------------
// /api/profiles — pure handler, no filesystem.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn profiles_list_returns_six_known_profiles() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/profiles").await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().expect("array");
    let mut names: Vec<&str> = arr.iter().map(|v| v["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    // Pin the registered set so a refactor that drops a profile is loud.
    assert_eq!(
        names,
        vec![
            "automation",
            "coding",
            "full",
            "messaging",
            "minimal",
            "research",
        ],
        "profile registration drift: {body}"
    );
    // Each entry must carry a non-empty tools list — the dashboard renders
    // these directly. An empty list would silently break the UI.
    for entry in arr {
        let tools = entry["tools"].as_array().expect("tools array");
        assert!(
            !tools.is_empty(),
            "profile {:?} has no tools",
            entry["name"]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn profiles_get_known_profile_returns_tools() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/profiles/coding").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "coding");
    assert!(
        body["tools"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "coding profile must expose tools: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn profiles_get_unknown_profile_returns_404() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/profiles/no-such-profile").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        body["error"].is_string() || body["error"]["message"].is_string(),
        "404 must carry a structured error payload: {body}"
    );
}

// ---------------------------------------------------------------------------
// /api/templates — filesystem-bound, scoped to each harness's own home_dir.
// ---------------------------------------------------------------------------

struct TemplateFixture {
    dir: PathBuf,
}

impl Drop for TemplateFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The returned guard owns the template directory on disk.
/// Dropping it deletes the fixture, so a caller that discards the return value deletes it immediately and every later read of that template answers 404.
/// `#[must_use]` turns that mistake into a compile error rather than a failure that surfaces in an unrelated assertion.
#[must_use = "bind the guard to keep the template on disk; discarding it deletes the fixture immediately"]
fn write_template(agents_dir: &Path, name: &str, body: &str) -> TemplateFixture {
    let dir = agents_dir.join(name);
    std::fs::create_dir_all(&dir).expect("create template dir");
    std::fs::write(dir.join("agent.toml"), body).expect("write agent.toml");
    TemplateFixture { dir }
}

#[test]
fn template_fixture_cleans_up_during_unwind() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let unique = "tmpl_unwind_cleanup";

    let unwind = std::panic::catch_unwind(|| {
        let _fixture = write_template(tmp.path(), unique, "not parsed by this guard test");
        assert!(tmp.path().join(unique).exists());
        panic!("exercise fixture unwind cleanup");
    });

    assert!(unwind.is_err(), "fixture test must exercise unwinding");
    assert!(
        !tmp.path().join(unique).exists(),
        "template fixture must be removed while unwinding"
    );
}

fn minimal_manifest_toml(name: &str, description: &str) -> String {
    format!(
        r#"name = "{name}"
version = "0.1.0"
description = "{description}"
module = "builtin:chat"
tags = ["test"]

[model]
provider = "default"
model = "default"

[capabilities]
tools = ["web_fetch"]
"#
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_list_includes_seeded_template() {
    let h = boot().await;
    let unique = "tmpl_list_alpha";
    let _fixture = write_template(
        &agents_dir(&h),
        unique,
        &minimal_manifest_toml("alpha", "Alpha test template"),
    );

    let (status, body) = get_json(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let templates = body["templates"].as_array().expect("templates array");
    let total = body["total"].as_u64().expect("total u64");
    assert_eq!(
        total as usize,
        templates.len(),
        "total must match array len: {body}"
    );
    let row = templates
        .iter()
        .find(|r| r["name"] == unique)
        .unwrap_or_else(|| panic!("seeded template missing from list: {body}"));
    assert_eq!(row["description"], "Alpha test template", "{body}");
}

/// The TUI templates screen renders provider/model per row and gates spawning on whether that provider is configured.
/// Before #7760 it rendered a compiled-in list instead of calling this route at all; now that it does, the listing has to carry what each template actually declares rather than leaving the client to assume a default.
#[tokio::test(flavor = "multi_thread")]
async fn templates_list_carries_provider_and_model_from_the_manifest() {
    let h = boot().await;
    let unique = "tmpl_list_provider";
    let _fixture = write_template(
        &agents_dir(&h),
        unique,
        r#"name = "delta"
version = "0.1.0"
description = "Delta test template"
module = "builtin:chat"

[model]
provider = "anthropic"
model = "claude-test-9"

[capabilities]
tools = ["file_read"]
"#,
    );

    let (status, body) = get_json(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = body["templates"]
        .as_array()
        .expect("templates array")
        .iter()
        .find(|r| r["name"] == unique)
        .unwrap_or_else(|| panic!("seeded template missing from list: {body}"))
        .clone();
    assert_eq!(row["provider"], "anthropic", "{body}");
    assert_eq!(row["model"], "claude-test-9", "{body}");
    assert_eq!(row["description"], "Delta test template", "{body}");
}

/// A single unparseable manifest used to fail the whole listing with a 500, so one operator typo blanked every agent type for every client.
/// It must be skipped instead, leaving the valid entries visible.
#[tokio::test(flavor = "multi_thread")]
async fn templates_list_skips_a_malformed_manifest_instead_of_failing() {
    let h = boot().await;
    let dir = agents_dir(&h);
    let good = "tmpl_skip_good";
    let bad = "tmpl_skip_bad";
    let _good_fixture = write_template(&dir, good, &minimal_manifest_toml("echo", "Echo survives"));
    let _bad_fixture = write_template(&dir, bad, "this is not = = valid toml [[[");

    let (status, body) = get_json(&h, "/api/templates").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "one bad manifest must not fail the listing: {body}"
    );
    let templates = body["templates"].as_array().expect("templates array");
    assert!(
        templates.iter().any(|r| r["name"] == good),
        "the valid template must still be listed: {body}"
    );
    assert!(
        !templates.iter().any(|r| r["name"] == bad),
        "the malformed template must be skipped, not rendered: {body}"
    );
}

/// The listing must not advertise a name that `/templates/{name}` and `/templates/{name}/toml` will reject — a row a client cannot fetch or spawn from is a dead end on the screen.
#[tokio::test(flavor = "multi_thread")]
async fn templates_list_omits_names_the_detail_routes_reject() {
    let h = boot().await;
    let unusable = "tmpl.dotted.name";
    let _fixture = write_template(
        &agents_dir(&h),
        unusable,
        &minimal_manifest_toml("dotted", "Dotted"),
    );

    let (status, body) = get_json(&h, "/api/templates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body["templates"]
            .as_array()
            .expect("templates array")
            .iter()
            .any(|r| r["name"] == unusable),
        "a name the validator rejects must not be listed: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_known_template_returns_manifest() {
    let h = boot().await;
    let unique = "tmpl_get_bravo";
    let toml_body = minimal_manifest_toml("bravo", "Bravo description");
    let _fixture = write_template(&agents_dir(&h), unique, &toml_body);

    let (status, body) = get_json(&h, &format!("/api/templates/{unique}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], unique);
    assert_eq!(body["manifest"]["name"], "bravo");
    assert_eq!(body["manifest"]["description"], "Bravo description");
    assert_eq!(body["manifest"]["module"], "builtin:chat");
    assert!(
        body["manifest_toml"]
            .as_str()
            .map(|s| s.contains("name = \"bravo\""))
            .unwrap_or(false),
        "manifest_toml must round-trip the raw file: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = get_json(&h, "/api/templates/does_not_exist_xyz").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        body["error"].is_string() || body["error"]["message"].is_string(),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_rejects_path_traversal_as_404() {
    // The handler runs `validate_template_name` and turns a malformed name
    // into a 404 (NOT 400) so we don't leak the existence of the validator
    // to scanners. Pin that contract.
    let h = boot().await;
    // axum normalises `..` in paths, so target a name that survives URL
    // routing but still trips the validator: a dot-bearing string.
    let (status, body) = get_json(&h, "/api/templates/foo.bar").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_toml_returns_plaintext_for_known_template() {
    let h = boot().await;
    let unique = "tmpl_toml_charlie";
    let toml_body = minimal_manifest_toml("charlie", "Charlie raw");
    let _fixture = write_template(&agents_dir(&h), unique, &toml_body);

    let (status, headers, bytes) = get(&h, &format!("/api/templates/{unique}/toml")).await;
    assert_eq!(status, StatusCode::OK);
    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/plain"),
        "expected text/plain content-type, got: {ct:?}"
    );
    let body_str = String::from_utf8(bytes).expect("utf8");
    assert!(
        body_str.contains("name = \"charlie\""),
        "raw TOML must round-trip verbatim: {body_str:?}"
    );
    assert!(
        body_str.contains("Charlie raw"),
        "raw TOML must include description: {body_str:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_toml_unknown_returns_plaintext_404() {
    let h = boot().await;
    let (status, headers, bytes) = get(&h, "/api/templates/no_such_tmpl/toml").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/plain"),
        "404 path must also serve text/plain to match success shape: {ct:?}"
    );
    assert!(!bytes.is_empty(), "404 plaintext body must be non-empty");
}

// ---------------------------------------------------------------------------
// /api/templates/{name} — registry-promotion privacy pass (refs #7771).
//
// An agent type an operator built on their own install is an `AgentManifest` written against one machine.
// Contributing it to a shared registry publishes that machine's details unless something strips them first, and the registry validator requires only `name` / `description` / `module`, so nothing downstream catches it.
// The detail endpoint carries the read-only half of that pass; these tests pin its shape and its two directions — what must not survive, and what must.
// ---------------------------------------------------------------------------

/// A template manifest with one host-specific or secret-adjacent value per category, each a distinctive sentinel so the assertions are unambiguous.
fn leaky_manifest_toml(name: &str) -> String {
    format!(
        r#"name = "{name}"
version = "0.1.0"
description = "Reads sources and writes briefs."
author = "jane.doe@acme-internal.example"
module = "builtin:chat"
workspace = "/Users/janedoe/.librefang/workspaces/{name}-a1b2"
is_hand = true

[model]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
system_prompt = "You are a research assistant."
api_key_env = "ACME_PROD_ANTHROPIC_KEY"
base_url = "https://llm-gateway.acme.internal/v1"

[capabilities]
tools = ["web_fetch"]
network = ["vault.acme.internal:443"]
shell = ["/opt/acme/bin/deploy"]

[metadata]
cost_centre = "SENTINEL_COST_CENTRE"

[workspaces]
contracts = {{ mount = "/Volumes/acme-legal/contracts", mode = "r" }}

[[context_injection]]
name = "runbook"
content = "Escalate via SENTINEL_RUNBOOK."
"#
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_reports_what_promotion_would_strip() {
    let h = boot().await;
    let unique = "tmpl_promo_leaky";
    let _fixture = write_template(&agents_dir(&h), unique, &leaky_manifest_toml("researcher"));

    let (status, body) = get_json(&h, &format!("/api/templates/{unique}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let preview = &body["promotion_preview"];
    assert!(
        preview.is_object(),
        "the detail response must carry the promotion privacy pass: {body}"
    );

    let findings = preview["findings"]
        .as_array()
        .unwrap_or_else(|| panic!("findings must be an array: {body}"));
    let stripped: Vec<&str> = findings
        .iter()
        .filter(|finding| finding["removed_by_sanitizer"] == true)
        .map(|finding| finding["field"].as_str().unwrap_or_default())
        .collect();

    // One assertion per category of sensitive field, so a regression names which category stopped being reported.
    for expected in [
        "author",            // operator identity
        "model.api_key_env", // credential binding
        "model.base_url",    // private endpoint
        "capabilities.network",
        "capabilities.shell", // host policy
        "metadata",           // operator key/value bag
        "workspace",          // absolute host path
        "workspaces",         // named mount into the host filesystem
        "context_injection",  // operator free text
        "is_hand",            // local provenance
    ] {
        assert!(
            stripped.contains(&expected),
            "'{expected}' must be reported as dropped by promotion: {body}"
        );
    }

    for finding in findings {
        assert!(
            finding["category"].is_string(),
            "every finding names its category: {finding}"
        );
        let preview_text = finding["preview"].as_str().unwrap_or_default();
        assert!(
            preview_text.chars().count() <= 97,
            "previews must stay bounded: {finding}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_promotion_preview_omits_host_specifics_and_keeps_the_portable_half() {
    let h = boot().await;
    let unique = "tmpl_promo_scrub";
    let _fixture = write_template(&agents_dir(&h), unique, &leaky_manifest_toml("researcher"));

    let (status, body) = get_json(&h, &format!("/api/templates/{unique}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let publishable = body["promotion_preview"]["manifest_toml"]
        .as_str()
        .unwrap_or_else(|| panic!("the publishable manifest must render as TOML: {body}"));

    for sentinel in [
        "/Users/janedoe",
        "/Volumes/acme-legal",
        "/opt/acme/bin/deploy",
        "ACME_PROD_ANTHROPIC_KEY",
        "llm-gateway.acme.internal",
        "vault.acme.internal",
        "SENTINEL_COST_CENTRE",
        "SENTINEL_RUNBOOK",
        "jane.doe@acme-internal.example",
    ] {
        assert!(
            !publishable.contains(sentinel),
            "'{sentinel}' must not reach the publishable manifest: {publishable}"
        );
    }

    // Stripping everything would leave nothing worth publishing, so pin the half that has to survive.
    for kept in [
        "researcher",
        "Reads sources and writes briefs.",
        "builtin:chat",
        "You are a research assistant.",
        "anthropic",
        "claude-sonnet-4-20250514",
        "web_fetch",
    ] {
        assert!(
            publishable.contains(kept),
            "'{kept}' must survive promotion: {publishable}"
        );
    }

    // The raw file is still returned verbatim — this endpoint reports on the operator's manifest, it does not rewrite it.
    assert!(
        body["manifest_toml"]
            .as_str()
            .map(|raw| raw.contains("ACME_PROD_ANTHROPIC_KEY"))
            .unwrap_or(false),
        "the operator's own manifest must be returned unmodified: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_promotion_preview_flags_a_secret_inside_a_retained_prompt() {
    let h = boot().await;
    // A credential pasted into the system prompt sits inside a field promotion has to keep, so the operator has to edit it by hand.
    // The detector exists to say so rather than to silently keep it.
    let unique = "tmpl_promo_review";
    let _fixture = write_template(
        &agents_dir(&h),
        unique,
        r#"name = "leaky-prompt"
version = "0.1.0"
description = "Fetches build status."
module = "builtin:chat"

[model]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
system_prompt = "Authenticate with sk-ant-api03-QQQQWWWWEEEERRRRTTTTYYYY before calling."
"#,
    );

    let (status, body) = get_json(&h, &format!("/api/templates/{unique}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let preview = &body["promotion_preview"];
    assert_eq!(
        preview["requires_review"], true,
        "a secret inside a retained field must require operator review: {body}"
    );
    let flagged = preview["findings"]
        .as_array()
        .unwrap_or_else(|| panic!("findings must be an array: {body}"))
        .iter()
        .any(|finding| {
            finding["field"] == "model.system_prompt"
                && finding["category"] == "secret_literal"
                && finding["removed_by_sanitizer"] == false
        });
    assert!(flagged, "the pasted key must be flagged: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_get_promotion_preview_is_quiet_for_an_already_portable_template() {
    let h = boot().await;
    let unique = "tmpl_promo_clean";
    let _fixture = write_template(
        &agents_dir(&h),
        unique,
        &minimal_manifest_toml("charlie", "Charlie type"),
    );

    let (status, body) = get_json(&h, &format!("/api/templates/{unique}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let preview = &body["promotion_preview"];
    assert_eq!(
        preview["findings"].as_array().map(Vec::len),
        Some(0),
        "a portable template must produce no findings: {body}"
    );
    assert_eq!(preview["requires_review"], false, "{body}");
    assert!(
        preview["manifest_toml"].is_string(),
        "the publishable manifest must still be offered: {body}"
    );
}

// ---------------------------------------------------------------------------
// #8112 — `GET /api/templates/{name}` and `POST /api/agents {"template": name}`
// must resolve the SAME agent-types store, even when `home_dir` diverges from
// `LIBREFANG_HOME`. Before the fix, `get_agent_template` read via the
// env-based `read_agent_type()` wrapper while the spawn path
// (`resolve_manifest` in `routes/agents/lifecycle.rs`) already read via
// `read_agent_type_in(&state.kernel.config_ref().home_dir, …)` — an embedder
// whose `KernelConfig.home_dir` was not derived from `LIBREFANG_HOME` (every
// `MockKernelBuilder` test kernel, in fact) saw the editor and the spawn path
// disagree about which document a name refers to.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn template_detail_route_agrees_with_the_kernels_own_home_dir() {
    let h = boot().await;
    let unique = "tmpl_home_dir_parity";
    // Written under this harness's OWN kernel home_dir, not `LIBREFANG_HOME`
    // (which nothing in this test ever sets).
    let _fixture = write_template(
        &agents_dir(&h),
        unique,
        &minimal_manifest_toml("home-dir-parity", "Home dir parity check"),
    );

    let (status, body) = get_json(&h, &format!("/api/templates/{unique}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/templates/{{name}} must resolve against this kernel's own \
         home_dir, not the process-wide LIBREFANG_HOME env var: {body}"
    );
    assert_eq!(body["manifest"]["name"], "home-dir-parity");
}
