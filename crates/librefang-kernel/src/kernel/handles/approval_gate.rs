//! [`kernel_handle::ApprovalGate`] — tool-approval policy + RBAC gate. Holds
//! the synchronous "does this tool require approval?" predicates and the
//! async request/submit/resolve flow used by the agent loop. Hand-tagged
//! agents (curated trusted packages) auto-approve unless the per-user
//! policy demanded human approval (RBAC M3, #3054).

use std::sync::Arc;

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
    ) -> librefang_types::user_policy::UserToolGate {
        // The synthetic `"cron"` and `"autonomous"` channels are the only
        // two the kernel treats as system-internal. Both are synthesised
        // by the kernel itself for daemon-driven calls that have no
        // user-facing sender:
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
        // Earlier drafts also matched `"system"` / `"internal"` and
        // treated `(None, None)` as system, but neither sentinel is
        // synthesised anywhere in the codebase, and the `(None, None)`
        // shortcut silently re-opened the H7 fail-open at the trait
        // boundary the AuthManager unit tests were written to close
        // (PR #3205 review item #1). Both have been removed: an
        // unattributed inbound now goes through the guest gate so
        // RBAC fails closed end-to-end.
        let system_call = matches!(
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
        use librefang_types::approval::{ApprovalDecision, ApprovalRequest as TypedRequest};

        // Hand agents are curated trusted packages — auto-approve tool execution.
        // Check if this agent has a "hand:" tag indicating it was spawned by activate_hand().
        if let Ok(aid) = agent_id.parse::<AgentId>() {
            if let Some(entry) = self.agents.registry.get(aid) {
                if entry.tags.iter().any(|t| t.starts_with("hand:")) {
                    info!(agent_id, tool_name, "Auto-approved for hand agent");
                    return Ok(ApprovalDecision::Approved);
                }
            }
        }

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
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: session_id.map(|s| s.to_string()),
        };

        // Publish an ApprovalRequested event so channel adapters can notify users
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
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: session_id.map(|s| s.to_string()),
        };

        self.governance
            .approval_manager
            .submit_request(req.clone(), deferred)
            .map_err(|e| e.to_string())?;

        // Publish event + push notification (same as blocking path)
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
        let (response, deferred) = self
            .governance
            .approval_manager
            .resolve(request_id, decision, decided_by, totp_verified, user_id)
            .map_err(|msg| {
                if msg.contains("not found") {
                    kernel_handle::KernelOpError::AgentNotFound(request_id.to_string())
                } else {
                    kernel_handle::KernelOpError::Internal(msg)
                }
            })?;

        // Deferred approval execution resumes in the background so API callers do
        // not block on slow tools.
        if let Some(ref def) = deferred {
            let decision_clone = response.decision.clone();
            let kernel = Arc::clone(
                self.self_handle
                    .get()
                    .and_then(|w| w.upgrade())
                    .as_ref()
                    .ok_or_else(|| "Kernel self-handle unavailable".to_string())?,
            );
            let deferred_clone = def.clone();
            spawn_logged("approval_resolution", async move {
                kernel
                    .handle_approval_resolution(request_id, decision_clone, deferred_clone)
                    .await;
            });
        }

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
