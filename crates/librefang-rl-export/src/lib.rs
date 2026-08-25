//! Long-horizon RL rollout trajectory exporter.
//!
//! This crate is the LibreFang-side egress surface that turns a finished
//! agent rollout into an upload to an upstream RL-tracking service. It
//! is the concrete delivery of issue #3331 ("Long-horizon RL rollout
//! entry point"); all three upstream targets land together.
//!
//! # Scope of this crate (#3331)
//!
//! Three upstream targets are supported today, each behind an additive
//! variant on the `#[non_exhaustive]` [`ExportTarget`] enum:
//!
//! - **Weights & Biases** — REST API has been frozen for years; the
//!   conventional first integration for any trajectory producer.
//! - **Tinker** — Thinking Machines' distributed LLM post-training API;
//!   no dedicated opaque-trajectory upload endpoint today, so the
//!   exporter maps onto Tinker's `(create_session, telemetry)` pair.
//! - **Atropos** — NousResearch's local FastAPI RL-environments
//!   microservice; the exporter maps onto Atropos's
//!   `(register-env, scored_data)` pair.
//!
//! # Wire-format decoupling (#3330)
//!
//! The exporter is intentionally **format-agnostic**. A
//! [`RlTrajectoryExport`] carries the trajectory as an opaque
//! `Vec<u8>` plus structured metadata; whatever bytes the rollout
//! producer hands us are uploaded verbatim. The companion RFC #3330
//! locks the on-the-wire serialization for trajectories, but **this
//! crate does not depend on that RFC** — it can land and be
//! integration-tested today, and the wire format can be decided later
//! without changing the `export()` surface.
//!
//! # HTTP client
//!
//! All outbound HTTP clients start from
//! [`librefang_http::proxied_client_builder`], the workspace's shared
//! reqwest client configuration. This is non-negotiable per the
//! `librefang-extensions` AGENTS.md ("no bespoke `reqwest::Client`"):
//! the shared builder carries TLS fallback roots, timeouts, and
//! `User-Agent: librefang/<version>`. Exporters additionally disable
//! redirects and proxies, then pin domain names to the complete set of
//! addresses already checked by the SSRF policy. A proxy cannot preserve
//! that check-to-connect DNS invariant, so proxy-only deployments fail closed.

#![deny(missing_docs)]

mod atropos;
pub mod error;
mod redact;
mod retry;
mod ssrf;
mod tinker;
mod wandb;

pub use error::ExportError;

use std::time::Duration;

use chrono::{DateTime, Utc};

/// Total wall-clock budget for one upstream request attempt.
///
/// The shared HTTP builder already bounds connection setup and individual
/// response reads. A total timeout is still required here because an upstream
/// can otherwise keep a request alive indefinitely by sending data slowly.
const EXPORT_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

fn build_export_http_client_with(
    builder: reqwest::ClientBuilder,
    resolution: &ssrf::ResolvedEgress,
) -> Result<reqwest::Client, ExportError> {
    build_export_http_client_with_timeout(builder, resolution, EXPORT_REQUEST_TIMEOUT)
}

fn build_export_http_client_with_timeout(
    builder: reqwest::ClientBuilder,
    resolution: &ssrf::ResolvedEgress,
    request_timeout: Duration,
) -> Result<reqwest::Client, ExportError> {
    let mut builder = builder
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(request_timeout);
    if let Some(hostname) = &resolution.hostname {
        builder = builder.resolve_to_addrs(hostname, &resolution.addresses);
    }
    builder.build().map_err(ExportError::from)
}

fn build_export_http_client(
    resolution: &ssrf::ResolvedEgress,
) -> Result<reqwest::Client, ExportError> {
    build_export_http_client_with(librefang_http::proxied_client_builder(), resolution)
}

