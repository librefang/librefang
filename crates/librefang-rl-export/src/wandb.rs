//! Weights & Biases trajectory exporter.
//!
//! Implements the RL trajectory export path against the W&B REST surface
//! (<https://docs.wandb.ai/ref/api/rest/>). The flow has two HTTP calls:
//!
//! 1. `POST <base>/api/runs` to create / register the run on the W&B
//!    side and recover its server-assigned URL. The request includes the
//!    project and (optional) entity so W&B can scope the run correctly.
//! 2. `POST <base>/files/<entity>/<project>/<run_id>` to upload the
//!    opaque trajectory bytes as a single artefact attached to the run.
//!    The body is the raw `Vec<u8>` payload — wire format is owned by
//!    the producer (see #3330) and this crate stays format-agnostic.
//!
//! Authentication uses W&B's bare-API-key convention encoded into HTTP
//! Basic with the empty user component: `Authorization: Basic
//! base64("api:<key>")`. The leading `api:` is the W&B-documented user
//! placeholder — the API key itself is the password.
//!
//! All HTTP traffic flows through `librefang_http::proxied_client()` so
//! the operator's `[proxy]` config and TLS fallback apply uniformly with
//! every other outbound caller in the workspace (per the
//! `librefang-extensions` crate's HTTP client convention).

use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    error::{classify_response_decode_error, classify_status, read_body_truncated, ExportError},
    ExportReceipt, RlTrajectoryExport,
};

/// Default W&B API base URL. Tests override via `export_to_wandb_with_base`.
const DEFAULT_WANDB_BASE: &str = "https://api.wandb.ai";

/// Wire shape of the W&B "create run" request body. Field names match
/// the REST documentation; optional fields are omitted via
/// `skip_serializing_if`.
#[derive(Debug, Serialize)]
struct CreateRunRequest<'a> {
    project: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    entity: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<&'a str>,
    /// ISO-8601 RFC3339 start time for the run window.
    started_at: String,
    /// ISO-8601 RFC3339 finish time. W&B accepts an already-completed
    /// run window so the rollout-side caller can post a single export
    /// after the trajectory finishes.
    finished_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'a serde_json::Value>,
}

/// Wire shape of the W&B "create run" response. We only consume
/// `run_id` and `url`; any additional fields are ignored.
#[derive(Debug, Deserialize)]
struct CreateRunResponse {
    run_id: String,
    url: String,
}

/// Export a trajectory to W&B. Internal entry point; the public
/// `crate::export` dispatch matches on `ExportTarget::WandB` and calls
/// in here.
pub(crate) async fn export_to_wandb(
    project: &str,
    entity: Option<&str>,
    run_id_hint: Option<&str>,
    api_key: &str,
    export: RlTrajectoryExport,
) -> Result<ExportReceipt, ExportError> {
    export_to_wandb_with_base(
        DEFAULT_WANDB_BASE,
        project,
        entity,
        run_id_hint,
        api_key,
        export,
    )
    .await
}

