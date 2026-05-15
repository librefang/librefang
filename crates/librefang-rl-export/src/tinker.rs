//! Tinker trajectory exporter.
//!
//! Tinker (<https://thinkingmachines.ai/tinker/>) is Thinking Machines'
//! distributed LLM post-training API. Unlike W&B — which has a dedicated
//! "create run + upload file" pair — Tinker's REST surface is built around
//! **training calls** (`/api/v1/forward`, `/api/v1/forward_backward`,
//! `/api/v1/optim_step`) and **session-scoped telemetry**. There is no
//! Tinker-side "give me opaque trajectory bytes for this run" endpoint
//! today.
//!
//! The closest two-call pair Tinker exposes that maps cleanly to the W&B
//! "register the run, then post the data" flow is:
//!
//! 1. `POST {base}/api/v1/create_session` — register a new client session
//!    on the Tinker side and recover its server-assigned `session_id`.
//! 2. `POST {base}/api/v1/telemetry` — submit a single `GenericEvent`
//!    under that session whose `event_data` carries the base64-encoded
//!    opaque trajectory bytes plus the rollout window timestamps.
//!
//! Authentication is `X-API-Key: <api_key>` per Tinker's
//! `ApiKeyAuthProvider` (the SDK rejects keys that don't start with
//! `tml-`; this crate forwards the key verbatim and lets the upstream
//! enforce the prefix). See the Tinker SDK source at
//! <https://github.com/thinking-machines-lab/tinker/blob/main/src/tinker/lib/_auth_token_provider.py>
//! and the REST resources at
//! <https://github.com/thinking-machines-lab/tinker/tree/main/src/tinker/resources>.
//!
//! # Assumption flagged for maintainer review
//!
//! Tinker's public docs describe the SDK, not a stable upload surface
//! for opaque rollout bytes — the `create_session + telemetry` pair is
//! the closest stable match against the current SDK source. If Tinker
//! ships a dedicated trajectory endpoint in a future release, this
//! module should switch to it; until then the telemetry-event path is
//! what the upstream actually accepts. See the PR body for sign-off.
//!
//! All HTTP traffic flows through `librefang_http::proxied_client()` so
//! the operator's `[proxy]` config and TLS fallback apply uniformly.

use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{error::ExportError, ExportReceipt, TrajectoryExport};

/// Default Tinker REST base URL. Mirrors the value the Tinker Python
/// SDK falls back to when `TINKER_BASE_URL` is unset
/// (<https://github.com/thinking-machines-lab/tinker/blob/main/src/tinker/_client.py>).
/// Tests override via `export_to_tinker_with_base`.
const DEFAULT_TINKER_BASE: &str = "https://tinker.thinkingmachines.dev/services/tinker-prod";

/// Maximum upstream response body size we keep on an error. Matches the
/// W&B exporter so error sizes are bounded uniformly.
const MAX_ERROR_BODY_BYTES: usize = 4096;

/// SDK-version string we report to Tinker on every call. Tinker accepts
/// arbitrary version strings (the SDK uses its own crate version); we
/// identify ourselves so an operator can grep Tinker-side telemetry for
/// LibreFang-originated sessions.
const LIBREFANG_SDK_VERSION: &str = concat!("librefang-rl-export/", env!("CARGO_PKG_VERSION"));

/// Platform string sent in the telemetry envelope. Low-cardinality on
/// purpose — Tinker's telemetry schema treats `platform` as a label.
const LIBREFANG_PLATFORM: &str = "librefang";

/// Wire shape of the Tinker "create session" request body. Matches the
/// Stainless-generated SDK's `CreateSessionRequest` (tags, user_metadata,
/// sdk_version, project_id, type discriminator).
#[derive(Debug, Serialize)]
struct CreateSessionRequest<'a> {
    tags: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_metadata: Option<&'a serde_json::Value>,
    sdk_version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<&'a str>,
    /// Discriminator; the SDK pins this to the literal `"create_session"`.
    #[serde(rename = "type")]
    request_type: &'static str,
}