/// Target service to export a trajectory to.
///
/// This enum is `#[non_exhaustive]` so additional variants can land
/// without breaking callers.
///
/// # Secret handling: `*_env` indirection
///
/// API keys MUST NOT be inlined into this enum. Each secret-bearing
/// variant carries an `api_key_env: String` field holding the **name**
/// of the environment variable that holds the secret; the exporter
/// reads the env var with `std::env::var` at upload time and fails
/// closed with [`ExportError::InvalidConfig`] if the variable is unset
/// or empty. This matches the rest of the workspace's
/// `client_secret_env` / `api_key_env` convention (see
/// `librefang-types::config::types::BraveSearchConfig` etc.) and keeps
/// secrets out of `config.toml`, history snapshots, and process dumps.
///
/// `Debug` is derived (the public fields are env-var names — not the
/// secret values — so plain `Debug` is safe). Adding a new variant
/// that holds the secret material itself (rather than the env-var
/// name) is a *regression*; route through the `_env` indirection
/// instead.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ExportTarget {
    /// Export to Weights & Biases (<https://wandb.ai>). The W&B REST
    /// surface accepts run metadata + arbitrary file artefacts; we
    /// post the trajectory bytes as one file under a freshly-created
    /// (or pre-existing) run.
    WandB {
        /// W&B project name. Required by the W&B REST surface. The
        /// project must already exist; we do not auto-create.
        project: String,
        /// W&B entity (team or username). Required — W&B's "personal
        /// entity" resolution via the API key is undocumented and
        /// upload requests need an explicit entity in the URL path
        /// (the previous `"default"` fallback was a guess that would
        /// silently land the run under a wrong-named bucket). Callers
        /// who want the personal entity must look it up out of band
        /// and pass it here.
        entity: String,
        /// Optional client-supplied run id hint. W&B accepts the hint
        /// when creating the run; the server-assigned id is what
        /// ends up in the [`ExportReceipt`]. When `None`,
        /// [`RlTrajectoryExport::run_id`] is forwarded as the hint.
        run_id: Option<String>,
        /// Name of the environment variable holding the W&B API key.
        /// Resolved with `std::env::var` at upload time and sent as
        /// the password half of HTTP Basic auth with the literal user
        /// `api`. See <https://docs.wandb.ai/ref/api/rest/>. Missing /
        /// empty env var fails closed with `InvalidConfig`.
        api_key_env: String,
    },
    /// Export to Tinker (<https://thinkingmachines.ai/tinker/>).
    ///
    /// Tinker's REST surface is training-call-centric and doesn't
    /// expose a dedicated opaque-trajectory upload endpoint today.
    /// This variant maps the rollout onto the closest stable
    /// `(create_session, telemetry)` pair Tinker actually accepts;
    /// see the module-level docs in `tinker.rs` for the assumption
    /// flagged for maintainer sign-off and the SDK source links.
    Tinker {
        /// Name of the environment variable holding the Tinker API
        /// key. Resolved with `std::env::var` at upload time and sent
        /// as the `X-API-Key` header verbatim. Tinker's own SDK
        /// requires the `tml-` prefix; this crate forwards the key
        /// as-is and lets the upstream enforce the prefix (so
        /// JWT-style credentials surfaced by `TINKER_CREDENTIAL_CMD`
        /// still flow through). Missing / empty env var fails closed
        /// with `InvalidConfig`.
        api_key_env: String,
        /// Project identifier sent as `project_id` on the create-session
        /// call and also surfaced as a session tag. Required.
        project: String,
        /// Optional override for the Tinker REST base URL. When
        /// `None` the crate uses Tinker's documented prod default
        /// (`https://tinker.thinkingmachines.dev/services/tinker-prod`).
        /// Operators on a self-hosted control plane set this; tests
        /// point it at a `wiremock::MockServer`. SSRF-validated:
        /// loopback / private / link-local destinations are rejected.
        base_url: Option<String>,
    },
    /// Export to Atropos (<https://github.com/NousResearch/atropos>),
    /// NousResearch's RL environments microservice.
    ///
    /// Unlike W&B / Tinker, Atropos is **not a cloud-hosted service**:
    /// the API server is a local process the operator runs as part of
    /// their training stack. There is no authentication. This variant
    /// maps the rollout onto Atropos's `register-env` / `scored_data`
    /// pair; see the module docs in `atropos.rs` for the
    /// trainer-must-be-running assumption.
    ///
    /// **Atropos is the one exporter that talks to loopback / private
    /// addresses by design** (the trainer is a local FastAPI service).
    /// `base_url` MUST be a loopback (`127.0.0.0/8`, `::1`) or
    /// RFC-1918 private destination; the SSRF guard rejects anything
    /// public, and an unset `base_url` is rejected outright (no
    /// implicit `localhost:8000` default) so operators make the
    /// decision explicitly.
    ///
    /// `RlTrajectoryExport.trajectory_bytes` MUST already be valid
    /// `ScoredData` JSON (`tokens` / `masks` / `scores` / …); the
    /// exporter forwards the bytes verbatim and lets Atropos validate.
    Atropos {
        /// Producer name registered with Atropos as `desired_name`.
        /// Atropos appends an index (`<name>_<n>`) and returns the
        /// resolved name in the receipt. Required.
        project: String,
        /// Atropos `run-api` base URL. Required and SSRF-validated
        /// against the loopback / RFC-1918 allowlist for the Atropos
        /// variant. There is intentionally **no implicit default** —
        /// the prior `http://localhost:8000` was a guess that violated
        /// the workspace SSRF policy. Operators set it explicitly to
        /// the local trainer; tests point at a `wiremock::MockServer`.
        base_url: String,
        /// Maximum token length to report on `RegisterEnv`. `None`
        /// uses the conservative default `32_768`.
        max_token_length: Option<u32>,
        /// Group size to report on `RegisterEnv`. `None` uses `1`.
        group_size: Option<u32>,
        /// Weight to report on `RegisterEnv`. `None` uses `1.0`.
        weight: Option<f32>,
    },
}

