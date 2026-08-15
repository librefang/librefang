//! [`kernel_handle::ApprovalGate`] — tool-approval policy + RBAC gate.
//! Holds the synchronous "does this tool require approval?" predicates and the async request/submit/resolve flow used by the agent loop.
//! Hand-tagged agents (curated trusted packages) auto-approve on the context-carrying non-blocking path unless per-user policy demanded human approval (RBAC M3, #3054).

use tracing::{debug, info};

use librefang_runtime::kernel_handle;
use librefang_types::agent::AgentId;
use librefang_types::tool::ToolApprovalSubmission;

use super::super::{spawn_logged, LibreFangKernel, SYSTEM_CHANNEL_AUTONOMOUS, SYSTEM_CHANNEL_CRON};

#[async_trait::async_trait]
impl kernel_handle::ApprovalGate for LibreFangKernel {
    fn requires_approval(&self, tool_name: &str) -> bool {
        self.governance
            .approval_manager
            .requires_approval(tool_name)
    }

    fn requires_approval_with_context(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> bool {
        self.governance
            .approval_manager
            .requires_approval_with_context(tool_name, sender_id, channel)
    }

    fn is_tool_denied_with_context(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> bool {
        self.governance
            .approval_manager
            .is_tool_denied_with_context(tool_name, sender_id, channel)
    }

    fn resolve_user_tool_decision(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
        system_call: bool,
    ) -> librefang_types::user_policy::UserToolGate {
        // The synthetic `"cron"` and `"autonomous"` channels are the only
        // two the kernel treats as system-internal by *channel*. Both are
        // synthesised by the kernel itself for daemon-driven calls that
        // have no user-facing sender:
        //   - `"cron"` — `kernel/mod.rs::start_periodic_loops` cron tick
        //     (~line 11950) for `[[cron_jobs]]` fires.
        //   - `"autonomous"` — `start_continuous_autonomous_loop`
        //     (~line 12412) for autonomous-tick prompts on agents whose
        //     manifest declares `[autonomous]`.
        // Both fan out the agent's own loop with a synthetic
        // `SenderContext { channel: "cron" | "autonomous" }`. Issue #3243
        // tracks the autonomous case: without this carve-out, every
        // autonomous tool call falls into `guest_gate` → NeedsApproval
        // and floods the approval queue when RBAC is enabled.
        //
        // The `system_call` argument is the channel-less counterpart:
        // system-internal forks (currently only auto_dream) run through
        // `run_forked_agent_streaming` on the parent's canonical session
        // with a `None` sender context, so they have no synthetic channel
        // to match here. They flag themselves via `LoopOptions.system_call`
        // instead, which the runtime dispatch forwards as this argument
        // (#6463). Either signal — the flag OR one of the two system
        // channels — bypasses the per-user gate.
        //
        // Earlier drafts also matched `"system"` / `"internal"` and
        // treated `(None, None)` as system, but neither sentinel is
        // synthesised anywhere in the codebase, and the `(None, None)`
        // shortcut silently re-opened the H7 fail-open at the trait
        // boundary the AuthManager unit tests were written to close
        // (PR #3205 review item #1). Both have been removed: an
        // unattributed inbound (no flag, no system channel) now goes
        // through the guest gate so RBAC fails closed end-to-end.
        let system_call = system_call
            || matches!(
                channel,
                Some(c) if c == SYSTEM_CHANNEL_CRON || c == SYSTEM_CHANNEL_AUTONOMOUS
            );
        self.security
            .auth
            .resolve_user_tool_decision(tool_name, sender_id, channel, system_call)
    }

    async fn request_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
        session_id: Option<&str>,
    ) -> Result<librefang_types::approval::ApprovalDecision, kernel_handle::KernelOpError> {
        use librefang_types::approval::ApprovalRequest as TypedRequest;

        // The blocking trait carries neither sender identity nor `force_human`.
        // It therefore cannot prove that per-user policy allowed a hand-agent carve-out.
        // Always queue a real approval on this context-free path; context-carrying hand execution uses `submit_tool_approval` below.

        let policy = self.governance.approval_manager.policy();
        let risk_level = crate::approval::ApprovalManager::classify_risk(tool_name);
        let agent_display = self.approval_agent_display(agent_id);
        let description = format!("Agent {} requests to execute {}", agent_display, tool_name);
        let request_id = uuid::Uuid::new_v4();
        let req = TypedRequest {
            id: request_id,
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            description: description.clone(),
            action_summary: action_summary
                .chars()
                .take(librefang_types::approval::MAX_ACTION_SUMMARY_LEN)
                .collect(),
            risk_level,
            requested_at: chrono::Utc::now(),
            timeout_secs: policy.timeout_secs,
            sender_id: None,
            channel: None,
            chat_id: None,
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: session_id.map(|s| s.to_string()),
            // Blocking path is used by tools that wait inline; the
            // KernelHandle::request_approval signature does not carry
            // the originating LLM tool_use_id today, so the ACP
            // adapter falls back to `approval-{req_id}` for these.
            tool_use_id: None,
        };

        // Publish an ApprovalRequested event so channel adapters can notify users.
        // Blocking path: the `KernelHandle::request_approval` signature does
        // not carry sender context (it's used by tools that wait inline),
        // so sender_id / channel are `None` here. Channel listener falls
        // back to its `notification_recipients` + `AgentBinding` fan-out
        // for these — same behaviour as pre-fix.
        {
            use librefang_types::event::{
                ApprovalRequestedEvent, Event, EventPayload, EventTarget,
            };
            let event = Event::new(
                agent_id.parse().unwrap_or_default(),
                EventTarget::System,
                EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                    request_id: request_id.to_string(),
                    agent_id: agent_id.to_string(),
                    tool_name: tool_name.to_string(),
                    description: description.clone(),
                    risk_level: format!("{:?}", risk_level),
                    sender_id: None,
                    channel: None,
                    chat_id: None,
                }),
            );
            self.events.event_bus.publish(event).await;
        }

