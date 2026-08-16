use super::*;

fn security_status_payload(
    rate_limit: &librefang_types::config::RateLimitConfig,
    gcra_tokens_per_minute: u32,
    trusted_manifest_signers: &[String],
    api_key_empty: bool,
    audit_count: usize,
) -> serde_json::Value {
    let auth_mode = if api_key_empty {
        "localhost_only"
    } else {
        "bearer_token"
    };
    let manifest_signing_available = !trusted_manifest_signers.is_empty()
        && trusted_manifest_signers.iter().all(|signer| {
            hex::decode(signer.trim()).is_ok_and(|public_key| public_key.len() == 32)
        });

    serde_json::json!({
        // These are runtime invariants, not operator-configurable switches.
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
                "tokens_per_minute": gcra_tokens_per_minute,
                "algorithm": "GCRA"
            },
            "websocket_limits": {
                "max_per_ip": rate_limit.max_ws_per_ip,
                "idle_timeout_secs": rate_limit.ws_idle_timeout_secs,
                "max_message_size": 64 * 1024,
                "max_messages_per_minute": rate_limit.ws_messages_per_minute
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
                "available": manifest_signing_available
            }
        },
        "secret_zeroization": true,
        "total_features": if manifest_signing_available { 15 } else { 14 }
    })
}

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
    let audit_count = state.kernel.audit().len();
    let config = state.kernel.config_ref();

    Json(security_status_payload(
        &config.rate_limit,
        state.gcra_tokens_per_minute,
        &config.trusted_manifest_signers,
        api_key_empty,
        audit_count,
    ))
}

#[cfg(test)]
mod tests {
    use super::security_status_payload;
    use librefang_types::config::RateLimitConfig;

    #[test]
    fn security_status_reports_applied_limits_and_manifest_trust() {
        let rate_limit = RateLimitConfig {
            api_requests_per_minute: 237,
            max_ws_per_ip: 9,
            ws_messages_per_minute: 41,
            ws_idle_timeout_secs: 73,
            ..RateLimitConfig::default()
        };
        let signers = vec!["00".repeat(32)];

        let status = security_status_payload(&rate_limit, 193, &signers, false, 12);

        assert_eq!(
            status["configurable"]["rate_limiter"]["tokens_per_minute"],
            193
        );
        assert_eq!(status["configurable"]["websocket_limits"]["max_per_ip"], 9);
        assert_eq!(
            status["configurable"]["websocket_limits"]["idle_timeout_secs"],
            73
        );
        assert_eq!(
            status["configurable"]["websocket_limits"]["max_messages_per_minute"],
            41
        );
        assert_eq!(status["monitoring"]["manifest_signing"]["available"], true);
        assert_eq!(status["monitoring"]["audit_trail"]["entry_count"], 12);
        assert_eq!(status["total_features"], 15);
    }

    #[test]
    fn security_status_matches_effective_minimum_quota_and_missing_signers() {
        let rate_limit = RateLimitConfig {
            api_requests_per_minute: 0,
            ..RateLimitConfig::default()
        };

        let status = security_status_payload(&rate_limit, 1, &[], true, 0);

        assert_eq!(
            status["configurable"]["rate_limiter"]["tokens_per_minute"],
            1
        );
        assert_eq!(status["monitoring"]["manifest_signing"]["available"], false);
        assert_eq!(status["configurable"]["auth"]["mode"], "localhost_only");
        assert_eq!(status["total_features"], 14);
    }

    #[test]
    fn security_status_rejects_malformed_manifest_trust_anchors() {
        let rate_limit = RateLimitConfig::default();
        for signers in [
            vec!["not-hex".to_string()],
            vec!["00".repeat(31)],
            vec!["00".repeat(32), "invalid".to_string()],
        ] {
            let status = security_status_payload(&rate_limit, 500, &signers, false, 0);
            assert_eq!(status["monitoring"]["manifest_signing"]["available"], false);
            assert_eq!(status["total_features"], 14);
        }
    }
}