/// A single RL rollout trajectory ready to be exported.
///
/// `trajectory_bytes` is opaque — the wire format is owned by the
/// producer (and ultimately locked by #3330). The exporter does not
/// inspect, validate, or transcode the payload; it forwards the bytes
/// to the upstream verbatim. This keeps the exporter stable across
/// wire-format iterations.
///
/// Named `RlTrajectoryExport` (not `TrajectoryExport`) so the type is
/// unambiguously distinct from the kernel's
/// `librefang_kernel::trajectory::TrajectoryExporter` — that one emits a
/// redacted **session** audit trail for support / compliance, whereas
/// this struct describes an **RL rollout** egress destined for an
/// external training service. The two concepts share zero state and
/// must not be confused.
#[derive(Debug, Clone)]
pub struct RlTrajectoryExport {
    /// Caller-side run identifier. Used as a default hint when the
    /// target accepts one (e.g. W&B's `run_id` field); upstreams may
    /// reassign and return their own server-side id, which ends up in
    /// the receipt.
    pub run_id: String,
    /// Opaque trajectory bytes. See module-level docs on wire-format
    /// decoupling — this crate does not parse, validate, or
    /// transcode them.
    pub trajectory_bytes: Vec<u8>,
    /// Optional structured metadata describing the toolset / agent /
    /// environment that produced the trajectory. Forwarded to the
    /// upstream as the run's metadata blob when the target supports
    /// one (W&B does). `None` is fine.
    pub toolset_metadata: Option<serde_json::Value>,
    /// Wall-clock start of the rollout window. Forwarded to the
    /// upstream so the run's reported duration matches reality.
    pub started_at: DateTime<Utc>,
    /// Wall-clock end of the rollout window.
    pub finished_at: DateTime<Utc>,
}

/// Receipt returned by a successful [`export`] call.
///
/// All fields point at the **upstream's** view of the upload — in
/// particular `target_run_url` is whatever URL the upstream returned
/// (e.g. `https://wandb.ai/<entity>/<project>/runs/<id>`), so the
/// operator can click straight through to the experiment page.
#[derive(Debug, Clone)]
pub struct ExportReceipt {
    /// Public, browser-loadable URL of the run on the upstream.
    pub target_run_url: String,
    /// Number of trajectory bytes uploaded. Mirrors
    /// `RlTrajectoryExport::trajectory_bytes.len()` on success.
    pub bytes_uploaded: u64,
    /// Wall-clock time the upload completed, as observed locally.
    pub uploaded_at: DateTime<Utc>,
}