/// Wire shape of the Tinker "create session" response. We consume
/// `session_id`; the other fields (`info_message`, `warning_message`,
/// `error_message`, `type`) are accepted but unused.
#[derive(Debug, Deserialize)]
struct CreateSessionResponse {
    session_id: String,
}

/// Wire shape of a Tinker `GenericEvent`. Mirrors the Stainless-generated
/// SDK type at
/// <https://github.com/thinking-machines-lab/tinker/blob/main/src/tinker/types/generic_event.py>.
#[derive(Debug, Serialize)]
struct GenericEvent<'a> {
    event: &'static str,
    event_id: String,
    event_name: &'static str,
    event_session_index: u64,
    severity: &'static str,
    timestamp: String,
    event_data: serde_json::Value,
    #[serde(rename = "type")]
    event_type_marker: &'a str,
}

/// Wire shape of the Tinker telemetry request body.
#[derive(Debug, Serialize)]
struct TelemetrySendRequest<'a> {
    events: Vec<GenericEvent<'a>>,
    platform: &'static str,
    sdk_version: &'a str,
    session_id: &'a str,
}

/// Export a trajectory to Tinker. Internal entry point; the public
/// `crate::export` dispatch matches on `ExportTarget::Tinker` and calls
/// in here.
pub(crate) async fn export_to_tinker(
    project: &str,
    api_key: &str,
    base_url_override: Option<&str>,
    export: TrajectoryExport,
) -> Result<ExportReceipt, ExportError> {
    let base = base_url_override.unwrap_or(DEFAULT_TINKER_BASE);
    export_to_tinker_with_base(base, project, api_key, export).await
}

/// Same as `export_to_tinker` but with a caller-supplied base URL.
/// Exposed at `pub(crate)` so the in-crate wiremock tests can point at
/// a `MockServer::uri()`; production callers go through the public
/// `crate::export` surface.
pub(crate) async fn export_to_tinker_with_base(
    base: &str,
    project: &str,
    api_key: &str,
    export: TrajectoryExport,
) -> Result<ExportReceipt, ExportError> {
    if api_key.is_empty() {
        return Err(ExportError::InvalidConfig(
            "Tinker api_key is empty".to_string(),
        ));
    }
    if project.is_empty() {
        return Err(ExportError::InvalidConfig(
            "Tinker project is empty".to_string(),
        ));
    }

    let client = librefang_http::proxied_client();

    // Step 1: register the session on Tinker.
    let create_url = format!("{}/api/v1/create_session", base.trim_end_matches('/'));
    let create_body = CreateSessionRequest {
        tags: vec!["librefang-rollout", project],
        user_metadata: export.toolset_metadata.as_ref(),
        sdk_version: LIBREFANG_SDK_VERSION,
        project_id: Some(project),
        request_type: "create_session",
    };

    let create_resp = client
        .post(&create_url)
        .header("x-api-key", api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&create_body)
        .send()
        .await?;

    let create_status = create_resp.status();
    if !create_status.is_success() {
        let body = read_body_truncated(create_resp).await;
        return Err(classify_status(create_status.as_u16(), body));
    }

    let create_json: CreateSessionResponse = create_resp
        .json()
        .await
        .map_err(|e| ExportError::MalformedResponse(format!("create-session JSON: {e}")))?;

    // Step 2: submit the trajectory as a single GenericEvent under the
    // newly created session. Tinker's telemetry endpoint accepts arbitrary
    // structured event_data; we base64 the opaque bytes so the JSON
    // envelope stays valid regardless of the producer's chosen wire
    // format (cf. #3330).
    let bytes_len = export.trajectory_bytes.len() as u64;
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(export.trajectory_bytes.as_slice());

    let event_data = serde_json::json!({
        "rollout_run_id": export.run_id,
        "trajectory_bytes_b64": encoded,
        "trajectory_bytes_len": bytes_len,
        "started_at": export.started_at.to_rfc3339(),
        "finished_at": export.finished_at.to_rfc3339(),
    });
    let event = GenericEvent {
        event: "generic",
        event_id: export.run_id.clone(),
        event_name: "librefang.rollout.trajectory",
        event_session_index: 0,
        severity: "INFO",
        timestamp: export.finished_at.to_rfc3339(),
        event_data,
        event_type_marker: "generic_event",
    };
    let telemetry_body = TelemetrySendRequest {
        events: vec![event],
        platform: LIBREFANG_PLATFORM,
        sdk_version: LIBREFANG_SDK_VERSION,
        session_id: &create_json.session_id,
    };

    let upload_url = format!("{}/api/v1/telemetry", base.trim_end_matches('/'));
    let upload_resp = client
        .post(&upload_url)
        .header("x-api-key", api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&telemetry_body)
        .send()
        .await?;

    let upload_status = upload_resp.status();
    if !upload_status.is_success() {
        let body = read_body_truncated(upload_resp).await;
        return Err(classify_status(upload_status.as_u16(), body));
    }

    // Tinker has no run-URL concept; the closest browser-loadable handle
    // is the session id itself. Surface a stable session URL pattern so
    // operators can wire it through; the literal path mirrors the
    // Tinker SDK's `service.get_session(session_id)` convention.
    // Percent-encode the session_id segment — Tinker's id is opaque
    // server-side and there is no documented charset guarantee.
    let target_run_url = format!(
        "{}/api/v1/get_session/{}",
        base.trim_end_matches('/'),
        urlencoding::encode(&create_json.session_id),
    );

    Ok(ExportReceipt {
        target_run_url,
        bytes_uploaded: bytes_len,
        uploaded_at: Utc::now(),
    })
}

