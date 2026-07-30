use super::*;

// ---------------------------------------------------------------------------
// Migration endpoint
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Security dashboard endpoint
// ---------------------------------------------------------------------------
/// GET /api/security — Security feature status for the dashboard.
#[utoipa::path(
    get,
    path = "/api/security",
    tag = "system",
    responses(
        (status = 200, description = "Security feature status", body = crate::types::JsonObject)
    )
)]
pub async fn security_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Ask the live master-credential handle — the same one the auth middleware
    // verifies against (#6613). Reading `api_key` alone reported
    // `localhost_only` for a hash-only or env-sourced deployment that is in
    // fact fully bearer-gated; re-resolving the field from a config snapshot
    // would report correctly but unlock the vault per request for a
    // `vault:NAME` value, to answer a yes/no question the handle already holds.
    let api_key_empty = !state.master_key.is_configured().await;
    let auth_mode = if api_key_empty {
        "localhost_only"
    } else {
        "bearer_token"
    };

    let audit_count = state.kernel.audit().len();

    Json(serde_json::json!({
        "core_protections": {
            "path_traversal": true,
            "ssrf_protection": true,
            "capability_system": true,
            "privilege_escalation_prevention": true,
            "subprocess_isolation": true,
            "security_headers": true,
            "wire_hmac_auth": true,
            "request_id_tracking": true
        },
        "configurable": {
            "rate_limiter": {
                "enabled": true,
                "tokens_per_minute": 500,
                "algorithm": "GCRA"
            },
            "websocket_limits": {
                "max_per_ip": 5,
                "idle_timeout_secs": 1800,
                "max_message_size": 65536,
                "max_messages_per_minute": 10
            },
            "wasm_sandbox": {
                "fuel_metering": true,
                "epoch_interruption": true,
                "default_timeout_secs": 30,
                "default_fuel_limit": 1_000_000u64
            },
            "auth": {
                "mode": auth_mode,
                "api_key_set": !api_key_empty
            }
        },
        "monitoring": {
            "audit_trail": {
                "enabled": true,
                "algorithm": "SHA-256 Merkle Chain",
                "entry_count": audit_count
            },
            "taint_tracking": {
                "enabled": true,
                "tracked_labels": [
                    "ExternalNetwork",
                    "UserInput",
                    "PII",
                    "Secret",
                    "UntrustedAgent"
                ]
            },
            "manifest_signing": {
                "algorithm": "Ed25519",
                "available": true
            }
        },
        "secret_zeroization": true,
        "total_features": 15
    }))
}