/// Export a trajectory to the chosen [`ExportTarget`].
///
/// This is the only public entry point; per-target implementations
/// live in private modules (`wandb`, plus future `tinker` / `atropos`)
/// and are dispatched on the variant. The function is fully `async`
/// and performs all I/O via the workspace shared HTTP client; the
/// caller is expected to run it on a Tokio runtime.
///
/// # Errors
///
/// - [`ExportError::InvalidConfig`] — caller-supplied configuration
///   (empty API key, empty project, …) was rejected before any
///   network I/O happened.
/// - [`ExportError::AuthError`] — upstream rejected the credentials
///   (HTTP 401 / 403).
/// - [`ExportError::UpstreamRejected`] — upstream returned a non-auth
///   4xx / 5xx. Status code and (truncated) body are forwarded.
/// - [`ExportError::NetworkError`] — transport-layer failure.
/// - [`ExportError::MalformedResponse`] — upstream returned a 2xx but
///   the body did not match the expected shape.
pub async fn export(
    target: ExportTarget,
    mut payload: RlTrajectoryExport,
) -> Result<ExportReceipt, ExportError> {
    normalize_export_metadata(&mut payload);
    export_with_secret_resolver(target, payload, |name| std::env::var(name)).await
}

async fn export_with_secret_resolver<F>(
    target: ExportTarget,
    payload: RlTrajectoryExport,
    lookup_secret: F,
) -> Result<ExportReceipt, ExportError>
where
    F: Fn(&str) -> Result<String, std::env::VarError>,
{
    match target {
        ExportTarget::WandB {
            project,
            entity,
            run_id,
            api_key_env,
        } => {
            let api_key = resolve_env_secret_with(&api_key_env, "W&B api_key_env", &lookup_secret)?;
            wandb::validate_config(&project, &entity, &api_key)?;
            // SSRF gate: the public dispatch always validates the
            // outbound base URL before any outbound HTTP. W&B is a cloud service —
            // loopback / private / link-local destinations are rejected.
            let resolution = ssrf::resolve_and_validate_egress_url(
                wandb::DEFAULT_WANDB_BASE,
                ssrf::EgressMode::Public,
            )
            .await?;
            let client = build_export_http_client(&resolution)?;
            wandb::export_to_wandb(
                &project,
                &entity,
                run_id.as_deref(),
                &api_key,
                client,
                payload,
            )
            .await
        }
        ExportTarget::Tinker {
            api_key_env,
            project,
            base_url,
        } => {
            let api_key =
                resolve_env_secret_with(&api_key_env, "Tinker api_key_env", &lookup_secret)?;
            tinker::validate_config(&project, &api_key)?;
            // SSRF gate. Tinker is a public cloud service even when an
            // operator overrides `base_url` for a self-hosted control
            // plane (still a public DNS name); loopback / private /
            // link-local destinations are rejected.
            let base = base_url.as_deref().unwrap_or(tinker::DEFAULT_TINKER_BASE);
            let resolution =
                ssrf::resolve_and_validate_egress_url(base, ssrf::EgressMode::Public).await?;
            let client = build_export_http_client(&resolution)?;
            tinker::export_to_tinker(&project, &api_key, base_url.as_deref(), client, payload).await
        }
        ExportTarget::Atropos {
            project,
            base_url,
            max_token_length,
            group_size,
            weight,
        } => {
            atropos::validate_config(&project, &base_url, &payload)?;
            // SSRF gate. Atropos is a local-only microservice — only
            // loopback / RFC-1918 destinations are accepted; public
            // destinations and link-local / IMDS are rejected.
            let resolution = ssrf::resolve_and_validate_egress_url(
                &base_url,
                ssrf::EgressMode::LoopbackOrPrivate,
            )
            .await?;
            let client = build_export_http_client(&resolution)?;
            atropos::export_to_atropos(
                &project,
                &base_url,
                atropos::AtroposTuning {
                    max_token_length,
                    group_size,
                    weight,
                },
                client,
                payload,
            )
            .await
        }
    }
}

