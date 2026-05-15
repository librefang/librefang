//! Long-horizon RL rollout trajectory exporter.
//!
//! This crate is the LibreFang-side egress surface that turns a finished
//! agent rollout into an upload to an upstream RL-tracking service. It
//! is the first concrete piece of issue #3331 ("Long-horizon RL rollout
//! entry point").
//!
//! # Scope of this crate (#3331 step 1 of 3)
//!
//! - **Step 1 — Weights & Biases (this PR).** Most upstream-stable of
//!   the three target services; their public REST API has been frozen
//!   for years and is the conventional first integration for any
//!   trajectory producer.
//! - **Step 2 — Tinker** (follow-up PR). Lands as an additive
//!   `ExportTarget::Tinker { … }` variant + `src/tinker.rs` module;
//!   no breaking change to the public API of this crate.
//! - **Step 3 — Atropos** (follow-up PR). Same shape: additive
//!   `ExportTarget::Atropos { … }` + `src/atropos.rs`.
//!
//! # Wire-format decoupling (#3330)
//!
//! The exporter is intentionally **format-agnostic**. A
//! [`TrajectoryExport`] carries the trajectory as an opaque
//! `Vec<u8>` plus structured metadata; whatever bytes the rollout
//! producer hands us are uploaded verbatim. The companion RFC #3330
//! locks the on-the-wire serialization for trajectories, but **this
//! crate does not depend on that RFC** — it can land and be
//! integration-tested today, and the wire format can be decided later
//! without changing the `export()` surface.
//!
//! # HTTP client
//!
//! All outbound HTTP flows through
//! [`librefang_http::proxied_client`], the workspace's shared
//! reqwest client. This is non-negotiable per the
//! `librefang-extensions` AGENTS.md ("no bespoke `reqwest::Client`"):
//! the shared client carries the configured proxy, TLS fallback
//! roots, and `User-Agent: librefang/<version>`.

#![deny(missing_docs)]

mod atropos;
pub mod error;
mod tinker;
mod wandb;

pub use error::ExportError;

use chrono::{DateTime, Utc};

/// Target service to export a trajectory to.
///
/// This enum is `#[non_exhaustive]` so additional variants can land
/// without breaking callers.
///
/// `Debug` is hand-implemented (not derived) so that `api_key` fields
/// never appear in logs / panics / tracing spans verbatim. Adding a new
/// variant with secret material requires updating the `Debug` impl —
/// the in-crate `match` is exhaustive on purpose to force that
/// awareness.
#[non_exhaustive]
#[derive(Clone)]
pub enum ExportTarget {
    /// Export to Weights & Biases (<https://wandb.ai>). The W&B REST
    /// surface accepts run metadata + arbitrary file artefacts; we
    /// post the trajectory bytes as one file under a freshly-created
    /// (or pre-existing) run.
    WandB {
        /// W&B project name. Required by the W&B REST surface. The
        /// project must already exist; we do not auto-create.
        project: String,
        /// W&B entity (team or username). When `None`, W&B resolves
        /// the personal entity from the API key on the server side.
        entity: Option<String>,
        /// Optional client-supplied run id hint. W&B accepts the hint
        /// when creating the run; the server-assigned id is what
        /// ends up in the [`ExportReceipt`].
        run_id: Option<String>,
        /// W&B API key. Sent as the password half of HTTP Basic
        /// auth with the literal user `api`. See
        /// <https://docs.wandb.ai/ref/api/rest/>.
        api_key: String,
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
        /// Tinker API key. Sent as the `X-API-Key` header verbatim.
        /// Tinker's own SDK requires the `tml-` prefix; this crate
        /// forwards the key as-is and lets the upstream enforce the
        /// prefix (so JWT-style credentials surfaced by
        /// `TINKER_CREDENTIAL_CMD` still flow through).
        api_key: String,
        /// Project identifier sent as `project_id` on the create-session
        /// call and also surfaced as a session tag. Required.
        project: String,
        /// Optional override for the Tinker REST base URL. When
        /// `None` the crate uses Tinker's documented prod default
        /// (`https://tinker.thinkingmachines.dev/services/tinker-prod`).
        /// Operators on a self-hosted control plane set this; tests
        /// point it at a `wiremock::MockServer`.
        base_url: Option<String>,
    },
    /// Export to Atropos (<https://github.com/NousResearch/atropos>),
    /// NousResearch's RL environments microservice.
    ///
    /// Unlike W&B / Tinker, Atropos is **not a cloud-hosted service**:
    /// the API server is a local process the operator runs as part of
    /// their training stack (default `http://localhost:8000`). There
    /// is no authentication. This variant maps the rollout onto
    /// Atropos's `register-env` / `scored_data` pair; see the module
    /// docs in `atropos.rs` for the trainer-must-be-running assumption.
    ///
    /// `TrajectoryExport.trajectory_bytes` MUST already be valid
    /// `ScoredData` JSON (`tokens` / `masks` / `scores` / …); the
    /// exporter forwards the bytes verbatim and lets Atropos validate.
    Atropos {
        /// Producer name registered with Atropos as `desired_name`.
        /// Atropos appends an index (`<name>_<n>`) and returns the
        /// resolved name in the receipt. Required.
        project: String,
        /// Optional override for the Atropos `run-api` base URL. When
        /// `None` the crate uses `http://localhost:8000` (the
        /// Atropos `run-api` default). Operators running the trainer
        /// on a different host or port set this; tests point it at a
        /// `wiremock::MockServer`.
        base_url: Option<String>,
    },
}

