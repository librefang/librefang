//! Passkey (WebAuthn/FIDO2) HTTP endpoints (#5981).
//!
//! Six routes implementing the two standard WebAuthn ceremonies plus
//! credential management:
//!
//! ```text
//! POST   /api/auth/passkey/registration-options      (auth; Owner)
//! POST   /api/auth/passkey/registration-verify       (auth; Owner)
//! POST   /api/auth/passkey/authentication-options     (public)
//! POST   /api/auth/passkey/authentication-verify      (public; mints session)
//! GET    /api/auth/passkey/credentials                 (auth; list)
//! DELETE /api/auth/passkey/credentials/{id}            (auth; Owner; revoke)
//! ```
//!
//! The ceremony cryptography lives in [`crate::passkey::PasskeyEngine`]; this
//! module is the thin HTTP shell that loads/persists credentials via the
//! [`librefang_memory::passkey_store::PasskeyStore`] and, on a successful
//! assertion, mints a dashboard session byte-for-byte identical to
//! `dashboard_login` (see [`crate::server::mint_dashboard_session`]).
//!
//! ## TOTP interaction
//!
//! A passkey is phishing-resistant possession + (with user verification)
//! inherence — it already satisfies the second-factor requirement on its own.
//! A successful passkey assertion therefore mints the session directly and
//! does **not** trigger the password-path TOTP challenge. Operators who want
//! a typed second factor should keep using password login; passkey login is
//! the additive, friction-light alternative.

use super::AppState;
use crate::middleware::AuthenticatedApiUser;
use crate::passkey::{encode_credential_id, PasskeyError};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::net::SocketAddr;
use std::sync::Arc;
use webauthn_rs::prelude::{Passkey, PublicKeyCredential, RegisterPublicKeyCredential};

/// Build routes for the passkey sub-domain. Merged under `/api/auth` by
/// `server.rs::api_v1_routes()`.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/auth/passkey/registration-options",
            axum::routing::post(registration_options),
        )
        .route(
            "/auth/passkey/registration-verify",
            axum::routing::post(registration_verify),
        )
        .route(
            "/auth/passkey/authentication-options",
            axum::routing::post(authentication_options),
        )
        .route(
            "/auth/passkey/authentication-verify",
            axum::routing::post(authentication_verify),
        )
        .route(
            "/auth/passkey/credentials",
            axum::routing::get(list_credentials),
        )
        .route(
            "/auth/passkey/credentials/{id}",
            axum::routing::delete(revoke_credential),
        )
}

// ---------------------------------------------------------------------------
// Small response helpers
// ---------------------------------------------------------------------------

fn json_err(status: StatusCode, error: &str, message: impl AsRef<str>) -> Response {
    (
        status,
        Json(serde_json::json!({
            "ok": false,
            "error": error,
            "message": message.as_ref(),
        })),
    )
        .into_response()
}

/// 503 when passkey login is not enabled / misconfigured.
fn engine_unavailable() -> Response {
    json_err(
        StatusCode::SERVICE_UNAVAILABLE,
        "passkey_disabled",
        "Passkey login is not enabled on this server (set passkey_enabled and the RP config).",
    )
}

/// Map an engine error onto an HTTP response. Ceremony/verification failures
/// are client-facing `400`s; a corrupt stored credential is a server fault.
fn engine_error_response(e: PasskeyError) -> Response {
    match e {
        PasskeyError::UnknownCeremony => json_err(
            StatusCode::BAD_REQUEST,
            "ceremony_expired",
            "The passkey ceremony expired or was already used. Start again.",
        ),
        PasskeyError::Webauthn(inner) => json_err(
            StatusCode::BAD_REQUEST,
            "webauthn_failed",
            inner.to_string(),
        ),
        PasskeyError::CorruptCredential(inner) => json_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "corrupt_credential",
            inner.to_string(),
        ),
    }
}