        // Push approval notification to configured channels.
        // Resolution order: per-request route_to > policy routing rules > per-agent rules > global defaults.
        {
            use librefang_types::capability::glob_matches;

            let cfg = self.config.load_full();
            let policy = self.governance.approval_manager.policy();
            let targets: Vec<librefang_types::approval::NotificationTarget> =
                if !req.route_to.is_empty() {
                    // Highest priority: explicitly routed targets on the request itself
                    req.route_to.clone()
                } else {
                    // Check policy routing rules (match tool_pattern)
                    let routed: Vec<librefang_types::approval::NotificationTarget> = policy
                        .routing
                        .iter()
                        .filter(|r| glob_matches(&r.tool_pattern, tool_name))
                        .flat_map(|r| r.route_to.clone())
                        .collect();
                    if !routed.is_empty() {
                        routed
                    } else {
                        // Check per-agent notification rules
                        let agent_routed: Vec<librefang_types::approval::NotificationTarget> = cfg
                            .notification
                            .agent_rules
                            .iter()
                            .filter(|rule| {
                                glob_matches(&rule.agent_pattern, agent_id)
                                    && rule.events.iter().any(|e| e == "approval_requested")
                            })
                            .flat_map(|rule| rule.channels.clone())
                            .collect();
                        if !agent_routed.is_empty() {
                            agent_routed
                        } else {
                            // Fallback: global approval_channels
                            cfg.notification.approval_channels.clone()
                        }
                    }
                };

            let msg = format!(
                "{} Approval needed: agent {} wants to run `{}` — {}",
                risk_level.emoji(),
                agent_display,
                tool_name,
                description,
            );
            let req_id_str = request_id.to_string();
            for target in &targets {
                self.push_approval_interactive(target, &msg, &req_id_str)
                    .await;
            }
        }

        let decision = self.governance.approval_manager.request_approval(req).await;

        // Publish resolved event so channel adapters can notify outcome
        {
            use librefang_types::event::{ApprovalResolvedEvent, Event, EventPayload, EventTarget};
            let event = Event::new(
                agent_id.parse().unwrap_or_default(),
                EventTarget::System,
                EventPayload::ApprovalResolved(ApprovalResolvedEvent {
                    request_id: request_id.to_string(),
                    agent_id: agent_id.to_string(),
                    tool_name: tool_name.to_string(),
                    decision: decision.as_str().to_string(),
                    decided_by: None,
                }),
            );
            self.events.event_bus.publish(event).await;
        }

