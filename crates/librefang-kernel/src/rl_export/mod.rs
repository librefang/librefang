//! RL rollout trajectory export producer (issues #3330 / #3331).
//!
//! The `librefang-rl-export` crate is fully tested but has no producer
//! feeding it. This module is that producer: it hooks the agent turn-end
//! path (`HookEvent::AgentLoopEnd`), serializes the finished session's
//! trajectory via the kernel's existing `trajectory` exporter, and forwards
//! the bytes to `librefang_rl_export::export` on a detached task.
//!
//! # Gates (cheapest first, per agent)
//!
//! 1. **Kernel shutdown** — skip if the daemon is unwinding.
//! 2. **Global enabled** — `config.rl_export.enabled` must be true.
//! 3. **Per-agent override** — `agent.toml: [rl_export] enabled` supersedes
//!    the global toggle when set. The resolution is override > global.
//! 4. **Target configured** — `config.rl_export.target` must be `Some`;
//!    otherwise there is nowhere to send the rollout.
//!
//! # Trajectory bytes encoding
//!
//! `RlTrajectoryExport.trajectory_bytes` is the UTF-8 bytes of the kernel
//! trajectory bundle's **JSONL** form (`TrajectoryBundle::to_jsonl()`):
//! one JSON metadata header line followed by one redacted message per line.
//! This reuses the existing, redaction-applying session serializer rather
//! than inventing a new wire format — the exporter is format-agnostic and
//! forwards the bytes verbatim (see the `librefang-rl-export` crate docs on
//! wire-format decoupling, #3330). Redaction (credential masking, workspace
//! path collapsing) is applied by the bundle builder, so secrets do not
//! leave the daemon in the trajectory payload.
//!
//! # Fire-and-forget
//!
//! The export runs on a `spawn_supervised` detached task so the agent loop's
//! return path never blocks on network I/O. A failed export logs a warning
//! and is dropped — the rollout's primary record is the session itself,
//! which is already persisted; the export is a best-effort egress.

use std::sync::Arc;

use librefang_rl_export::{ExportTarget, RlTrajectoryExport};
use librefang_types::agent::AgentId;
use librefang_types::config::RlExportTarget;

use crate::kernel::LibreFangKernel;
use crate::AgentSubsystemApi;

/// Resolve whether RL export is enabled for `agent_id`: the per-agent
/// manifest override wins over the kernel-global toggle when set.
///
/// Mirrors auto_dream's `effective_thresholds` resolution shape (manifest
/// `Option` override > global config value).
fn export_enabled_for_agent(kernel: &LibreFangKernel, agent_id: AgentId) -> bool {
    let global = kernel.config_snapshot().rl_export.enabled;
    // `get_arc` (Arc bump) not `get` (deep AgentEntry+manifest clone): this
    // runs on the AgentLoopEnd hot path for every turn of every agent, even
    // when export is globally disabled. Same rationale as `is_auto_dream_enabled`.
    kernel
        .agent_registry_ref()
        .get_arc(agent_id)
        .and_then(|e| e.manifest.rl_export.enabled)
        .unwrap_or(global)
}

/// Map the config-side [`RlExportTarget`] onto the exporter's
/// [`ExportTarget`]. The two enums are kept separate because the exporter's
/// is `#[non_exhaustive]` and does not derive serde; this is the single
/// conversion boundary.
fn to_export_target(target: RlExportTarget) -> ExportTarget {
    match target {
        RlExportTarget::Wandb {
            project,
            entity,
            run_id,
            api_key_env,
        } => ExportTarget::WandB {
            project,
            entity,
            run_id,
            api_key_env,
        },
        RlExportTarget::Tinker {
            api_key_env,
            project,
            base_url,
        } => ExportTarget::Tinker {
            api_key_env,
            project,
            base_url,
        },
        RlExportTarget::Atropos {
            project,
            base_url,
            max_token_length,
            group_size,
            weight,
        } => ExportTarget::Atropos {
            project,
            base_url,
            max_token_length,
            group_size,
            weight,
        },
    }
}