/// Map an HTTP status + body to the appropriate `ExportError` variant.
/// Mirrors `wandb::classify_status` exactly so the error surface stays
/// uniform across exporters.
fn classify_status(status: u16, body: String) -> ExportError {
    if status == 401 || status == 403 {
        ExportError::AuthError
    } else {
        ExportError::UpstreamRejected { status, body }
    }
}

/// Read an error response body, truncating to `MAX_ERROR_BODY_BYTES`.
async fn read_body_truncated(resp: reqwest::Response) -> String {
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return format!("<error reading body: {e}>"),
    };
    let slice = if bytes.len() > MAX_ERROR_BODY_BYTES {
        &bytes[..MAX_ERROR_BODY_BYTES]
    } else {
        &bytes[..]
    };
    String::from_utf8_lossy(slice).into_owned()
}

#[cfg(test)]
mod tests {
    //! Tinker exporter tests. Each test stands up a `wiremock::MockServer`
    //! and points `export_to_tinker_with_base` at it. The mocks pin the
    //! two endpoints, the `X-API-Key` auth shape, and the receipt shape.
    //! The test structure mirrors `wandb::tests` so the two exporters
    //! evolve uniformly.
    use super::*;
    use chrono::TimeZone;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_export(run_id: &str) -> TrajectoryExport {
        TrajectoryExport {
            run_id: run_id.to_string(),
            trajectory_bytes: b"opaque-trajectory-bytes".to_vec(),
            toolset_metadata: Some(serde_json::json!({"tools": ["shell", "fetch"]})),
            started_at: Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap(),
            finished_at: Utc.with_ymd_and_hms(2026, 5, 14, 10, 42, 0).unwrap(),
        }
    }