/// Deserialize a stored credential blob into a [`Passkey`], logging and
/// skipping a corrupt row rather than failing the whole ceremony.
fn parse_stored_passkeys(rows: &[librefang_memory::passkey_store::PasskeyRecord]) -> Vec<Passkey> {
    rows.iter()
        .filter_map(|r| match serde_json::from_str::<Passkey>(&r.cred) {
            Ok(pk) => Some(pk),
            Err(e) => {
                tracing::warn!(
                    credential_id = %r.credential_id,
                    error = %e,
                    "skipping corrupt stored passkey credential"
                );
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Registration (auth-gated; Owner)
// ---------------------------------------------------------------------------

/// Begin adding a passkey to the authenticated account. Returns a
/// `ceremony_id` plus the `PublicKeyCredentialCreationOptions` for
/// `navigator.credentials.create()`.
#[utoipa::path(
    post,
    path = "/api/auth/passkey/registration-options",
    tag = "auth",
    responses(
        (status = 200, description = "Creation options + ceremony id", body = crate::types::JsonObject),
        (status = 401, description = "Not authenticated"),
        (status = 503, description = "Passkey login not enabled")
    )
)]
pub(crate) async fn registration_options(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
    _body: Option<Json<serde_json::Value>>,
) -> Response {
    let Some(engine) = state.passkey_engine.clone() else {
        return engine_unavailable();
    };
    let Some(axum::Extension(user)) = api_user else {
        return json_err(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Login required.",
        );
    };

    let existing = match state.passkey_store.list_for_user(&user.name) {
        Ok(rows) => parse_stored_passkeys(&rows),
        Err(e) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "store_error",
                e.to_string(),
            )
        }
    };

    match engine.start_registration(&user.name, &existing) {
        Ok((ceremony_id, options)) => Json(serde_json::json!({
            "ceremony_id": ceremony_id,
            "options": options,
        }))
        .into_response(),
        Err(e) => engine_error_response(e),
    }
}

/// Finish adding a passkey: verify the attestation and persist the credential.
#[utoipa::path(
    post,
    path = "/api/auth/passkey/registration-verify",
    tag = "auth",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Credential stored", body = crate::types::JsonObject),
        (status = 400, description = "Verification failed"),
        (status = 401, description = "Not authenticated"),
        (status = 503, description = "Passkey login not enabled")
    )
)]
pub(crate) async fn registration_verify(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
    Json(body): Json<RegistrationVerifyRequest>,
) -> Response {
    let Some(engine) = state.passkey_engine.clone() else {
        return engine_unavailable();
    };
    let Some(axum::Extension(user)) = api_user else {
        return json_err(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Login required.",
        );
    };

    let passkey = match engine.finish_registration(&body.ceremony_id, &user.name, &body.credential)
    {
        Ok(pk) => pk,
        Err(e) => return engine_error_response(e),
    };

    let credential_id = encode_credential_id(passkey.cred_id());
    let cred_json = match serde_json::to_string(&passkey) {
        Ok(s) => s,
        Err(e) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialize_error",
                e.to_string(),
            )
        }
    };
    let label = body
        .label
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());

    let record = librefang_memory::passkey_store::new_record(
        credential_id.clone(),
        user.name.clone(),
        cred_json,
        label,
    );
    if let Err(e) = state.passkey_store.insert(&record) {
        return json_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        );
    }

    tracing::info!(user = %user.name, %credential_id, "passkey registered");
    Json(serde_json::json!({ "ok": true, "credential_id": credential_id })).into_response()
}

// ---------------------------------------------------------------------------
// Authentication (public; mints session)
// ---------------------------------------------------------------------------

/// Begin a passkey login. Public — no session exists yet. Returns the
/// `PublicKeyCredentialRequestOptions` for `navigator.credentials.get()`.
#[utoipa::path(
    post,
    path = "/api/auth/passkey/authentication-options",
    tag = "auth",
    responses(
        (status = 200, description = "Request options + ceremony id", body = crate::types::JsonObject),
        (status = 400, description = "No passkeys registered"),
        (status = 503, description = "Passkey login not enabled")
    )
)]
pub(crate) async fn authentication_options(
    State(state): State<Arc<AppState>>,
    _body: Option<Json<serde_json::Value>>,
) -> Response {
    let Some(engine) = state.passkey_engine.clone() else {
        return engine_unavailable();
    };
    let principal = engine.principal().to_string();

    let passkeys = match state.passkey_store.list_for_user(&principal) {
        Ok(rows) => parse_stored_passkeys(&rows),
        Err(e) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "store_error",
                e.to_string(),
            )
        }
    };
    if passkeys.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "no_passkeys",
            "No passkeys are registered for this account.",
        );
    }

    match engine.start_authentication(&passkeys) {
        Ok((ceremony_id, options)) => Json(serde_json::json!({
            "ceremony_id": ceremony_id,
            "options": options,
        }))
        .into_response(),
        Err(e) => engine_error_response(e),
    }
}