        Ok(decision)
    }

    async fn submit_tool_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
        deferred: librefang_types::tool::DeferredToolExecution,
        session_id: Option<&str>,
    ) -> Result<ToolApprovalSubmission, kernel_handle::KernelOpError> {
        use librefang_types::approval::ApprovalRequest as TypedRequest;

        // Hand agents are curated trusted packages — auto-approve for non-blocking execution.
        // EXCEPTION (RBAC M3, #3054): when the per-user policy demanded approval
        // (`force_human=true`), the carve-out MUST NOT fire — otherwise a Viewer/User
        // chatting with a hand-tagged agent silently inherits the agent's full
        // tool surface, defeating user-level RBAC entirely.
        if !deferred.force_human {
            if let Ok(aid) = agent_id.parse::<AgentId>() {
                if let Some(entry) = self.agents.registry.get(aid) {
                    if entry.tags.iter().any(|t| t.starts_with("hand:")) {
                        info!(
                            agent_id,
                            tool_name, "Auto-approved for hand agent (non-blocking)"
                        );
                        return Ok(ToolApprovalSubmission::AutoApproved);
                    }
                }
            }
        } else {
            debug!(
                agent_id,
                tool_name, "Hand-agent auto-approval skipped because user policy demanded approval"
            );
        }

        let policy = self.governance.approval_manager.policy();
        // #5600: per-session approval cache. If this exact tool name has
        // already been approved earlier in the same session, skip the
        // prompt. Gated by `policy.cache_approvals_per_session`
        // (default true).
        //
        // SECURITY (RBAC M3, #3054): MUST be skipped when
        // `deferred.force_human=true`. The user-policy gate flips
        // `force_human` on every call that demanded human approval —
        // honouring the session cache here would let the second call
        // of a tool silently bypass the per-call approval that the
        // RBAC M3 carve-out is designed to enforce.
        if policy.cache_approvals_per_session && !deferred.force_human {
            if let Some(sid) = session_id {
                if self
                    .governance
                    .approval_manager
                    .has_session_approval(sid, tool_name)
                {
                    info!(
                        agent_id,
                        tool_name,
                        session_id = sid,
                        "Auto-approved by per-session cache (#5600)"
                    );
                    return Ok(ToolApprovalSubmission::AutoApproved);
                }
            }
        }
        let risk_level = crate::approval::ApprovalManager::classify_risk(tool_name);
        let agent_display = self.approval_agent_display(agent_id);
        let description = format!("Agent {} requests to execute {}", agent_display, tool_name);
        let request_id = uuid::Uuid::new_v4();
        // The deferred payload built in `tool_runner::dispatch.rs`
        // carries the channel + sender from the agent_loop's
        // `SenderContext`. Pre-fix these were hardcoded to `None`,
        // which is what stranded Telegram approvals — the channel
        // listener had no idea which chat to route the
        // `[Approve] [Deny]` keyboard to and silently dropped the
        // notification (only a DEBUG line, nothing visible to the
        // operator).
        let routed_sender_id = deferred.sender_id.clone();
        let routed_channel = deferred.channel.clone();
        let routed_chat_id = deferred.chat_id.clone();
        let req = TypedRequest {
            id: request_id,
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            description: description.clone(),
            action_summary: action_summary
                .chars()
                .take(librefang_types::approval::MAX_ACTION_SUMMARY_LEN)
                .collect(),
            risk_level,
            requested_at: chrono::Utc::now(),
            timeout_secs: policy.timeout_secs,
            sender_id: routed_sender_id.clone(),
            channel: routed_channel.clone(),
            chat_id: routed_chat_id.clone(),
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: session_id.map(|s| s.to_string()),
            // Carry the LLM-assigned tool_use_id forward so the ACP
            // adapter can attach the editor's permission modal to the
            // streaming `ToolCall` card the editor already rendered
            // (#3313). The deferred payload is the canonical source.
            tool_use_id: Some(deferred.tool_use_id.clone()),
        };

        self.governance
            .approval_manager
            .submit_request(req.clone(), deferred)
            .map_err(|e| e.to_string())?;

        // Publish event + push notification (same as blocking path).
        // The channel listener subscribes to the EventBus (NOT the
        // approval_manager's own broadcast); the new sender_id +
        // channel fields are what let it route the `[Approve] [Deny]`
        // keyboard straight back to the originating chat without
        // needing any `notification_recipients` / `AgentBinding`
        // operator-inbox configuration.
        {
            use librefang_types::event::{
                ApprovalRequestedEvent, Event, EventPayload, EventTarget,
            };
            let event = Event::new(
                agent_id.parse().unwrap_or_default(),
                EventTarget::System,
                EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                    request_id: request_id.to_string(),
                    agent_id: agent_id.to_string(),
                    tool_name: tool_name.to_string(),
                    description: description.clone(),
                    risk_level: format!("{:?}", risk_level),
                    sender_id: routed_sender_id,
                    channel: routed_channel,
                    chat_id: routed_chat_id,
                }),
            );
            self.events.event_bus.publish(event).await;
        }
        {
            use librefang_types::capability::glob_matches;
            let cfg = self.config.load_full();
            let targets: Vec<librefang_types::approval::NotificationTarget> = {
                let routed: Vec<_> = policy
                    .routing
                    .iter()
                    .filter(|r| glob_matches(&r.tool_pattern, tool_name))
                    .flat_map(|r| r.route_to.clone())
                    .collect();
                if !routed.is_empty() {
                    routed
                } else {
                    let agent_routed: Vec<_> = cfg
                        .notification
                        .agent_rules
                        .iter()
                        .filter(|rule| {
                            glob_matches(&rule.agent_pattern, agent_id)
                                && rule.events.iter().any(|e| e == "approval_requested")
                        })
                        .flat_map(|rule| rule.channels.clone())
                        .collect();
                    if !agent_routed.is_empty() {
                        agent_routed
                    } else {
                        cfg.notification.approval_channels.clone()
                    }
                }
            };
            let msg = format!(
                "{} Approval needed: agent {} wants to run `{}` — {}",
                risk_level.emoji(),
                agent_display,
                tool_name,
                description,
            );
            let req_id_str = request_id.to_string();
            for target in &targets {
                self.push_approval_interactive(target, &msg, &req_id_str)
                    .await;
            }
        }

        Ok(ToolApprovalSubmission::Pending { request_id })
    }

    async fn resolve_tool_approval(
        &self,
        request_id: uuid::Uuid,
        decision: librefang_types::approval::ApprovalDecision,
        decided_by: Option<String>,
        totp_verified: bool,
        user_id: Option<&str>,
    ) -> Result<
        (
            librefang_types::approval::ApprovalResponse,
            Option<librefang_types::tool::DeferredToolExecution>,
        ),
        kernel_handle::KernelOpError,
    > {
        // #3541 follow-up: classify the missing-id case as
        // `KernelOpError::AgentNotFound` / `Internal` so the API
        // boundary surfaces 404 via the typed mapping. The underlying
        // `ApprovalManager::resolve` still returns `String` (typing it
        // is left to a separate ApprovalManager refactor); the substring
        // check is scoped to the manager's exact "not found or expired"
        // wording. All other error wordings flow through `Internal`.
        let (response, deferred_with_kernel) = self
            .governance
            .approval_manager
            .resolve_with_deferred_preflight(
                request_id,
                decision,
                decided_by,
                totp_verified,
                user_id,
                || {
                    self.self_handle
                        .get()
                        .and_then(|weak| weak.upgrade())
                        .ok_or_else(|| "Kernel self-handle unavailable".to_string())
                },
            )
            .map_err(|msg| {
                if msg.starts_with("Already ") {
                    // Double-resolve of an already-terminal approval ("Already {decision} by {who}") is a state conflict, not a malformed request: map to `Conflict` (409) so a client can tell "someone already handled this" apart from a bad request (400) or a never-existed id (404).
                    // `resolve` consults the durable audit log, so this stays a stable 409 even after the in-memory `recent` ring evicts the entry or the daemon restarts (issue #6492 Bug 3).
                    kernel_handle::KernelOpError::Conflict(msg)
                } else if msg.contains("not found") {
                    kernel_handle::KernelOpError::AgentNotFound(request_id.to_string())
                } else if msg.contains("TOTP code required") {
                    // A missing second factor is a well-formed request that lacks a required field → 400, not 500.
                    // Map to `InvalidInput` so the typed status mapping (`api::error::kernel_op_status`) keeps the pre-#3541 400.
                    kernel_handle::KernelOpError::InvalidInput(msg)
                } else {
                    kernel_handle::KernelOpError::Internal(msg)
                }
            })?;

        // Deferred approval execution resumes in the background so API callers do
        // not block on slow tools.
        let deferred = deferred_with_kernel.map(|(def, kernel)| {
            let decision_clone = response.decision.clone();
            let deferred_clone = def.clone();
            spawn_logged("approval_resolution", async move {
                kernel
                    .handle_approval_resolution(request_id, decision_clone, deferred_clone)
                    .await;
            });
            def
        });

        Ok((response, deferred))
    }

    fn get_approval_status(
        &self,
        request_id: uuid::Uuid,
    ) -> Result<Option<librefang_types::approval::ApprovalDecision>, kernel_handle::KernelOpError>
    {
        // If still pending, no decision yet.
        if self
            .governance
            .approval_manager
            .get_pending(request_id)
            .is_some()
        {
            return Ok(None);
        }
        // Check recent resolved records.
        let recent = self.governance.approval_manager.list_recent(200);
        for record in &recent {
            if record.request.id == request_id {
                return Ok(Some(record.decision.clone()));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::approval::{ApprovalDecision, ApprovalRequest, RiskLevel};
    use librefang_types::config::KernelConfig;
    use librefang_types::tool::DeferredToolExecution;

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_self_handle_leaves_deferred_approval_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            ..KernelConfig::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
        assert!(kernel.self_handle.get().is_none());

        let request_id = uuid::Uuid::new_v4();
        let request = ApprovalRequest {
            id: request_id,
            agent_id: AgentId::new().to_string(),
            tool_name: "shell_exec".to_string(),
            description: "run command".to_string(),
            action_summary: "run command".to_string(),
            risk_level: RiskLevel::High,
            requested_at: chrono::Utc::now(),
            timeout_secs: 60,
            sender_id: None,
            channel: None,
            chat_id: None,
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: None,
            tool_use_id: Some("tool-use-test".to_string()),
        };
        let deferred = DeferredToolExecution {
            agent_id: request.agent_id.clone(),
            tool_use_id: "tool-use-test".to_string(),
            tool_name: request.tool_name.clone(),
            input: serde_json::json!({}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            chat_id: None,
            account_id: None,
            workspace_root: None,
            force_human: false,
            session_id: None,
        };
        kernel
            .governance
            .approval_manager
            .submit_request(request, deferred)
            .expect("submit approval");

        let error = kernel_handle::ApprovalGate::resolve_tool_approval(
            &kernel,
            request_id,
            ApprovalDecision::Approved,
            Some("test".to_string()),
            false,
            None,
        )
        .await
        .expect_err("missing self-handle must fail before resolution");

        assert!(matches!(
            error,
            kernel_handle::KernelOpError::Internal(ref message)
                if message == "Kernel self-handle unavailable"
        ));
        assert!(
            kernel
                .governance
                .approval_manager
                .get_pending(request_id)
                .is_some(),
            "failed preflight must leave approval retryable"
        );

        kernel.shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_self_handle_does_not_block_manual_approval() {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            ..KernelConfig::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
        assert!(kernel.self_handle.get().is_none());

        let request_id = uuid::Uuid::new_v4();
        let request = ApprovalRequest {
            id: request_id,
            agent_id: AgentId::new().to_string(),
            tool_name: "shell_exec".to_string(),
            description: "run command".to_string(),
            action_summary: "run command".to_string(),
            risk_level: RiskLevel::High,
            requested_at: chrono::Utc::now(),
            timeout_secs: 60,
            sender_id: None,
            channel: None,
            chat_id: None,
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: None,
            tool_use_id: None,
        };
        kernel
            .governance
            .approval_manager
            .submit_manual_request(request)
            .expect("submit approval");

        let (response, deferred) = kernel_handle::ApprovalGate::resolve_tool_approval(
            &kernel,
            request_id,
            ApprovalDecision::Denied,
            Some("test".to_string()),
            false,
            None,
        )
        .await
        .expect("manual approval does not need deferred resume state");

        assert_eq!(response.decision, ApprovalDecision::Denied);
        assert!(deferred.is_none());
        assert!(kernel
            .governance
            .approval_manager
            .get_pending(request_id)
            .is_none());

        kernel.shutdown();
    }
}
