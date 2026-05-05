//! Integration tests for the TOTP enrollment / confirm / status / revoke flow.
//!
//! Exercises `POST /api/approvals/totp/{setup,confirm,revoke}` and
//! `GET /api/approvals/totp/status` end-to-end against a freshly-booted
//! mock kernel. Issue references: #3402, #3403.
//!
//! Notes:
//! - The system router is mounted directly under `/api`, mirroring the
//!   pairing tests — this skips the global auth middleware so each test
//!   focuses on the handler's own logic, not the auth gate.
//! - Generating a "current" TOTP code in tests requires reproducing the
//!   exact (algo, digits, step, secret, issuer) tuple that `ApprovalManager`
//!   uses. That contract is asserted by reading the secret out of the vault
//!   right after setup and feeding it through `totp-rs` with the same
//!   parameters as `ApprovalManager::generate_totp_secret`.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
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

async fn json_post(
    h: &Harness,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
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

async fn get(h: &Harness, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
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

/// Build a `TOTP` client that exactly mirrors `ApprovalManager::generate_totp_secret`
/// so `generate_current()` produces a code the kernel will accept.
fn totp_for(secret_base32: &str, issuer: &str) -> totp_rs::TOTP {
    use totp_rs::{Algorithm, Secret, TOTP};
    let raw = Secret::Encoded(secret_base32.to_string())
        .to_bytes()
        .expect("decode base32 secret");
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        raw,
        Some(issuer.to_string()),
        String::new(),
    )
    .expect("totp init")
}

fn current_code(h: &Harness) -> String {
    let secret = h
        .state
        .kernel
        .vault_get("totp_secret")
        .expect("totp_secret in vault");
    let issuer = h.state.kernel.approvals().policy().totp_issuer.clone();
    totp_for(&secret, &issuer)
        .generate_current()
        .expect("generate current code")
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Status before any enrollment must report an empty, unconfirmed, unenforced
/// state — the dashboard relies on these defaults to decide whether to surface
/// the "Set up 2FA" CTA.
#[tokio::test(flavor = "multi_thread")]
async fn status_initial_state_is_empty() {
    let h = boot();
    let (status, body) = get(&h, "/api/approvals/totp/status").await;
    assert_eq!(status, StatusCode::OK, "got body: {body:?}");
    assert_eq!(body["enrolled"], false);
    assert_eq!(body["confirmed"], false);
    assert_eq!(body["enforced"], false);
    assert_eq!(body["remaining_recovery_codes"], 0);
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

/// First-time setup must mint a base32 secret, an otpauth URI, a data:image
/// QR payload, and a fresh batch of recovery codes — and persist the secret
/// in the vault as "pending" (`totp_confirmed = "false"`).
#[tokio::test(flavor = "multi_thread")]
async fn setup_returns_secret_uri_qr_and_recovery_codes() {
    let h = boot();
    let (status, body) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "got body: {body:?}");

    let secret = body["secret"].as_str().expect("secret string");
    assert!(!secret.is_empty(), "secret must not be empty");

    let uri = body["otpauth_uri"].as_str().expect("otpauth_uri string");
    assert!(uri.starts_with("otpauth://totp/"), "got {uri}");

    let qr = body["qr_code"].as_str().expect("qr_code string");
    assert!(
        qr.starts_with("data:image/png;base64,"),
        "qr_code must be a data: URI, got prefix {:?}",
        &qr.chars().take(30).collect::<String>()
    );

    let codes = body["recovery_codes"].as_array().expect("recovery_codes");
    assert_eq!(codes.len(), 8, "expected 8 recovery codes");

    // Vault state: pending confirmation.
    assert_eq!(
        h.state.kernel.vault_get("totp_secret").as_deref(),
        Some(secret)
    );
    assert_eq!(
        h.state.kernel.vault_get("totp_confirmed").as_deref(),
        Some("false"),
        "secret must be present but unconfirmed after setup"
    );

    // Status now reports enrolled-but-unconfirmed.
    let (_, st) = get(&h, "/api/approvals/totp/status").await;
    assert_eq!(st["enrolled"], true);
    assert_eq!(st["confirmed"], false);
    assert_eq!(st["remaining_recovery_codes"], 8);
}

/// Calling `setup` a second time while the first enrollment is still pending
/// must NOT silently overwrite the original secret — that would invalidate
/// the QR already scanned into a user's authenticator app without any
/// indication. Expect 409 CONFLICT.
#[tokio::test(flavor = "multi_thread")]
async fn setup_pending_enrollment_is_conflict_not_overwrite() {
    let h = boot();
    let (s1, b1) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    assert_eq!(s1, StatusCode::OK, "first setup must succeed: {b1:?}");
    let original_secret = b1["secret"].as_str().unwrap().to_string();

    let (s2, b2) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    assert_eq!(s2, StatusCode::CONFLICT, "got body: {b2:?}");
    assert_eq!(b2["status"], "pending_confirmation");

    // Vault secret must still be the first one.
    assert_eq!(
        h.state.kernel.vault_get("totp_secret").as_deref(),
        Some(original_secret.as_str()),
        "second setup must not clobber pending secret"
    );
}

/// Once an enrollment is confirmed, `setup` MUST require the caller to
/// authenticate with the existing TOTP/recovery code before issuing a new
/// secret. Calling without `current_code` is a 400 — anything else would
/// allow a session-hijacked attacker to silently rotate 2FA off the victim.
#[tokio::test(flavor = "multi_thread")]
async fn setup_when_already_confirmed_requires_current_code() {
    let h = boot();
    // First setup + confirm.
    let (_, b1) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    assert!(b1["secret"].is_string());
    let code = current_code(&h);
    let (sc, _) = json_post(
        &h,
        "/api/approvals/totp/confirm",
        serde_json::json!({ "code": code }),
    )
    .await;
    assert_eq!(sc, StatusCode::OK);
    assert_eq!(
        h.state.kernel.vault_get("totp_confirmed").as_deref(),
        Some("true")
    );

    // Bare setup call must now refuse.
    let (s2, b2) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    assert_eq!(s2, StatusCode::BAD_REQUEST, "got body: {b2:?}");
    let err = b2["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("current_code"),
        "error must mention current_code, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Confirm
// ---------------------------------------------------------------------------

/// Confirming with a fresh, valid current code must flip `totp_confirmed`
/// to `"true"` and report `confirmed=true` on the next status fetch.
#[tokio::test(flavor = "multi_thread")]
async fn confirm_with_valid_code_activates_enrollment() {
    let h = boot();
    let (_, _) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    let code = current_code(&h);

    let (status, body) = json_post(
        &h,
        "/api/approvals/totp/confirm",
        serde_json::json!({ "code": code }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got body: {body:?}");
    assert_eq!(body["status"], "confirmed");

    let (_, st) = get(&h, "/api/approvals/totp/status").await;
    assert_eq!(st["enrolled"], true);
    assert_eq!(st["confirmed"], true);
}

/// Confirming with a wrong code must not flip `totp_confirmed`; the secret
/// remains pending and a follow-up status call still shows `confirmed=false`.
#[tokio::test(flavor = "multi_thread")]
async fn confirm_with_invalid_code_keeps_pending() {
    let h = boot();
    let (_, _) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;

    let (status, body) = json_post(
        &h,
        "/api/approvals/totp/confirm",
        serde_json::json!({ "code": "000000" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got body: {body:?}");

    assert_eq!(
        h.state.kernel.vault_get("totp_confirmed").as_deref(),
        Some("false"),
        "confirmed flag must stay false after a bad code"
    );
    let (_, st) = get(&h, "/api/approvals/totp/status").await;
    assert_eq!(st["confirmed"], false);
}

/// Replaying the same TOTP code twice (even one that just successfully
/// confirmed) must be rejected — the replay-prevention bucket
/// (`is_totp_code_used`) is the only line of defense between an attacker
/// who shoulder-surfed a single 30-second window and full 2FA bypass.
#[tokio::test(flavor = "multi_thread")]
async fn confirm_rejects_replayed_code() {
    let h = boot();
    let (_, _) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    let code = current_code(&h);

    let (s1, _) = json_post(
        &h,
        "/api/approvals/totp/confirm",
        serde_json::json!({ "code": code.clone() }),
    )
    .await;
    assert_eq!(s1, StatusCode::OK);

    // Re-issue the same code: the replay bucket (`is_totp_code_used`)
    // must reject it, even though the first call already succeeded.
    let (s2, b2) = json_post(
        &h,
        "/api/approvals/totp/confirm",
        serde_json::json!({ "code": code }),
    )
    .await;
    assert_eq!(s2, StatusCode::BAD_REQUEST, "got body: {b2:?}");
    let err = b2["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("already been used"),
        "expected replay error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Revoke
// ---------------------------------------------------------------------------

/// Revoke must refuse before any enrollment exists — returning 200 here
/// would let an unauthenticated probe of this endpoint clear arbitrary
/// vault keys (or report misleading success). Expect 400.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_before_enrollment_is_bad_request() {
    let h = boot();
    let (status, body) = json_post(
        &h,
        "/api/approvals/totp/revoke",
        serde_json::json!({ "code": "000000" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got body: {body:?}");
    let err = body["error"]
        .as_str()
        .or_else(|| body["error"]["message"].as_str())
        .unwrap_or_default();
    assert!(
        err.contains("not enrolled"),
        "expected 'not enrolled' error, got: {err}"
    );
}

/// Revoke with a valid current TOTP code must wipe both the secret and the
/// confirmation flag — the login gate keys off `totp_confirmed`, so leaving
/// it `true` would silently keep 2FA active even after a "successful"
/// revoke (the precise regression that motivated the rewrite at line ~2933).
#[tokio::test(flavor = "multi_thread")]
async fn revoke_with_valid_code_clears_enrollment() {
    let h = boot();
    let (_, _) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    let confirm_code = current_code(&h);
    let (sc, _) = json_post(
        &h,
        "/api/approvals/totp/confirm",
        serde_json::json!({ "code": confirm_code }),
    )
    .await;
    assert_eq!(sc, StatusCode::OK);

    // Use a recovery code (not the just-consumed TOTP code, which is now in
    // the replay bucket — and waiting out the 30s step boundary would slow
    // the test).
    let recovery_json = h
        .state
        .kernel
        .vault_get("totp_recovery_codes")
        .expect("recovery codes in vault");
    let recovery: Vec<String> = serde_json::from_str(&recovery_json).expect("decode recovery");
    let one_code = recovery
        .first()
        .expect("at least one recovery code")
        .clone();

    let (status, body) = json_post(
        &h,
        "/api/approvals/totp/revoke",
        serde_json::json!({ "code": one_code }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got body: {body:?}");

    // After revoke, confirmed flag must be cleared (the login gate's check),
    // and the secret should also be gone so the verify path is dead.
    assert_ne!(
        h.state.kernel.vault_get("totp_confirmed").as_deref(),
        Some("true"),
        "totp_confirmed must not remain 'true' after revoke"
    );
    // The handler 'wipes' by writing the empty string (it doesn't remove the
    // vault entry outright); the status handler treats empty == missing, so the
    // verify path is dead either way. Accept both shapes.
    let secret_after = h.state.kernel.vault_get("totp_secret").unwrap_or_default();
    assert!(
        secret_after.is_empty(),
        "totp_secret must be empty/absent after revoke, got: {secret_after:?}"
    );

    let (_, st) = get(&h, "/api/approvals/totp/status").await;
    assert_eq!(st["enrolled"], false);
    assert_eq!(st["confirmed"], false);
}

/// Revoke with a wrong code must NOT clear enrollment. This guards the
/// most damaging single-step bypass: a hostile call that lands a 200 here
/// would disable 2FA outright.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_with_invalid_code_does_not_clear() {
    let h = boot();
    let (_, _) = json_post(&h, "/api/approvals/totp/setup", serde_json::json!({})).await;
    let confirm_code = current_code(&h);
    let (sc, _) = json_post(
        &h,
        "/api/approvals/totp/confirm",
        serde_json::json!({ "code": confirm_code }),
    )
    .await;
    assert_eq!(sc, StatusCode::OK);

    let (status, body) = json_post(
        &h,
        "/api/approvals/totp/revoke",
        serde_json::json!({ "code": "000000" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got body: {body:?}");

    // Enrollment must survive intact.
    assert_eq!(
        h.state.kernel.vault_get("totp_confirmed").as_deref(),
        Some("true"),
        "confirmed flag must NOT be cleared by a failed revoke"
    );
    assert!(
        h.state.kernel.vault_get("totp_secret").is_some(),
        "secret must NOT be wiped by a failed revoke"
    );
}