/// Build the [`RlTrajectoryExport`] payload for an agent's current session.
///
/// Returns `None` when the agent is unknown or the trajectory bundle cannot
/// be built (e.g. the session row is missing for a non-current id). The
/// `run_id` is `"<agent_id>:<session_id>"` so an upstream run is uniquely
/// addressable back to the producing agent and session. `started_at` is the
/// first recorded message timestamp (falling back to now when none is
/// present); `finished_at` is the export instant.
fn build_payload(kernel: &LibreFangKernel, agent_id: AgentId) -> Option<RlTrajectoryExport> {
    let session_id = kernel.agent_registry_ref().get(agent_id)?.session_id;

    let bundle = match kernel.export_session_trajectory(agent_id, session_id) {
        Ok(bundle) => bundle,
        Err(e) => {
            tracing::debug!(agent = %agent_id, error = %e, "rl_export: could not build trajectory bundle, skipping");
            return None;
        }
    };

    let finished_at = chrono::Utc::now();
    // Use the earliest message timestamp as the rollout start; fall back to
    // `finished_at` when no message carries a timestamp (empty / fresh
    // session). The bundle stores RFC-3339 strings; parse leniently.
    let started_at = bundle
        .messages
        .iter()
        .filter_map(|m| m.timestamp.as_deref())
        .filter_map(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .min()
        .unwrap_or(finished_at);

    let trajectory_bytes = bundle.to_jsonl().into_bytes();
    let toolset_metadata = Some(serde_json::json!({
        "agent_id": agent_id.to_string(),
        "session_id": session_id.0.to_string(),
        "agent_name": bundle.metadata.agent_name,
        "model": bundle.metadata.model,
        "provider": bundle.metadata.provider,
        "message_count": bundle.metadata.message_count,
    }));

    Some(RlTrajectoryExport {
        run_id: format!("{}:{}", agent_id, session_id.0),
        trajectory_bytes,
        toolset_metadata,
        started_at,
        finished_at,
    })
}

/// Event-driven trigger: called from the `AgentLoopEnd` hook whenever an
/// agent finishes a turn. Cheap early-exits keep the hot path near-free; the
/// payload assembly and network upload run on a detached task so the agent
/// loop never blocks on serialization or I/O.
pub fn maybe_export_on_turn_end(kernel: Arc<LibreFangKernel>, agent_id: AgentId) {
    // Gate 1: kernel shutdown.
    if kernel.agents.supervisor.is_shutting_down() {
        return;
    }
    // Gate 2 + 3: global toggle resolved against the per-agent override.
    if !export_enabled_for_agent(&kernel, agent_id) {
        return;
    }
    // Gate 4: a destination must be configured.
    let Some(target) = kernel.config_snapshot().rl_export.target.clone() else {
        tracing::warn!(
            agent = %agent_id,
            "rl_export: enabled but no [rl_export] target configured — skipping export"
        );
        return;
    };

    crate::supervised_spawn::spawn_supervised("rl_export_dispatch", async move {
        // Re-check the gates inside the task: the operator could have
        // flipped the toggle or started a shutdown between the synchronous
        // pre-filter and the task being scheduled.
        if kernel.agents.supervisor.is_shutting_down() {
            return;
        }
        if !export_enabled_for_agent(&kernel, agent_id) {
            return;
        }

        let Some(payload) = build_payload(&kernel, agent_id) else {
            return;
        };
        let run_id = payload.run_id.clone();
        let byte_len = payload.trajectory_bytes.len();

        match librefang_rl_export::export(to_export_target(target), payload).await {
            Ok(receipt) => {
                tracing::info!(
                    agent = %agent_id,
                    run_id = %run_id,
                    bytes = byte_len,
                    target_url = %receipt.target_run_url,
                    "rl_export: trajectory exported"
                );
            }
            Err(e) => {
                tracing::warn!(
                    agent = %agent_id,
                    run_id = %run_id,
                    error = %e,
                    "rl_export: trajectory export failed"
                );
            }
        }
    });
}

/// `HookHandler` wiring the runtime's `AgentLoopEnd` event to the RL-export
/// producer. Registered once during `LibreFangKernel::set_self_handle` so it
/// can hold a `Weak<LibreFangKernel>` and upgrade on fire — same shape as
/// `AutoDreamTurnEndHook` and `SkillWorkshopTurnEndHook`.
pub struct RlExportTurnEndHook {
    kernel: std::sync::Weak<LibreFangKernel>,
}

impl RlExportTurnEndHook {
    pub fn new(kernel: std::sync::Weak<LibreFangKernel>) -> Self {
        Self { kernel }
    }
}

impl librefang_runtime::hooks::HookHandler for RlExportTurnEndHook {
    fn on_event(&self, ctx: &librefang_runtime::hooks::HookContext) -> Result<(), String> {
        use librefang_types::agent::HookEvent;
        // Observe-only: ignore everything but AgentLoopEnd. The registry
        // already filters by event type, so this is defensive.
        if ctx.event != HookEvent::AgentLoopEnd {
            return Ok(());
        }
        // Skip fork turns. Forked turns (auto_dream, proactive memory,
        // skill review) reuse the parent session in-memory and aren't a
        // distinct rollout to export; exporting them would double-count and
        // leak background-task content into the rollout stream.
        if ctx
            .data
            .get("is_fork")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(());
        }
        // Kernel dropped (process shutting down) — nothing to do.
        let Some(kernel) = self.kernel.upgrade() else {
            return Ok(());
        };
        let Ok(uuid) = uuid::Uuid::parse_str(ctx.agent_id) else {
            tracing::debug!(
                agent_id = %ctx.agent_id,
                "rl_export: AgentLoopEnd hook saw non-UUID agent_id, skipping",
            );
            return Ok(());
        };
        maybe_export_on_turn_end(kernel, AgentId(uuid));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_runtime::hooks::{HookContext, HookHandler};
    use librefang_types::agent::HookEvent;
    use librefang_types::config::{KernelConfig, RlExportConfig, RlExportTarget};

    /// Config-side → exporter-side target conversion is field-for-field
    /// faithful for every variant. Pins the single conversion boundary so a
    /// renamed/added field surfaces here rather than as a silent mis-export.
    #[test]
    fn to_export_target_maps_every_variant() {
        match to_export_target(RlExportTarget::Wandb {
            project: "p".into(),
            entity: "e".into(),
            run_id: Some("r".into()),
            api_key_env: "K".into(),
        }) {
            ExportTarget::WandB {
                project,
                entity,
                run_id,
                api_key_env,
            } => {
                assert_eq!(project, "p");
                assert_eq!(entity, "e");
                assert_eq!(run_id.as_deref(), Some("r"));
                assert_eq!(api_key_env, "K");
            }
            other => panic!("expected WandB, got {other:?}"),
        }

        match to_export_target(RlExportTarget::Tinker {
            api_key_env: "K".into(),
            project: "p".into(),
            base_url: Some("https://example.test".into()),
        }) {
            ExportTarget::Tinker {
                api_key_env,
                project,
                base_url,
            } => {
                assert_eq!(api_key_env, "K");
                assert_eq!(project, "p");
                assert_eq!(base_url.as_deref(), Some("https://example.test"));
            }
            other => panic!("expected Tinker, got {other:?}"),
        }

        match to_export_target(RlExportTarget::Atropos {
            project: "p".into(),
            base_url: "http://127.0.0.1:8000".into(),
            max_token_length: Some(1024),
            group_size: Some(2),
            weight: Some(0.5),
        }) {
            ExportTarget::Atropos {
                project,
                base_url,
                max_token_length,
                group_size,
                weight,
            } => {
                assert_eq!(project, "p");
                assert_eq!(base_url, "http://127.0.0.1:8000");
                assert_eq!(max_token_length, Some(1024));
                assert_eq!(group_size, Some(2));
                assert_eq!(weight, Some(0.5));
            }
            other => panic!("expected Atropos, got {other:?}"),
        }
    }

    /// `[rl_export]` defaults to disabled — the opt-in contract. A fresh
    /// KernelConfig must not export anything.
    #[test]
    fn rl_export_config_defaults_disabled() {
        let cfg = KernelConfig::default();
        assert!(!cfg.rl_export.enabled);
        assert!(cfg.rl_export.target.is_none());
    }

    /// The config round-trips through TOML with a W&B target, proving the
    /// tagged-enum shape an operator writes in `config.toml` deserializes.
    #[test]
    fn rl_export_config_toml_round_trip() {
        let toml = r#"
enabled = true
target = { type = "wandb", project = "rollouts", entity = "team", api_key_env = "WANDB_API_KEY" }
"#;
        let cfg: RlExportConfig = toml::from_str(toml).expect("config parses");
        assert!(cfg.enabled);
        match cfg.target.expect("target set") {
            RlExportTarget::Wandb {
                project,
                entity,
                api_key_env,
                run_id,
            } => {
                assert_eq!(project, "rollouts");
                assert_eq!(entity, "team");
                assert_eq!(api_key_env, "WANDB_API_KEY");
                assert!(run_id.is_none());
            }
            other => panic!("expected Wandb target, got {other:?}"),
        }
    }

    /// Hook tolerates a dropped kernel (`Weak::new`) without panicking — a
    /// panicking hook would crash the agent loop thread.
    #[test]
    fn hook_with_dropped_kernel_is_silent_noop() {
        let hook = RlExportTurnEndHook::new(std::sync::Weak::new());
        let ctx = HookContext {
            agent_name: "probe",
            agent_id: &uuid::Uuid::new_v4().to_string(),
            event: HookEvent::AgentLoopEnd,
            data: serde_json::json!({"is_fork": false}),
        };
        assert!(hook.on_event(&ctx).is_ok());
    }

    /// Fork turns are skipped — they reuse the parent session and aren't a
    /// distinct rollout. With a dropped kernel this still must not panic.
    #[test]
    fn hook_skips_fork_turns() {
        let hook = RlExportTurnEndHook::new(std::sync::Weak::new());
        let ctx = HookContext {
            agent_name: "probe",
            agent_id: &uuid::Uuid::new_v4().to_string(),
            event: HookEvent::AgentLoopEnd,
            data: serde_json::json!({"is_fork": true}),
        };
        assert!(hook.on_event(&ctx).is_ok());
    }

    /// Non-AgentLoopEnd events are silent no-ops.
    #[test]
    fn hook_ignores_unrelated_events() {
        let hook = RlExportTurnEndHook::new(std::sync::Weak::new());
        for event in [
            HookEvent::BeforeToolCall,
            HookEvent::AfterToolCall,
            HookEvent::BeforePromptBuild,
        ] {
            let ctx = HookContext {
                agent_name: "probe",
                agent_id: &uuid::Uuid::new_v4().to_string(),
                event,
                data: serde_json::json!({}),
            };
            assert!(hook.on_event(&ctx).is_ok(), "event {event:?} ignored");
        }
    }

    // ── Kernel-backed producer behaviour ──────────────────────────────────

    use crate::kernel::LibreFangKernel;
    use librefang_types::agent::{AgentManifest, ModelConfig};

    /// Boot a throwaway kernel rooted in a tempdir. Returns the kernel and
    /// the tempdir guard (must outlive the kernel).
    fn boot_kernel(rl_export: RlExportConfig) -> (LibreFangKernel, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).expect("home dir");
        let config = KernelConfig {
            home_dir: home.clone(),
            data_dir: home.join("data"),
            rl_export,
            ..KernelConfig::default()
        };
        let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boots");
        (kernel, tmp)
    }

    /// Spawn a minimal agent and return its id.
    fn spawn_test_agent(kernel: &LibreFangKernel, rl_enabled: Option<bool>) -> AgentId {
        // Unique per spawn so a single test can register several agents in one
        // kernel without colliding on the deterministic name-derived AgentId.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let manifest = AgentManifest {
            name: format!("rl-export-test-agent-{unique}"),
            description: "produces rollouts".to_string(),
            author: "test".to_string(),
            module: "builtin:chat".to_string(),
            model: ModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                max_tokens: 4096,
                temperature: 0.7,
                system_prompt: "you are a test".to_string(),
                api_key_env: None,
                base_url: None,
                context_window: None,
                max_output_tokens: None,
                extra_params: std::collections::BTreeMap::new(),
            },
            rl_export: librefang_types::agent::RlExportOverride {
                enabled: rl_enabled,
            },
            ..Default::default()
        };
        kernel
            .spawn_agent_inner(manifest, None, None, None)
            .expect("agent spawns")
    }

    /// Global toggle off and no per-agent override → export not enabled.
    /// This is the config-disabled = no export contract: `build_payload`
    /// would never run because the gate short-circuits.
    #[test]
    fn disabled_config_does_not_enable_export() {
        let (kernel, _tmp) = boot_kernel(RlExportConfig::default());
        let agent_id = spawn_test_agent(&kernel, None);
        assert!(!export_enabled_for_agent(&kernel, agent_id));
        kernel.shutdown();
    }

    /// Per-agent override `Some(true)` opts a single agent in even when the
    /// global toggle is off; `Some(false)` opts out even when global is on.
    #[test]
    fn per_agent_override_supersedes_global() {
        // Global off, agent on.
        let (kernel, _tmp) = boot_kernel(RlExportConfig::default());
        let agent_in = spawn_test_agent(&kernel, Some(true));
        assert!(export_enabled_for_agent(&kernel, agent_in));
        kernel.shutdown();

        // Global on, agent off.
        let (kernel, _tmp) = boot_kernel(RlExportConfig {
            enabled: true,
            target: None,
        });
        let agent_out = spawn_test_agent(&kernel, Some(false));
        assert!(!export_enabled_for_agent(&kernel, agent_out));
        // An agent with no override inherits the global `true`.
        let agent_inherit = spawn_test_agent(&kernel, None);
        assert!(export_enabled_for_agent(&kernel, agent_inherit));
        kernel.shutdown();
    }

    /// Config-enabled path assembles a payload with the correct run_id shape
    /// (`<agent_id>:<session_id>`) and non-empty JSONL trajectory bytes
    /// (at minimum the metadata header line is always present).
    #[test]
    fn enabled_config_assembles_payload() {
        let (kernel, _tmp) = boot_kernel(RlExportConfig {
            enabled: true,
            target: Some(RlExportTarget::Wandb {
                project: "p".into(),
                entity: "e".into(),
                run_id: None,
                api_key_env: "K".into(),
            }),
        });
        let agent_id = spawn_test_agent(&kernel, None);
        assert!(export_enabled_for_agent(&kernel, agent_id));

        let session_id = kernel
            .agent_registry_ref()
            .get(agent_id)
            .expect("agent entry")
            .session_id;

        let payload = build_payload(&kernel, agent_id).expect("payload assembled");
        assert_eq!(payload.run_id, format!("{}:{}", agent_id, session_id.0));
        assert!(
            !payload.trajectory_bytes.is_empty(),
            "trajectory bytes must be non-empty (JSONL metadata header)"
        );
        // The JSONL bytes are valid UTF-8 and the first line is the metadata
        // header — proves we forwarded the bundle's to_jsonl() output.
        let text = String::from_utf8(payload.trajectory_bytes).expect("utf-8 jsonl");
        let first_line = text.lines().next().expect("at least one line");
        assert!(
            first_line.contains("\"kind\":\"metadata\""),
            "first JSONL line must be the metadata header: {first_line}"
        );
        assert!(payload.toolset_metadata.is_some());
        kernel.shutdown();
    }
}