/// Same as `export_to_wandb` but with a caller-supplied base URL.
/// Exposed at `pub(crate)` so the in-crate wiremock tests can point at
/// a `MockServer::uri()`; production callers go through the public
/// `crate::export` surface which always uses `DEFAULT_WANDB_BASE`.
pub(crate) async fn export_to_wandb_with_base(
    base: &str,
    project: &str,
    entity: Option<&str>,
    run_id_hint: Option<&str>,
    api_key: &str,
    export: RlTrajectoryExport,
) -> Result<ExportReceipt, ExportError> {
    if api_key.is_empty() {
        return Err(ExportError::InvalidConfig(
            "W&B api_key is empty".to_string(),
        ));
    }
    if project.is_empty() {
        return Err(ExportError::InvalidConfig(
            "W&B project is empty".to_string(),
        ));
    }

    let client = librefang_http::proxied_client();
    let auth_header = build_basic_auth(api_key);

    // Step 1: create / register the run.
    let create_url = format!("{}/api/runs", base.trim_end_matches('/'));
    let create_body = CreateRunRequest {
        project,
        entity,
        run_id: run_id_hint.or(Some(export.run_id.as_str())),
        started_at: export.started_at.to_rfc3339(),
        finished_at: export.finished_at.to_rfc3339(),
        metadata: export.toolset_metadata.as_ref(),
    };

    let create_resp = client
        .post(&create_url)
        .header(reqwest::header::AUTHORIZATION, &auth_header)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&create_body)
        .send()
        .await?;

    let create_status = create_resp.status();
    if !create_status.is_success() {
        let body = read_body_truncated(create_resp).await;
        return Err(classify_status(create_status.as_u16(), body));
    }

    let create_json: CreateRunResponse = create_resp
        .json()
        .await
        .map_err(|e| classify_response_decode_error(e, "create-run JSON"))?;

    // The entity used for the upload path: prefer the caller-supplied
    // value; fall back to a literal "default" placeholder for the upload
    // URL when entity is unset (matches W&B's "personal entity" convention
    // for keyless single-user accounts — the server resolves it from the
    // API key). Tests assert the prefer-explicit path.
    let upload_entity = entity.unwrap_or("default");

    // Step 2: upload the trajectory bytes as a file artefact under the
    // newly created run. Each path segment is percent-encoded — entity,
    // project, and run_id are caller-controlled strings and unescaped
    // `/` or reserved characters would otherwise reshape the request
    // path or smuggle a query/fragment.
    let upload_url = format!(
        "{}/files/{}/{}/{}",
        base.trim_end_matches('/'),
        urlencoding::encode(upload_entity),
        urlencoding::encode(project),
        urlencoding::encode(&create_json.run_id),
    );
    let bytes_len = export.trajectory_bytes.len() as u64;

    let upload_resp = client
        .post(&upload_url)
        .header(reqwest::header::AUTHORIZATION, &auth_header)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(export.trajectory_bytes)
        .send()
        .await?;

    let upload_status = upload_resp.status();
    if !upload_status.is_success() {
        let body = read_body_truncated(upload_resp).await;
        return Err(classify_status(upload_status.as_u16(), body));
    }

    Ok(ExportReceipt {
        target_run_url: create_json.url,
        bytes_uploaded: bytes_len,
        uploaded_at: Utc::now(),
    })
}