    #[tokio::test]
    async fn export_happy_path_creates_session_then_submits_telemetry() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/create_session"))
            .and(header("x-api-key", "tml-test-key"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "type": "create_session",
                "session_id": "srv-assigned-session-42",
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v1/telemetry"))
            .and(header("x-api-key", "tml-test-key"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "accepted",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let receipt = export_to_tinker_with_base(
            &server.uri(),
            "rl-proj",
            "tml-test-key",
            sample_export("client-run-id"),
        )
        .await
        .expect("export must succeed against mock");

        assert_eq!(
            receipt.target_run_url,
            format!(
                "{}/api/v1/get_session/srv-assigned-session-42",
                server.uri()
            ),
            "receipt url must echo the server-assigned session id, not the client run hint",
        );
        assert_eq!(
            receipt.bytes_uploaded,
            b"opaque-trajectory-bytes".len() as u64,
            "bytes_uploaded must equal payload length",
        );
    }

    #[tokio::test]
    async fn export_forwards_trajectory_bytes_as_base64_event_data() {
        // Pins the *shape* of the telemetry event_data payload so a future
        // refactor cannot silently switch the wire format (the bytes must
        // round-trip through standard base64, not URL-safe base64 or raw
        // hex).
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/create_session"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "type": "create_session",
                "session_id": "s",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let expected_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"opaque-trajectory-bytes");
        // `body_partial_json` asserts that the request body is a superset
        // of this template — every key/value here must match, and extras
        // (timestamps, severity, etc.) are ignored. This pins the
        // base64 wire shape + the session id linkage without coupling to
        // the rest of the envelope.
        Mock::given(method("POST"))
            .and(path("/api/v1/telemetry"))
            .and(body_partial_json(serde_json::json!({
                "session_id": "s",
                "platform": LIBREFANG_PLATFORM,
                "events": [{
                    "event_data": {
                        "trajectory_bytes_b64": expected_b64,
                        "trajectory_bytes_len": 23,
                        "rollout_run_id": "rid",
                    },
                }],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "accepted",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let receipt =
            export_to_tinker_with_base(&server.uri(), "rl-proj", "tml-key", sample_export("rid"))
                .await
                .expect("export must succeed and match schema");
        assert_eq!(receipt.bytes_uploaded, 23);
    }

    #[tokio::test]
    async fn export_maps_401_to_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/create_session"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid api key"))
            .expect(1)
            .mount(&server)
            .await;

        let err =
            export_to_tinker_with_base(&server.uri(), "rl-proj", "bogus-key", sample_export("rid"))
                .await
                .expect_err("401 must surface as ExportError");
        assert!(matches!(err, ExportError::AuthError), "got {err:?}");
    }

    #[tokio::test]
    async fn export_maps_other_4xx_to_upstream_rejected_with_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/create_session"))
            .respond_with(ResponseTemplate::new(422).set_body_string("invalid session payload"))
            .expect(1)
            .mount(&server)
            .await;

        let err =
            export_to_tinker_with_base(&server.uri(), "rl-proj", "tml-key", sample_export("rid"))
                .await
                .expect_err("422 must surface as UpstreamRejected");
        match err {
            ExportError::UpstreamRejected { status, body } => {
                assert_eq!(status, 422);
                assert!(body.contains("invalid session payload"), "body={body}");
            }
            other => panic!("expected UpstreamRejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_api_key_is_rejected_before_any_http() {
        // No MockServer needed — InvalidConfig must fire before we touch
        // the network. We use a bogus base URL to prove no I/O happens.
        let err = export_to_tinker_with_base(
            "http://will.not.be.contacted.invalid",
            "rl-proj",
            "",
            sample_export("rid"),
        )
        .await
        .expect_err("empty api key must be rejected up front");
        assert!(matches!(err, ExportError::InvalidConfig(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn empty_project_is_rejected_before_any_http() {
        let err = export_to_tinker_with_base(
            "http://will.not.be.contacted.invalid",
            "",
            "tml-key",
            sample_export("rid"),
        )
        .await
        .expect_err("empty project must be rejected up front");
        assert!(matches!(err, ExportError::InvalidConfig(_)), "got {err:?}");
    }
}
