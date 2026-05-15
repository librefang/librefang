//! Atropos trajectory exporter.
//!
//! Atropos (<https://github.com/NousResearch/atropos>) is NousResearch's
//! LLM RL environments framework — a FastAPI microservice that mediates
//! between **rollout environments** (producers of trajectories) and a
//! **trainer** (consumer of batches). Unlike W&B and Tinker, Atropos is
//! **not a cloud-hosted experiment-tracking service**: the API server is
//! a process the operator runs locally (default `http://localhost:8000`),
//! and there is **no authentication layer at all** — it's a trainer-
//! environment bus, not an external upload target.
//!
//! From this crate's point of view the Atropos exporter is still the
//! same "register + submit" two-step pattern as W&B and Tinker:
//!
//! 1. `POST {base}/register-env` — register this rollout producer with
//!    the running Atropos trainer and recover the server-assigned
//!    `env_id` + `wandb_name`. Body matches `RegisterEnv` in
//!    `atroposlib/api/server.py`
//!    (`max_token_length`, `desired_name`, `weight`, `group_size`,
//!    `min_batch_allocation`).
//! 2. `POST {base}/scored_data` — submit the trajectory as a single
//!    `ScoredData` payload under the registered `env_id`. **The
//!    trajectory bytes must already be valid `ScoredData` JSON**
//!    (`tokens`, `masks`, `scores`, …, possibly `messages`); this
//!    crate forwards the opaque bytes verbatim with
//!    `Content-Type: application/json` and lets Atropos validate. If
//!    the bytes aren't valid `ScoredData`, Atropos returns 422 and we
//!    surface `UpstreamRejected{status: 422, body}` — the producer's
//!    problem, not the exporter's.
//!
//! # Caller-flow assumption flagged for maintainer review
//!
//! Atropos's `/register-env` is gated by `app.state.started`: if the
//! trainer process hasn't called `/register` (a separate, trainer-only
//! endpoint that is NOT part of this exporter's surface), the server
//! returns HTTP 200 with the **sentinel body**
//! `{"status": "wait for trainer to start"}` and *no* `env_id` field.
//! This crate detects the missing `env_id` and converts that case into
//! `UpstreamRejected { status: 503, body }` so callers see a
//! retry-after-trainer-up signal rather than a `MalformedResponse`.
//!
//! Atropos's design assumption is that the trainer is running before
//! any environment connects; the exporter is a producer, not a trainer,
//! so this crate does not try to bring up `/register` itself. Operators
//! deploy the Atropos trainer separately and point this exporter at it.
//! See <https://github.com/NousResearch/atropos/blob/main/atroposlib/api/server.py>
//! and `atroposlib/api/env_interaction.md` for the producer-side flow.
//!
//! All HTTP traffic flows through `librefang_http::proxied_client()`.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    error::{classify_status, read_body_truncated, ExportError},
    ExportReceipt, TrajectoryExport,
};

/// Default Atropos REST base URL. Matches the Atropos `run-api` default
/// documented in the project's README (the server binds to
/// `http://localhost:8000` unless overridden by CLI flags). Tests
/// override via `export_to_atropos_with_base`.
const DEFAULT_ATROPOS_BASE: &str = "http://localhost:8000";

/// Wire shape of the Atropos `RegisterEnv` request body. Mirrors the
/// `RegisterEnv` Pydantic model in
/// <https://github.com/NousResearch/atropos/blob/main/atroposlib/api/server.py>.
/// `min_batch_allocation` is `Option<f32>` because the server-side
/// field is optional and defaults to None.
#[derive(Debug, Serialize)]
struct RegisterEnvRequest<'a> {
    max_token_length: u32,
    desired_name: &'a str,
    weight: f32,
    group_size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_batch_allocation: Option<f32>,
}