/// Build the `Authorization: Basic …` header value for W&B.
///
/// W&B's documented convention for the REST API is HTTP Basic with the
/// literal user `api` and the API key as the password. See
/// <https://docs.wandb.ai/ref/api/rest/>. Encoding is standard base64
/// (RFC 4648 §4) of `api:<key>`.
fn build_basic_auth(api_key: &str) -> String {
    let raw = format!("api:{api_key}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
    format!("Basic {encoded}")
}

#[cfg(test)]
mod tests {
    //! W&B exporter tests. Each test stands up a `wiremock::MockServer`
    //! and points `export_to_wandb_with_base` at it. The mocks pin the
    //! two endpoints, the auth header shape, and the receipt shape.
    use super::*;
    use chrono::TimeZone;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_export(run_id: &str) -> RlTrajectoryExport {
        RlTrajectoryExport {
            run_id: run_id.to_string(),
            trajectory_bytes: b"opaque-trajectory-bytes".to_vec(),
            toolset_metadata: Some(serde_json::json!({"tools": ["shell", "fetch"]})),
            started_at: Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap(),
            finished_at: Utc.with_ymd_and_hms(2026, 5, 14, 10, 42, 0).unwrap(),
        }
    }

    #[tokio::test]
    async fn export_happy_path_creates_run_then_uploads_bytes() {
        let server = MockServer::start().await;

        // The mock /api/runs returns the run_id + url the second call needs.
        Mock::given(method("POST"))
            .and(path("/api/runs"))
            .and(header(
                "authorization",
                build_basic_auth("test-key").as_str(),
            ))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "run_id": "server-assigned-run-42",
                "url": "https://wandb.ai/acme/rl-proj/runs/server-assigned-run-42",
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/files/acme/rl-proj/server-assigned-run-42"))
            .and(header(
                "authorization",
                build_basic_auth("test-key").as_str(),
            ))
            .and(header("content-type", "application/octet-stream"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let receipt = export_to_wandb_with_base(
            &server.uri(),
            "rl-proj",
            Some("acme"),
            Some("client-hinted-run-id"),
            "test-key",
            sample_export("client-hinted-run-id"),
        )
        .await
        .expect("export must succeed against mock");

        assert_eq!(
            receipt.target_run_url, "https://wandb.ai/acme/rl-proj/runs/server-assigned-run-42",
            "receipt url must echo the server-assigned URL, not the client hint",
        );
        assert_eq!(
            receipt.bytes_uploaded,
            b"opaque-trajectory-bytes".len() as u64,
            "bytes_uploaded must equal payload length",
        );
    }

    #[tokio::test]
    async fn export_falls_back_to_default_entity_when_unset() {
        // Without an entity, the upload path uses the "default" placeholder
        // and W&B's server resolves the personal entity from the API key.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "run_id": "rid",
                "url": "https://wandb.ai/personal/rl-proj/runs/rid",
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/files/default/rl-proj/rid"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let receipt = export_to_wandb_with_base(
            &server.uri(),
            "rl-proj",
            None,
            None,
            "key",
            sample_export("rid"),
        )
        .await
        .expect("export must succeed with no entity");
        assert_eq!(receipt.bytes_uploaded, 23);
    }

    #[tokio::test]
    async fn export_maps_401_to_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/runs"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid api key"))
            .expect(1)
            .mount(&server)
            .await;

        let err = export_to_wandb_with_base(
            &server.uri(),
            "rl-proj",
            Some("acme"),
            None,
            "bogus-key",
            sample_export("rid"),
        )
        .await
        .expect_err("401 must surface as ExportError");
        assert!(matches!(err, ExportError::AuthError), "got {err:?}");
    }

    #[tokio::test]
    async fn export_maps_other_4xx_to_upstream_rejected_with_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/runs"))
            .respond_with(ResponseTemplate::new(404).set_body_string("project not found"))
            .expect(1)
            .mount(&server)
            .await;

        let err = export_to_wandb_with_base(
            &server.uri(),
            "missing-proj",
            Some("acme"),
            None,
            "k",
            sample_export("rid"),
        )
        .await
        .expect_err("404 must surface as UpstreamRejected");
        match err {
            ExportError::UpstreamRejected { status, body } => {
                assert_eq!(status, 404);
                assert!(body.contains("project not found"), "body={body}");
            }
            other => panic!("expected UpstreamRejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_api_key_is_rejected_before_any_http() {
        // No MockServer needed — InvalidConfig must fire before we touch
        // the network. We use a bogus base URL to prove no I/O happens.
        let err = export_to_wandb_with_base(
            "http://will.not.be.contacted.invalid",
            "rl-proj",
            Some("acme"),
            None,
            "",
            sample_export("rid"),
        )
        .await
        .expect_err("empty api key must be rejected up front");
        assert!(matches!(err, ExportError::InvalidConfig(_)), "got {err:?}");
    }

    #[test]
    fn basic_auth_uses_api_user_placeholder() {
        // Pins W&B's documented "api:<key>" Basic-auth convention so a
        // future refactor cannot silently switch to bare-key or
        // "<key>:".
        let header = build_basic_auth("secret");
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode("api:secret");
        assert_eq!(header, format!("Basic {expected_b64}"));
    }

    /// A 2xx response whose body fails to deserialize into the expected
    /// shape must surface as `MalformedResponse`, not `NetworkError` —
    /// the bytes arrived intact, the upstream just spoke a different
    /// contract. Pins the decode-vs-network split that lives in
    /// `error::classify_response_decode_error`.
    #[tokio::test]
    async fn export_maps_2xx_non_json_body_to_malformed_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
            .expect(1)
            .mount(&server)
            .await;

        let err = export_to_wandb_with_base(
            &server.uri(),
            "rl-proj",
            Some("acme"),
            None,
            "key",
            sample_export("rid"),
        )
        .await
        .expect_err("non-JSON 2xx body must surface as ExportError");
        match err {
            ExportError::MalformedResponse(msg) => {
                assert!(
                    msg.contains("create-run JSON"),
                    "decode error must carry call-site context, got: {msg}",
                );
            }
            other => panic!("expected MalformedResponse, got {other:?}"),
        }
    }

    /// Upload-URL path segments must be percent-encoded. A caller that
    /// passes `entity` / `project` / `run_id` strings containing `/`,
    /// space, or any reserved character must NOT reshape the request
    /// path; the mock asserts the exact encoded path it expects.
    #[tokio::test]
    async fn upload_url_percent_encodes_path_segments() {
        let server = MockServer::start().await;

        // Server-assigned run_id intentionally contains characters that
        // would otherwise corrupt the path: `/`, space, `+`.
        Mock::given(method("POST"))
            .and(path("/api/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "run_id": "run/with space+plus",
                "url": "https://wandb.ai/x/y/runs/r",
            })))
            .expect(1)
            .mount(&server)
            .await;

        // urlencoding::encode renders ` ` as `%20`, `/` as `%2F`, `+`
        // as `%2B`. The wandb-side path must reflect that exactly, or
        // the file upload would land on a different (or invalid) URL.
        Mock::given(method("POST"))
            .and(path(
                "/files/acme%2Fteam/rl%20proj/run%2Fwith%20space%2Bplus",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let receipt = export_to_wandb_with_base(
            &server.uri(),
            "rl proj",
            Some("acme/team"),
            None,
            "k",
            sample_export("rid"),
        )
        .await
        .expect("export must succeed with encoded path");
        assert_eq!(receipt.bytes_uploaded, 23);
    }
}