pub(crate) fn normalize_export_metadata(export: &mut RlTrajectoryExport) {
    if let Some(metadata) = export.toolset_metadata.take() {
        export.toolset_metadata = Some(redact::redact_metadata(metadata));
    }
}

/// Resolve an `*_env` indirection: read the named environment variable
/// and return its value. Empty / unset env vars surface as
/// [`ExportError::InvalidConfig`] so the operator sees the failure at
/// the call site rather than as a downstream 401.
///
/// `field_label` identifies the caller-side config field name (e.g.
/// `"W&B api_key_env"`) and is woven into the error so the operator
/// can locate the offending entry in `config.toml`. The actual env-var
/// name is also echoed — it's not a secret, only its value is.
fn resolve_env_secret_with<F>(
    env_var: &str,
    field_label: &str,
    lookup: F,
) -> Result<String, ExportError>
where
    F: FnOnce(&str) -> Result<String, std::env::VarError>,
{
    if env_var.is_empty() {
        return Err(ExportError::InvalidConfig(format!(
            "{field_label} is empty (expected the NAME of an environment variable holding the secret, not the secret itself)"
        )));
    }
    match lookup(env_var) {
        Ok(v) if !v.is_empty() => Ok(v),
        Ok(_) => Err(ExportError::InvalidConfig(format!(
            "{field_label} points at env var '{env_var}', which is set but empty"
        ))),
        Err(_) => Err(ExportError::InvalidConfig(format!(
            "{field_label} points at env var '{env_var}', which is not set"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};

    #[test]
    fn export_client_build_failure_is_not_replaced_by_a_default_client() {
        let invalid_builder = reqwest::Client::builder().user_agent("invalid\nuser-agent");
        let resolution = ssrf::ResolvedEgress {
            hostname: None,
            addresses: Vec::new(),
        };
        let error = build_export_http_client_with(invalid_builder, &resolution)
            .expect_err("an invalid secure client must fail closed");

        assert!(matches!(error, ExportError::NetworkError(_)));
    }

    #[tokio::test]
    async fn export_client_uses_only_pinned_dns_and_bypasses_proxies() {
        let target = MockServer::start().await;
        Mock::given(path("/pinned"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&target)
            .await;

        let proxy = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(502))
            .mount(&proxy)
            .await;

        let resolution = ssrf::ResolvedEgress {
            hostname: Some("rebind.invalid".to_string()),
            // Keep a reachable address first and a sentinel second. Repeated
            // `resolve` calls overwrite each other in reqwest, so an
            // implementation that failed to retain the full set would hit
            // the sentinel and return 502 instead.
            addresses: vec![*target.address(), *proxy.address()],
        };
        let builder = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy.uri()).expect("valid proxy URL"));
        let client = build_export_http_client_with(builder, &resolution)
            .expect("validated pinned client should build");

        let response = client
            .get("http://rebind.invalid/pinned")
            .send()
            .await
            .expect("the pinned target should receive the request");

        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
        assert_eq!(target.received_requests().await.unwrap().len(), 1);
        assert!(proxy.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn export_client_bounds_total_request_time() {
        let target = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(1)))
            .mount(&target)
            .await;

        let resolution = ssrf::ResolvedEgress {
            hostname: None,
            addresses: Vec::new(),
        };
        let client = build_export_http_client_with_timeout(
            reqwest::Client::builder(),
            &resolution,
            Duration::from_millis(25),
        )
        .expect("export client should build");

        let error = client
            .get(format!("{}/slow", target.uri()))
            .send()
            .await
            .expect_err("slow response must exceed the total request budget");

        assert!(error.is_timeout(), "expected timeout, got {error}");
    }

    /// `*_env` indirection: an empty env-var name fails fast with
    /// `InvalidConfig` (no env probe) so a caller that accidentally
    /// stored the literal secret in the `*_env` field — or left it
    /// unset entirely — sees the misconfiguration immediately, not as
    /// a downstream 401.
    #[test]
    fn resolve_env_secret_rejects_empty_var_name() {
        let err = resolve_env_secret_with("", "test", |_| {
            panic!("an empty variable name must fail before lookup")
        })
        .expect_err("empty env var name must be rejected");
        assert!(matches!(err, ExportError::InvalidConfig(_)), "got {err:?}");
    }

    /// `*_env` indirection: an unset env var surfaces as
    /// `InvalidConfig` mentioning the variable name (the *name* is
    /// not a secret — the operator needs to see which var was missing).
    #[test]
    fn resolve_env_secret_rejects_unset_var() {
        let var = "LIBREFANG_RL_EXPORT_TEST_DEFINITELY_UNSET_42";
        let err = resolve_env_secret_with(var, "test", |_| Err(std::env::VarError::NotPresent))
            .expect_err("unset env var must be rejected");
        match err {
            ExportError::InvalidConfig(msg) => {
                assert!(msg.contains(var), "error must mention env var name: {msg}");
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_export_safely_consumes_extremely_deep_metadata() {
        let mut metadata = serde_json::Value::Null;
        for _ in 0..50_000 {
            metadata = serde_json::Value::Array(vec![metadata]);
        }
        let payload = RlTrajectoryExport {
            run_id: "rid".to_string(),
            trajectory_bytes: b"bytes".to_vec(),
            toolset_metadata: Some(metadata),
            started_at: chrono::Utc::now(),
            finished_at: chrono::Utc::now(),
        };
        let target = ExportTarget::Atropos {
            project: String::new(),
            base_url: "http://127.0.0.1:8000".to_string(),
            max_token_length: None,
            group_size: None,
            weight: None,
        };

        let error = export(target, payload).await.unwrap_err();
        assert!(matches!(error, ExportError::InvalidConfig(_)));
    }

    /// SSRF gate: routing a Tinker export at the cloud metadata IP
    /// (169.254.169.254) MUST be rejected with `InvalidConfig` rather
    /// than completing a successful upload. Pins the egress allowlist
    /// against an operator who passes a tenant-controlled string into
    /// `ExportTarget::Tinker.base_url`.
    #[tokio::test]
    async fn export_rejects_tinker_base_url_at_imds() {
        let payload = RlTrajectoryExport {
            run_id: "rid".to_string(),
            trajectory_bytes: b"bytes".to_vec(),
            toolset_metadata: None,
            started_at: chrono::Utc::now(),
            finished_at: chrono::Utc::now(),
        };
        let target = ExportTarget::Tinker {
            api_key_env: "LIBREFANG_RL_EXPORT_TEST_IMDS_KEY".to_string(),
            project: "p".to_string(),
            base_url: Some("http://169.254.169.254/latest/meta-data/".to_string()),
        };
        let err = export_with_secret_resolver(target, payload, |_| Ok("tml-fake".to_string()))
            .await
            .expect_err("IMDS base URL must be rejected");
        match err {
            ExportError::InvalidConfig(msg) => {
                assert!(
                    msg.contains("169.254") || msg.to_lowercase().contains("block"),
                    "error must mention the blocked host: {msg}"
                );
            }
            other => panic!("expected InvalidConfig (SSRF), got {other:?}"),
        }
    }

    /// SSRF gate: routing an Atropos export at a public IP MUST be
    /// rejected. Atropos has no auth — exposing the producer to the
    /// public internet is the wrong shape entirely.
    #[tokio::test]
    async fn export_rejects_atropos_public_base_url() {
        let payload = RlTrajectoryExport {
            run_id: "rid".to_string(),
            trajectory_bytes: b"bytes".to_vec(),
            toolset_metadata: None,
            started_at: chrono::Utc::now(),
            finished_at: chrono::Utc::now(),
        };
        let target = ExportTarget::Atropos {
            project: "p".to_string(),
            base_url: "https://attacker.example.com/".to_string(),
            max_token_length: None,
            group_size: None,
            weight: None,
        };
        let err = export(target, payload)
            .await
            .expect_err("public base URL on Atropos must be rejected");
        assert!(matches!(err, ExportError::InvalidConfig(_)), "got {err:?}");
    }
}
