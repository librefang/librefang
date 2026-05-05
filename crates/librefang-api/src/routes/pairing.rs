//! Device pairing endpoints — extracted from `system.rs` per #3749.
//!
//! Mounts the `/pairing/*` subtree (request, complete, list/remove devices,
//! notify). Public route paths are unchanged; this module is a sibling under
//! `routes::` and is mounted via `.merge(crate::routes::pairing::router())`
//! from `system::router()`.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use base64::Engine as _;
use librefang_kernel::pairing::PairedDevice;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build routes for the device pairing domain (`/pairing/*`).
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/pairing/request", axum::routing::post(pairing_request))
        .route("/pairing/complete", axum::routing::post(pairing_complete))
        .route("/pairing/devices", axum::routing::get(pairing_devices))
        .route(
            "/pairing/devices/{id}",
            axum::routing::delete(pairing_remove_device),
        )
        .route("/pairing/notify", axum::routing::post(pairing_notify))
}

// ─── Device Pairing endpoints ───────────────────────────────────────────

/// Resolve the daemon base_url that mobile clients should connect to,
/// embedded in the QR pairing payload.
///
/// Resolution order:
/// 1. `pairing.public_base_url` (operator-supplied, immune to header tampering)
/// 2. `Host` request header + scheme inferred from `X-Forwarded-Proto`
///
/// Returns `Err` only when neither path produces a usable URL — callers
/// surface that as 500 rather than emit a malformed QR.
fn resolve_pairing_base_url(
    configured: Option<&str>,
    headers: &axum::http::HeaderMap,
    host: &str,
) -> Result<String, String> {
    if let Some(url) = configured {
        let trimmed = url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            // Configured URL must carry a real http(s) scheme — silently
            // accepting `librefang.example.com` or `ftp://...` would
            // produce a QR the mobile client refuses with a vague
            // "unexpected base_url protocol" error.
            if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                return Err(format!(
                    "pairing.public_base_url must start with http:// or https:// (got: {trimmed:?})"
                ));
            }
            return Ok(trimmed.to_string());
        }
    }
    if host.is_empty() {
        return Err("Cannot resolve daemon base_url: missing Host header and \
                    pairing.public_base_url is not set"
            .to_string());
    }
    // Take the first comma-separated value, trim, and only accept it if
    // the result is non-empty — header value `""` or `, https` would
    // otherwise yield `://host`.
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("http");
    Ok(format!("{scheme}://{host}"))
}