impl std::fmt::Debug for ExportTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Each variant intentionally renders `api_key` as a fixed
        // placeholder. The `match` is exhaustive (no wildcard arm) so
        // a future secret-bearing variant fails to compile until its
        // arm is added — which is the safety property we want.
        match self {
            Self::WandB {
                project,
                entity,
                run_id,
                api_key: _,
            } => f
                .debug_struct("WandB")
                .field("project", project)
                .field("entity", entity)
                .field("run_id", run_id)
                .field("api_key", &"<redacted>")
                .finish(),
            Self::Tinker {
                api_key: _,
                project,
                base_url,
            } => f
                .debug_struct("Tinker")
                .field("api_key", &"<redacted>")
                .field("project", project)
                .field("base_url", base_url)
                .finish(),
            Self::Atropos { project, base_url } => f
                .debug_struct("Atropos")
                .field("project", project)
                .field("base_url", base_url)
                .finish(),
        }
    }
}

/// A single trajectory ready to be exported.
///
/// `trajectory_bytes` is opaque — the wire format is owned by the
/// producer (and ultimately locked by #3330). The exporter does not
/// inspect, validate, or transcode the payload; it forwards the bytes
/// to the upstream verbatim. This keeps the exporter stable across
/// wire-format iterations.
#[derive(Debug, Clone)]
pub struct TrajectoryExport {
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
    /// `TrajectoryExport::trajectory_bytes.len()` on success.
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
    export: TrajectoryExport,
) -> Result<ExportReceipt, ExportError> {
    match target {
        ExportTarget::WandB {
            project,
            entity,
            run_id,
            api_key,
        } => {
            wandb::export_to_wandb(
                &project,
                entity.as_deref(),
                run_id.as_deref(),
                &api_key,
                export,
            )
            .await
        }
        ExportTarget::Tinker {
            api_key,
            project,
            base_url,
        } => tinker::export_to_tinker(&project, &api_key, base_url.as_deref(), export).await,
        ExportTarget::Atropos { project, base_url } => {
            atropos::export_to_atropos(&project, base_url.as_deref(), export).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hand-written `Debug` impl must redact api_key fields across
    /// every secret-bearing variant. Asserting against the literal key
    /// guards the property: even a misplaced field-list reordering
    /// during a future refactor will fail this test.
    #[test]
    fn debug_redacts_api_key_for_secret_bearing_variants() {
        let secret = "sk-live-DO-NOT-LEAK-12345";

        let wandb = ExportTarget::WandB {
            project: "rl-proj".to_string(),
            entity: Some("acme".to_string()),
            run_id: Some("run-1".to_string()),
            api_key: secret.to_string(),
        };
        let rendered = format!("{wandb:?}");
        assert!(
            !rendered.contains(secret),
            "Debug must not include api_key plaintext: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "Debug must render placeholder: {rendered}"
        );

        let tinker = ExportTarget::Tinker {
            api_key: secret.to_string(),
            project: "rl-proj".to_string(),
            base_url: None,
        };
        let rendered = format!("{tinker:?}");
        assert!(
            !rendered.contains(secret),
            "Debug must not include api_key plaintext: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "Debug must render placeholder: {rendered}"
        );
    }
}