/// Wire shape of the Atropos `register-env` response. We consume
/// `env_id` and `wandb_name`; other fields (`checkpoint_dir`,
/// `starting_step`, `checkpoint_interval`, `num_steps`,
/// `status`) are accepted but unused.
///
/// The Atropos server overloads this response shape: when the trainer
/// hasn't yet started, it returns `{"status": "wait for trainer to
/// start"}` with no `env_id` — both fields land in `None` and the
/// exporter surfaces a synthetic `UpstreamRejected`. See module docs.
#[derive(Debug, Deserialize)]
struct RegisterEnvResponse {
    #[serde(default)]
    env_id: Option<u64>,
    #[serde(default)]
    wandb_name: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Default registration knobs. Atropos's `RegisterEnv` is heavily
/// trainer-side-tuned (`group_size`, `min_batch_allocation`, …);
/// since we're exporting a finished trajectory rather than running a
/// live producer loop, we register with conservative defaults and let
/// the trainer assign whatever `env_id` it sees fit. Operators who
/// need different tuning should call the upstream API directly.
const DEFAULT_MAX_TOKEN_LENGTH: u32 = 32_768;
const DEFAULT_GROUP_SIZE: u32 = 1;
const DEFAULT_WEIGHT: f32 = 1.0;

/// Export a trajectory to Atropos. Internal entry point; the public
/// `crate::export` dispatch matches on `ExportTarget::Atropos` and
/// calls in here.
pub(crate) async fn export_to_atropos(
    project: &str,
    base_url_override: Option<&str>,
    export: TrajectoryExport,
) -> Result<ExportReceipt, ExportError> {
    let base = base_url_override.unwrap_or(DEFAULT_ATROPOS_BASE);
    export_to_atropos_with_base(base, project, export).await
}

/// Same as `export_to_atropos` but with a caller-supplied base URL.
/// Exposed at `pub(crate)` so the in-crate wiremock tests can point at
/// a `MockServer::uri()`; production callers go through the public
/// `crate::export` surface.
pub(crate) async fn export_to_atropos_with_base(
    base: &str,
    project: &str,
    export: TrajectoryExport,
) -> Result<ExportReceipt, ExportError> {
    if project.is_empty() {
        return Err(ExportError::InvalidConfig(
            "Atropos project (desired_name) is empty".to_string(),
        ));
    }
    if export.trajectory_bytes.is_empty() {
        return Err(ExportError::InvalidConfig(
            "Atropos export trajectory_bytes is empty".to_string(),
        ));
    }

    let client = librefang_http::proxied_client();

    // Step 1: register this producer with the running Atropos trainer.
    let register_url = format!("{}/register-env", base.trim_end_matches('/'));
    let register_body = RegisterEnvRequest {
        max_token_length: DEFAULT_MAX_TOKEN_LENGTH,
        desired_name: project,
        weight: DEFAULT_WEIGHT,
        group_size: DEFAULT_GROUP_SIZE,
        min_batch_allocation: None,
    };

    let register_resp = client
        .post(&register_url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&register_body)
        .send()
        .await?;

    let register_status = register_resp.status();
    if !register_status.is_success() {
        let body = read_body_truncated(register_resp).await;
        return Err(classify_status(register_status.as_u16(), body));
    }

    let register_json: RegisterEnvResponse = register_resp
        .json()
        .await
        .map_err(|e| ExportError::MalformedResponse(format!("register-env JSON: {e}")))?;

    // Atropos returns 200 with `{"status": "wait for trainer to start"}`
    // and no env_id when the trainer side hasn't booted yet. Convert
    // that overloaded 200-as-busy into a synthetic 503 UpstreamRejected
    // so the caller has a clear retry-after-trainer-up signal.
    let env_id = match register_json.env_id {
        Some(id) => id,
        None => {
            let status_label = register_json
                .status
                .unwrap_or_else(|| "unknown".to_string());
            return Err(ExportError::UpstreamRejected {
                status: 503,
                body: format!("atropos trainer not ready: {status_label}"),
            });
        }
    };

    // Step 2: submit the trajectory bytes as a ScoredData payload.
    // trajectory_bytes is opaque (see #3330) and must already be valid
    // ScoredData JSON; if it's not, Atropos returns 422 and we surface
    // UpstreamRejected{422, body}.
    let bytes_len = export.trajectory_bytes.len() as u64;
    let upload_url = format!("{}/scored_data", base.trim_end_matches('/'));
    let upload_resp = client
        .post(&upload_url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(export.trajectory_bytes)
        .send()
        .await?;

    let upload_status = upload_resp.status();
    if !upload_status.is_success() {
        let body = read_body_truncated(upload_resp).await;
        return Err(classify_status(upload_status.as_u16(), body));
    }

    // Atropos has no concept of a browser-loadable "run URL" — the API
    // is a local microservice. The closest stable handle for the
    // operator is the env's `wandb_name` (server-assigned during
    // register-env: `<desired_name>_<index>`) under the configured
    // `/latest_example` debug endpoint. Surface that so an operator
    // running `curl {base}/latest_example` can verify the upload
    // landed.
    let wandb_name = register_json
        .wandb_name
        .unwrap_or_else(|| format!("{project}_{env_id}"));
    // Encode the fragment payload: `wandb_name` can carry `#`, `&`, or
    // other reserved characters once Atropos appends its index suffix,
    // and an unescaped one would corrupt the rendered URL the operator
    // copies out of the receipt.
    let target_run_url = format!(
        "{}/latest_example#env={}",
        base.trim_end_matches('/'),
        urlencoding::encode(&wandb_name),
    );

    Ok(ExportReceipt {
        target_run_url,
        bytes_uploaded: bytes_len,
        uploaded_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    //! Atropos exporter tests. Same `wiremock::MockServer` shape as the
    //! W&B / Tinker tests so the three exporters evolve uniformly.
    use super::*;
    use chrono::TimeZone;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a sample export. Atropos consumes the trajectory bytes as a
    /// `ScoredData` JSON payload, so the sample bytes are a minimal
    /// valid `ScoredData` blob.
    fn sample_export(run_id: &str) -> TrajectoryExport {
        let scored_data = serde_json::json!({
            "tokens": [[1, 2, 3]],
            "masks": [[1, 1, 1]],
            "scores": [0.5],
        });
        let bytes = serde_json::to_vec(&scored_data).unwrap();
        TrajectoryExport {
            run_id: run_id.to_string(),
            trajectory_bytes: bytes,
            toolset_metadata: None,
            started_at: Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap(),
            finished_at: Utc.with_ymd_and_hms(2026, 5, 14, 10, 42, 0).unwrap(),
        }
    }

    #[tokio::test]
    async fn export_happy_path_registers_env_then_submits_scored_data() {
        let server = MockServer::start().await;

        // /register-env returns env_id + wandb_name; the body shape pin
        // proves the request matches Atropos's RegisterEnv schema.
        Mock::given(method("POST"))
            .and(path("/register-env"))
            .and(body_partial_json(serde_json::json!({
                "desired_name": "rl-proj",
                "group_size": 1,
                "weight": 1.0,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "success",
                "env_id": 7,
                "wandb_name": "rl-proj_3",
                "checkpoint_dir": "/tmp/atropos-ckpt",
                "starting_step": 0,
                "checkpoint_interval": 100,
                "num_steps": 1000,
            })))
            .expect(1)
            .mount(&server)
            .await;

        // /scored_data accepts the opaque bytes verbatim. The
        // body_partial_json check pins that the producer-side ScoredData
        // shape survives the round-trip (tokens / scores forwarded as-is).
        Mock::given(method("POST"))
            .and(path("/scored_data"))
            .and(body_partial_json(serde_json::json!({
                "tokens": [[1, 2, 3]],
                "scores": [0.5],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "received",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let receipt = export_to_atropos_with_base(&server.uri(), "rl-proj", sample_export("rid"))
            .await
            .expect("export must succeed against mock");

        assert_eq!(
            receipt.target_run_url,
            format!("{}/latest_example#env=rl-proj_3", server.uri()),
            "receipt url must point at /latest_example with the server-assigned wandb_name",
        );
        assert!(
            receipt.bytes_uploaded > 0,
            "bytes_uploaded must be > 0 for a non-empty trajectory",
        );
    }

    #[tokio::test]
    async fn export_translates_trainer_not_ready_to_upstream_rejected_503() {
        // Atropos returns HTTP 200 with {"status": "wait for trainer to
        // start"} and NO env_id when the trainer side hasn't booted.
        // This crate converts that overloaded 200-as-busy into a
        // synthetic 503 UpstreamRejected so callers see a clear
        // retry-after-trainer-up signal.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/register-env"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "wait for trainer to start",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let err = export_to_atropos_with_base(&server.uri(), "rl-proj", sample_export("rid"))
            .await
            .expect_err("trainer-not-ready sentinel must surface as ExportError");
        match err {
            ExportError::UpstreamRejected { status, body } => {
                assert_eq!(status, 503);
                assert!(body.contains("wait for trainer to start"), "body={body}");
            }
            other => panic!("expected UpstreamRejected{{503,..}}, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn export_maps_401_to_auth_error_for_proxy_fronted_deployments() {
        // Atropos itself has no auth, but operators sometimes front the
        // run-api with a reverse proxy that enforces it. 401 must still
        // collapse into AuthError so the error surface is uniform
        // across exporters.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/register-env"))
            .respond_with(ResponseTemplate::new(401).set_body_string("proxy: no api key"))
            .expect(1)
            .mount(&server)
            .await;

        let err = export_to_atropos_with_base(&server.uri(), "rl-proj", sample_export("rid"))
            .await
            .expect_err("401 must surface as ExportError");
        assert!(matches!(err, ExportError::AuthError), "got {err:?}");
    }

    #[tokio::test]
    async fn export_maps_422_validation_failure_to_upstream_rejected_with_body() {
        // Atropos's ScoredData validator rejects malformed payloads with
        // 422 + a Pydantic-shaped error body. Verify the status +
        // body forwarding.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/register-env"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "success",
                "env_id": 0,
                "wandb_name": "rl-proj_0",
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/scored_data"))
            .respond_with(ResponseTemplate::new(422).set_body_string(
                "{\"detail\":[{\"loc\":[\"body\",\"tokens\"],\"msg\":\"field required\"}]}",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let err = export_to_atropos_with_base(&server.uri(), "rl-proj", sample_export("rid"))
            .await
            .expect_err("422 must surface as UpstreamRejected");
        match err {
            ExportError::UpstreamRejected { status, body } => {
                assert_eq!(status, 422);
                assert!(body.contains("field required"), "body={body}");
            }
            other => panic!("expected UpstreamRejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_project_is_rejected_before_any_http() {
        // InvalidConfig must fire before we touch the network. We use a
        // bogus base URL to prove no I/O happens.
        let err = export_to_atropos_with_base(
            "http://will.not.be.contacted.invalid",
            "",
            sample_export("rid"),
        )
        .await
        .expect_err("empty project must be rejected up front");
        assert!(matches!(err, ExportError::InvalidConfig(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn empty_trajectory_bytes_is_rejected_before_any_http() {
        // Atropos's ScoredData validator would reject an empty body
        // anyway, but rejecting it locally avoids a pointless round
        // trip and surfaces InvalidConfig (caller config), not
        // UpstreamRejected (server error).
        let mut export = sample_export("rid");
        export.trajectory_bytes.clear();
        let err =
            export_to_atropos_with_base("http://will.not.be.contacted.invalid", "rl-proj", export)
                .await
                .expect_err("empty trajectory_bytes must be rejected up front");
        assert!(matches!(err, ExportError::InvalidConfig(_)), "got {err:?}");
    }
}