/// POST /api/pairing/request — Create a new pairing request (returns token + QR URI).
#[utoipa::path(post, path = "/api/pairing/request", tag = "pairing", responses((status = 200, description = "Pairing request created", body = crate::types::JsonObject)))]
pub async fn pairing_request(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // Pull the Host header directly — axum 0.8 dropped the dedicated `Host`
    // extractor, and the project doesn't depend on `axum-extra`. The header
    // is mandatory in HTTP/1.1 so a missing one signals a malformed client.
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    // Resolve the base_url the mobile client should hit.
    //
    // Prefer the operator-configured `pairing.public_base_url` so the QR
    // payload is not influenced by request headers — trusting client
    // `X-Forwarded-Proto` would let any authenticated dashboard caller
    // forge `https://` even on a plain-HTTP daemon.
    //
    // When unset, fall back to `Host` + scheme inferred from
    // `X-Forwarded-Proto` (filtering blank values so we never emit
    // `://host`). If `Host` is also unusable, refuse rather than ship a
    // QR with a broken base_url.
    let base_url = match resolve_pairing_base_url(
        state.kernel.config_ref().pairing.public_base_url.as_deref(),
        &headers,
        &host,
    ) {
        Ok(url) => url,
        Err(msg) => {
            return ApiErrorResponse::internal(msg)
                .into_json_tuple()
                .into_response();
        }
    };
    match state.kernel.pairing_ref().create_pairing_request() {
        Ok(req) => {
            // Encode QR payload as base64 JSON so base_url (with "://") doesn't
            // need percent-encoding inside the outer librefang:// URI.
            let payload = serde_json::json!({
                "v": 1,
                "base_url": base_url,
                "token": req.token,
                "expires_at": req.expires_at.to_rfc3339(),
            });
            let payload_b64 =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string());
            let qr_uri = format!("librefang://pair?payload={payload_b64}");
            Json(serde_json::json!({
                "token": req.token,
                "qr_uri": qr_uri,
                "expires_at": req.expires_at.to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => ApiErrorResponse::bad_request(e)
            .into_json_tuple()
            .into_response(),
    }
}

/// Body of `POST /api/pairing/complete`. Typed so a missing/empty `token`
/// is rejected up front rather than silently degraded to an empty string
/// that the kernel pairing manager has to re-validate.
#[derive(serde::Deserialize)]
pub struct PairingCompleteRequest {
    pub token: String,
    #[serde(default = "default_unknown")]
    pub display_name: String,
    #[serde(default = "default_unknown")]
    pub platform: String,
    #[serde(default)]
    pub push_token: Option<String>,
}

fn default_unknown() -> String {
    "unknown".to_string()
}

/// POST /api/pairing/complete — Complete pairing with token + device info.
#[utoipa::path(post, path = "/api/pairing/complete", tag = "pairing", request_body = crate::types::JsonObject, responses((status = 200, description = "Pairing completed", body = crate::types::JsonObject)))]
pub async fn pairing_complete(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<PairingCompleteRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    // ErrorTranslator is !Send; drop before the .await on user_api_keys below
    // so the handler future remains Send.
    drop(t);
    let token = body.token.trim();
    if token.is_empty() {
        return ApiErrorResponse::bad_request("token is required")
            .into_json_tuple()
            .into_response();
    }
    let display_name = body.display_name.as_str();
    let platform = body.platform.as_str();
    let push_token = body.push_token.clone();
    // Mint a fresh per-device bearer token. The plaintext is returned
    // to the mobile client exactly once below; only the Argon2 hash is
    // persisted, so this token cannot be reconstructed from a database
    // dump and cannot be re-used by anyone except the holder.
    let plaintext_key = {
        let bytes: [u8; 32] = rand::random();
        hex::encode(bytes)
    };
    // Device bearers are 256-bit CSPRNG outputs — high enough entropy that
    // the Argon2 KDF cost is dead weight on every mobile request. Use a
    // plain SHA-256 hash; `verify_password` recognises the `$sha256$`
    // prefix and dispatches to the cheap path.
    let api_key_hash = crate::password_hash::hash_device_token(&plaintext_key);

    let device_id = uuid::Uuid::new_v4().to_string();
    let device_info = PairedDevice {
        device_id: device_id.clone(),
        display_name: display_name.to_string(),
        platform: platform.to_string(),
        paired_at: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        push_token,
        api_key_hash: api_key_hash.clone(),
    };

    match state
        .kernel
        .pairing_ref()
        .complete_pairing(token, device_info)
    {
        Ok(device) => {
            // Register this device's bearer with the live auth table so
            // the next request from the mobile app actually authenticates.
            // Devices are mapped to UserRole::User (chat with agents but no
            // admin-level mutations) — promote per-device privileges via a
            // future config knob if required.
            let device_user_name = format!("device:{}", device.device_id);
            let auth = crate::middleware::ApiUserAuth {
                name: device_user_name.clone(),
                role: crate::middleware::UserRole::User,
                api_key_hash,
                user_id: librefang_types::agent::UserId::from_name(&device_user_name),
            };
            state.user_api_keys.write().await.push(auth);

            tracing::info!(
                target: "pairing.audit",
                device_id = %device.device_id,
                display_name = %device.display_name,
                platform = %device.platform,
                "paired new device — bearer minted and registered"
            );

            Json(serde_json::json!({
                "device_id": device.device_id,
                // Plaintext bearer — the mobile client must store this; it
                // is never returned again. Replaces the daemon master
                // `api_key` that earlier revisions handed out, so revoking
                // a device via DELETE /api/pairing/devices/{id} now
                // genuinely cuts off its access.
                "api_key": plaintext_key,
                "display_name": device.display_name,
                "platform": device.platform,
                "paired_at": device.paired_at.to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => {
            // Return 410 Gone for used/expired tokens to let the client
            // distinguish "token consumed" from a generic 400 input error.
            (
                axum::http::StatusCode::GONE,
                Json(serde_json::json!({"error": e})),
            )
                .into_response()
        }
    }
}

/// GET /api/pairing/devices — List paired devices.
#[utoipa::path(get, path = "/api/pairing/devices", tag = "pairing", responses((status = 200, description = "List paired devices", body = Vec<serde_json::Value>)))]
pub async fn pairing_devices(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    let devices: Vec<_> = state
        .kernel
        .pairing_ref()
        .list_devices()
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "device_id": d.device_id,
                "display_name": d.display_name,
                "platform": d.platform,
                "paired_at": d.paired_at.to_rfc3339(),
                "last_seen": d.last_seen.to_rfc3339(),
            })
        })
        .collect();
    Json(serde_json::json!({"devices": devices})).into_response()
}

/// DELETE /api/pairing/devices/{id} — Remove a paired device.
#[utoipa::path(delete, path = "/api/pairing/devices/{id}", tag = "pairing", params(("id" = String, Path, description = "Device ID")), responses((status = 200, description = "Device removed")))]
pub async fn pairing_remove_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(t.t("api-error-pairing-not-enabled"))
            .into_json_tuple()
            .into_response();
    }
    let result = state.kernel.pairing_ref().remove_device(&device_id);
    // ErrorTranslator is !Send; drop before any .await below.
    drop(t);
    match result {
        Ok(()) => {
            // Drop this device's bearer from the live auth table so a
            // revoked device's stored key stops authenticating immediately
            // — the persisted device row was just deleted, but without
            // this the in-memory `Vec<ApiUserAuth>` would keep accepting
            // the token until the next process restart.
            let device_user_name = format!("device:{device_id}");
            state
                .user_api_keys
                .write()
                .await
                .retain(|u| u.name != device_user_name);
            tracing::info!(
                target: "pairing.audit",
                device_id = %device_id,
                "revoked paired device — bearer removed from live auth table"
            );
            // DELETE returns 204 No Content with no body (#3843).
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => ApiErrorResponse::not_found(e)
            .into_json_tuple()
            .into_response(),
    }
}

/// POST /api/pairing/notify — Push a notification to all paired devices.
#[utoipa::path(post, path = "/api/pairing/notify", tag = "pairing", request_body = crate::types::JsonObject, responses((status = 200, description = "Notification sent", body = crate::types::JsonObject)))]
pub async fn pairing_notify(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (err_pairing_not_enabled, err_message_required) = {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        (
            t.t("api-error-pairing-not-enabled"),
            t.t("api-error-pairing-message-required"),
        )
    };
    if !state.kernel.config_ref().pairing.enabled {
        return ApiErrorResponse::not_found(err_pairing_not_enabled)
            .into_json_tuple()
            .into_response();
    }
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("LibreFang");
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return ApiErrorResponse::bad_request(err_message_required)
            .into_json_tuple()
            .into_response();
    }
    state
        .kernel
        .pairing_ref()
        .notify_devices(title, message)
        .await;
    Json(serde_json::json!({"ok": true, "notified": state.kernel.pairing_ref().list_devices().len()}))
        .into_response()
}
#[cfg(test)]
mod pairing_tests {
    use super::*;
    use axum::http::HeaderMap;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn configured_url_takes_precedence_over_host_header() {
        let h = headers(&[("x-forwarded-proto", "https")]);
        let resolved =
            resolve_pairing_base_url(Some("https://configured.example.com"), &h, "host.local")
                .unwrap();
        assert_eq!(resolved, "https://configured.example.com");
    }

    #[test]
    fn configured_url_must_have_scheme() {
        let h = HeaderMap::new();
        let err =
            resolve_pairing_base_url(Some("librefang.example.com"), &h, "host.local").unwrap_err();
        assert!(err.contains("must start with http://"), "got: {err}");
    }

    #[test]
    fn configured_url_rejects_non_http_scheme() {
        let h = HeaderMap::new();
        let err =
            resolve_pairing_base_url(Some("ftp://nope.example.com"), &h, "host.local").unwrap_err();
        assert!(err.contains("must start with"), "got: {err}");
    }

    #[test]
    fn configured_url_trailing_slash_trimmed() {
        let h = HeaderMap::new();
        let resolved = resolve_pairing_base_url(Some("https://x.example.com/"), &h, "").unwrap();
        assert_eq!(resolved, "https://x.example.com");
    }

    #[test]
    fn empty_configured_falls_back_to_host_with_default_scheme() {
        let h = HeaderMap::new();
        let resolved = resolve_pairing_base_url(Some(""), &h, "host.local:4545").unwrap();
        assert_eq!(resolved, "http://host.local:4545");
    }

    #[test]
    fn host_fallback_honors_x_forwarded_proto_https() {
        let h = headers(&[("x-forwarded-proto", "https")]);
        let resolved = resolve_pairing_base_url(None, &h, "host.local").unwrap();
        assert_eq!(resolved, "https://host.local");
    }

    #[test]
    fn host_fallback_handles_multi_value_x_forwarded_proto() {
        // Some proxies append values: take the first.
        let h = headers(&[("x-forwarded-proto", "https, http")]);
        let resolved = resolve_pairing_base_url(None, &h, "host.local").unwrap();
        assert_eq!(resolved, "https://host.local");
    }

    #[test]
    fn host_fallback_blank_x_forwarded_proto_does_not_yield_double_colon() {
        // Header present but empty must NOT produce "://host".
        let h = headers(&[("x-forwarded-proto", "")]);
        let resolved = resolve_pairing_base_url(None, &h, "host.local").unwrap();
        assert_eq!(resolved, "http://host.local");
    }

    #[test]
    fn missing_host_and_configured_returns_err() {
        let h = HeaderMap::new();
        let err = resolve_pairing_base_url(None, &h, "").unwrap_err();
        assert!(err.contains("missing Host header"), "got: {err}");
    }

    #[test]
    fn pairing_complete_request_rejects_missing_token() {
        let json = serde_json::json!({"display_name": "x", "platform": "ios"});
        let parsed: Result<PairingCompleteRequest, _> = serde_json::from_value(json);
        assert!(parsed.is_err(), "missing token should fail to deserialize");
    }

    #[test]
    fn pairing_complete_request_defaults_unknown() {
        let json = serde_json::json!({"token": "abc"});
        let parsed: PairingCompleteRequest = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.token, "abc");
        assert_eq!(parsed.display_name, "unknown");
        assert_eq!(parsed.platform, "unknown");
        assert!(parsed.push_token.is_none());
    }

    #[test]
    fn pairing_complete_request_accepts_full_payload() {
        let json = serde_json::json!({
            "token": "tok",
            "display_name": "My iPhone",
            "platform": "ios",
            "push_token": "fcm-xyz",
        });
        let parsed: PairingCompleteRequest = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.token, "tok");
        assert_eq!(parsed.display_name, "My iPhone");
        assert_eq!(parsed.platform, "ios");
        assert_eq!(parsed.push_token.as_deref(), Some("fcm-xyz"));
    }
}