/// Finish a passkey login: verify the assertion, persist the updated
/// sign-count, and mint a dashboard session identical to `dashboard_login`.
#[utoipa::path(
    post,
    path = "/api/auth/passkey/authentication-verify",
    tag = "auth",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Session token (same shape as dashboard-login)", body = crate::types::JsonObject),
        (status = 400, description = "Assertion verification failed"),
        (status = 503, description = "Passkey login not enabled")
    )
)]
pub(crate) async fn authentication_verify(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(body): Json<AuthenticationVerifyRequest>,
) -> Response {
    let Some(engine) = state.passkey_engine.clone() else {
        return engine_unavailable();
    };

    let auth_result = match engine.finish_authentication(&body.ceremony_id, &body.credential) {
        Ok(r) => r,
        Err(e) => return engine_error_response(e),
    };

    let credential_id = encode_credential_id(auth_result.cred_id());
    // Look up the asserted credential to (a) persist any sign-count bump and
    // (b) learn which principal it authenticates as.
    let Some(record) = state.passkey_store.get(&credential_id).ok().flatten() else {
        // The assertion verified against in-flight state but the row vanished
        // (revoked mid-ceremony). Treat as a failed login.
        return json_err(
            StatusCode::BAD_REQUEST,
            "unknown_credential",
            "The asserted passkey is no longer registered.",
        );
    };

    // Update the stored credential's sign-count / last-used. We re-serialize
    // unconditionally so `last_used_at` is always fresh; the counter bump is
    // applied via `update_credential`.
    if let Ok(mut passkey) = serde_json::from_str::<Passkey>(&record.cred) {
        passkey.update_credential(&auth_result);
        if let Ok(cred_json) = serde_json::to_string(&passkey) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if let Err(e) = state
                .passkey_store
                .update_cred(&credential_id, &cred_json, now)
            {
                tracing::warn!(error = %e, %credential_id, "failed to persist passkey sign-count update");
            }
        }
    }

    tracing::info!(user = %record.user_name, %credential_id, "passkey login succeeded");
    crate::server::mint_dashboard_session(
        &state,
        &record.user_name,
        "owner",
        peer_addr.ip(),
        &headers,
    )
    .await
}

// ---------------------------------------------------------------------------
// Credential management (auth)
// ---------------------------------------------------------------------------

/// List the authenticated account's registered passkeys (metadata only — the
/// credential blob is never exposed).
#[utoipa::path(
    get,
    path = "/api/auth/passkey/credentials",
    tag = "auth",
    responses(
        (status = 200, description = "Registered passkeys", body = crate::types::JsonObject),
        (status = 401, description = "Not authenticated")
    )
)]
pub(crate) async fn list_credentials(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    let Some(axum::Extension(user)) = api_user else {
        return json_err(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Login required.",
        );
    };
    match state.passkey_store.list_for_user(&user.name) {
        Ok(rows) => {
            let items: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "credential_id": r.credential_id,
                        "label": r.label,
                        "created_at": r.created_at,
                        "last_used_at": r.last_used_at,
                    })
                })
                .collect();
            Json(serde_json::json!({ "credentials": items })).into_response()
        }
        Err(e) => json_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        ),
    }
}

/// Revoke (delete) a registered passkey by its base64url credential id.
/// Scoped to the authenticated principal — one account can never delete
/// another's credential.
#[utoipa::path(
    delete,
    path = "/api/auth/passkey/credentials/{id}",
    tag = "auth",
    params(("id" = String, Path, description = "Base64url credential id")),
    responses(
        (status = 200, description = "Revoked", body = crate::types::JsonObject),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "No such credential for this account")
    )
)]
pub(crate) async fn revoke_credential(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
    Path(id): Path<String>,
) -> Response {
    let Some(axum::Extension(user)) = api_user else {
        return json_err(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Login required.",
        );
    };
    match state.passkey_store.delete(&id, &user.name) {
        Ok(true) => {
            tracing::info!(user = %user.name, credential_id = %id, "passkey revoked");
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(false) => json_err(
            StatusCode::NOT_FOUND,
            "not_found",
            "No such passkey credential for this account.",
        ),
        Err(e) => json_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub(crate) struct RegistrationVerifyRequest {
    ceremony_id: String,
    credential: RegisterPublicKeyCredential,
    #[serde(default)]
    label: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct AuthenticationVerifyRequest {
    ceremony_id: String,
    credential: PublicKeyCredential,
}
