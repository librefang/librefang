//! Execution approval manager — gates dangerous operations behind human approval.

use chrono::Utc;
use dashmap::DashMap;
use librefang_types::approval::{
    ApprovalAuditEntry, ApprovalDecision, ApprovalPolicy, ApprovalRequest, ApprovalResponse,
    RiskLevel, SecondFactor, TimeoutFallback,
};
use librefang_types::capability::glob_matches;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Instant;
use totp_rs::{Algorithm, Secret, TOTP};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Max pending requests per agent.
const MAX_PENDING_PER_AGENT: usize = 5;
/// Max recent approval records to retain for history and UI visibility.
const MAX_RECENT_APPROVALS: usize = 100;
/// Max escalation rounds before falling back to TimedOut.
const MAX_ESCALATIONS: u8 = 3;

/// Max consecutive TOTP failures before lockout.
const TOTP_MAX_FAILURES: u32 = 5;
/// TOTP lockout duration after max failures.
const TOTP_LOCKOUT_SECS: u64 = 300;

/// Re-export from librefang-types so approval.rs consumers don't need two imports.
pub use librefang_types::tool::DeferredToolExecution;

/// Manages approval requests for both blocking and deferred execution paths.
pub struct ApprovalManager {
    pending: DashMap<Uuid, PendingRequest>,
    recent: std::sync::Mutex<VecDeque<ApprovalRecord>>,
    policy: std::sync::RwLock<ApprovalPolicy>,
    audit_db: Option<Arc<StdMutex<Connection>>>,
    /// TOTP grace period cache: sender_id → last successful verification time.
    totp_grace: StdMutex<HashMap<String, Instant>>,
    /// TOTP failure tracking: sender_id → (failure_count, lockout_start).
    /// `lockout_start` is `None` until the failure count reaches the threshold,
    /// at which point it is set to the current instant. The lockout window is
    /// measured from that moment, not from the first failure.
    totp_failures: StdMutex<HashMap<String, (u32, Option<Instant>)>>,
}

struct PendingRequest {
    request: ApprovalRequest,
    sender: Option<tokio::sync::oneshot::Sender<ApprovalDecision>>,
    deferred: Option<DeferredToolExecution>,
    submitted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct EscalatedApproval {
    pub request_id: Uuid,
    pub request: ApprovalRequest,
}

#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    pub request: ApprovalRequest,
    pub decision: ApprovalDecision,
    pub decided_at: chrono::DateTime<Utc>,
    pub decided_by: Option<String>,
}

impl ApprovalManager {
    fn pending_count_for_agent(&self, agent_id: &str) -> usize {
        self.pending
            .iter()
            .filter(|r| r.value().request.agent_id == agent_id)
            .count()
    }

    pub fn new(policy: ApprovalPolicy) -> Self {
        Self {
            pending: DashMap::new(),
            recent: std::sync::Mutex::new(VecDeque::new()),
            policy: std::sync::RwLock::new(policy),
            audit_db: None,
            totp_grace: StdMutex::new(HashMap::new()),
            totp_failures: StdMutex::new(HashMap::new()),
        }
    }

    /// Create an approval manager with persistent audit logging.
    pub fn new_with_db(policy: ApprovalPolicy, conn: Arc<StdMutex<Connection>>) -> Self {
        let failures = Self::load_totp_lockout(&conn);
        Self {
            pending: DashMap::new(),
            recent: std::sync::Mutex::new(VecDeque::new()),
            policy: std::sync::RwLock::new(policy),
            audit_db: Some(conn),
            totp_grace: StdMutex::new(HashMap::new()),
            totp_failures: StdMutex::new(failures),
        }
    }

    /// Load persisted TOTP lockout state from the database.
    ///
    /// Entries whose lockout window has already expired are discarded at load
    /// time so a daemon restart does not extend the lockout beyond the original
    /// 5-minute window.
    fn load_totp_lockout(
        conn: &Arc<StdMutex<Connection>>,
    ) -> HashMap<String, (u32, Option<Instant>)> {
        let Ok(guard) = conn.lock() else {
            return HashMap::new();
        };
        let Ok(mut stmt) = guard.prepare("SELECT sender_id, failures, locked_at FROM totp_lockout")
        else {
            return HashMap::new();
        };

        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u32,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .ok();

        let Some(rows) = rows else {
            return HashMap::new();
        };
        let mut map = HashMap::new();
        for row in rows.filter_map(|r| r.ok()) {
            let (sender_id, failures, locked_at_unix) = row;
            let lockout_start = locked_at_unix.and_then(|ts| {
                let ts = ts as u64;
                let elapsed = now_unix.saturating_sub(ts);
                if elapsed >= TOTP_LOCKOUT_SECS {
                    None // Lockout has expired — don't restore
                } else {
                    // Reconstruct an Instant that is `elapsed` seconds in the past
                    Some(Instant::now() - std::time::Duration::from_secs(elapsed))
                }
            });
            // If lockout_start is None but failures >= threshold, the lockout
            // expired during the downtime — reset the counter.
            if failures >= TOTP_MAX_FAILURES && lockout_start.is_none() {
                // Expired — omit entry entirely (effective reset)
                continue;
            }
            map.insert(sender_id, (failures, lockout_start));
        }
        map
    }

    /// Check if a tool requires approval based on current policy.
    ///
    /// Entries in the `require_approval` list support wildcard patterns
    /// (e.g. `"file_*"` matches `"file_read"`, `"file_write"`, etc.).
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
        policy
            .require_approval
            .iter()
            .any(|pattern| glob_matches(pattern, tool_name))
    }

    /// Check whether a tool is hard-denied in the current sender/channel context.
    pub fn is_tool_denied_with_context(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> bool {
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());

        if let Some(sid) = sender_id {
            if policy.is_trusted_sender(sid) {
                debug!(
                    sender_id = sid,
                    tool_name, "Trusted sender — channel deny bypassed"
                );
                return false;
            }
        }

        channel
            .and_then(|ch| policy.check_channel_tool(ch, tool_name))
            .is_some_and(|allowed| !allowed)
    }

    /// Check if a tool requires approval, taking sender and channel context
    /// into account.
    ///
    /// Returns `false` (no approval needed) if:
    /// 1. The sender is in the `trusted_senders` list, OR
    /// 2. A channel rule explicitly allows the tool, OR
    /// 3. The tool is not in the `require_approval` list.
    ///
    /// Returns `true` (approval needed) if:
    /// 1. A channel rule explicitly denies the tool, OR
    /// 2. The tool is in `require_approval` and none of the above bypasses apply.
    pub fn requires_approval_with_context(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> bool {
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());

        // Trusted sender bypass: auto-approve all tools.
        if let Some(sid) = sender_id {
            if policy.is_trusted_sender(sid) {
                debug!(
                    sender_id = sid,
                    tool_name, "Trusted sender — approval bypassed"
                );
                return false;
            }
        }

        // Channel-specific rules.
        if let Some(ch) = channel {
            if let Some(allowed) = policy.check_channel_tool(ch, tool_name) {
                if !allowed {
                    debug!(channel = ch, tool_name, "Channel rule denies tool");
                    return true;
                }
                // Channel rule explicitly allows — bypass approval.
                debug!(
                    channel = ch,
                    tool_name, "Channel rule allows tool — approval bypassed"
                );
                return false;
            }
        }

        // Fall back to default require_approval list.
        policy
            .require_approval
            .iter()
            .any(|pattern| glob_matches(pattern, tool_name))
    }

    /// Submit an approval request. Blocks until approved, denied, or timed out.
    ///
    /// When `timeout_fallback` is `Escalate` and `escalation_count < MAX_ESCALATIONS`,
    /// a timeout re-inserts the request with bumped `escalation_count` so the caller
    /// can re-notify and re-call this method.
    pub async fn request_approval(&self, req: ApprovalRequest) -> ApprovalDecision {
        let fallback = self
            .policy
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .timeout_fallback
            .clone();
        let mut current_req = req;

        loop {
            let agent_pending = self.pending_count_for_agent(&current_req.agent_id);
            if agent_pending >= MAX_PENDING_PER_AGENT {
                warn!(agent_id = %current_req.agent_id, "Approval request rejected: too many pending");
                return ApprovalDecision::Denied;
            }

            let id = current_req.id;
            let escalation = current_req.escalation_count;
            let timeout =
                std::time::Duration::from_secs(effective_timeout_secs(&current_req, &fallback));
            let req_for_timeout = current_req.clone();

            let (tx, rx) = tokio::sync::oneshot::channel();
            self.pending.insert(
                id,
                PendingRequest {
                    request: current_req,
                    sender: Some(tx),
                    deferred: None,
                    submitted_at: chrono::Utc::now(),
                },
            );

            info!(request_id = %id, escalation, "Approval request submitted, waiting for resolution");

            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(decision)) => {
                    debug!(request_id = %id, ?decision, "Approval resolved");
                    return decision;
                }
                _ => match timeout_decision(&req_for_timeout, &fallback) {
                    ExpiryOutcome::Escalate => {
                        let mut escalated_req = self
                            .pending
                            .remove(&id)
                            .map(|(_, p)| p.request)
                            .unwrap_or(req_for_timeout);
                        escalated_req.escalation_count += 1;
                        warn!(
                            request_id = %id,
                            escalation = escalated_req.escalation_count,
                            "Approval timed out — escalating"
                        );
                        current_req = escalated_req;
                    }
                    ExpiryOutcome::Resolve(decision) => {
                        let request = self
                            .pending
                            .remove(&id)
                            .map(|(_, p)| p.request)
                            .unwrap_or(req_for_timeout);
                        self.push_recent(request, decision.clone(), None, Utc::now(), false);
                        warn!(request_id = %id, decision = %decision.as_str(), "Approval timed out");
                        return decision;
                    }
                },
            }
        }
    }

    /// Submit a tool for approval without blocking. Returns request UUID immediately.
    /// The DeferredToolExecution is stored and returned atomically on resolve().
    pub fn submit_request(
        &self,
        req: ApprovalRequest,
        deferred: DeferredToolExecution,
    ) -> Result<uuid::Uuid, String> {
        // Anti-duplicate guard: reject duplicate tool_use IDs so a single tool
        // call cannot be submitted twice, but allow identical inputs from
        // distinct tool calls in the same assistant response.
        let has_duplicate = self.pending.iter().any(|r| {
            if let Some(ref d) = r.value().deferred {
                d.tool_use_id == deferred.tool_use_id
            } else {
                false
            }
        });
        if has_duplicate {
            return Err("Duplicate approval request already pending".to_string());
        }

        // Per-agent pending limit
        let agent_pending_count = self.pending_count_for_agent(&req.agent_id);
        if agent_pending_count >= MAX_PENDING_PER_AGENT {
            return Err("Too many pending approval requests for this agent".to_string());
        }

        let id = req.id;
        self.pending.insert(
            id,
            PendingRequest {
                request: req,
                sender: None,
                deferred: Some(deferred),
                submitted_at: chrono::Utc::now(),
            },
        );
        Ok(id)
    }

    /// Sweep expired requests. Called periodically by kernel.
    /// Returns terminal decisions for deferred requests. Escalating requests stay pending.
    pub fn expire_pending_requests(
        &self,
    ) -> (
        Vec<EscalatedApproval>,
        Vec<(uuid::Uuid, ApprovalDecision, DeferredToolExecution)>,
    ) {
        let now = chrono::Utc::now();
        let mut escalated = Vec::new();
        let mut expired = Vec::new();
        let fallback = {
            let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
            policy.timeout_fallback.clone()
        };

        // Collect expired request IDs first to avoid holding iter while mutating
        let expired_ids: Vec<uuid::Uuid> = self
            .pending
            .iter()
            .filter(|entry| {
                let timeout_secs = effective_timeout_secs(&entry.value().request, &fallback);
                let elapsed = now.signed_duration_since(entry.value().submitted_at);
                elapsed > chrono::Duration::seconds(timeout_secs as i64)
            })
            .map(|entry| *entry.key())
            .collect();

        for id in expired_ids {
            if let Some((_, pending)) = self.pending.remove(&id) {
                match timeout_decision(&pending.request, &fallback) {
                    ExpiryOutcome::Escalate => {
                        let mut request = pending.request;
                        request.escalation_count += 1;
                        warn!(
                            request_id = %id,
                            escalation = request.escalation_count,
                            "Approval timed out, escalating deferred request"
                        );
                        self.pending.insert(
                            id,
                            PendingRequest {
                                request: request.clone(),
                                sender: pending.sender,
                                deferred: pending.deferred,
                                submitted_at: now,
                            },
                        );
                        escalated.push(EscalatedApproval {
                            request_id: id,
                            request,
                        });
                    }
                    ExpiryOutcome::Resolve(decision) => {
                        self.push_recent(
                            pending.request.clone(),
                            decision.clone(),
                            None,
                            now,
                            false,
                        );
                        if let Some(sender) = pending.sender {
                            let _ = sender.send(decision.clone());
                        }
                        if let Some(deferred) = pending.deferred {
                            expired.push((id, decision, deferred));
                        }
                    }
                }
            }
        }

        (escalated, expired)
    }

    /// Resolve a pending request (called by API/UI).
    ///
    /// Returns `(ApprovalResponse, Option<DeferredToolExecution>)` — the deferred payload
    /// is `Some` when the request was submitted via `submit_request()` (non-blocking path).
    ///
    /// When `second_factor` is `Totp` and the decision is `Approved`, the caller
    /// must verify the TOTP code *before* calling this method and set
    /// `totp_verified` to `true`. If TOTP is required but not verified,
    /// resolution is rejected.
    ///
    /// `user_id` identifies the actual human operator (for grace period tracking).
    /// This is distinct from `decided_by` which is the source label ("api", "channel").
    pub fn resolve(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        decided_by: Option<String>,
        totp_verified: bool,
        user_id: Option<&str>,
    ) -> Result<(ApprovalResponse, Option<DeferredToolExecution>), String> {
        // Read policy once and hold the snapshot for both the gate check and
        // the grace-period recording below, avoiding a hot-reload race between
        // two separate lock acquisitions.
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());

        // TOTP gate: only enforced on Approved decisions.
        // Peek at the pending request to get the tool_name for per-tool checks.
        if decision.is_approved() {
            let tool_needs_totp = self
                .pending
                .get(&request_id)
                .map(|p| policy.tool_requires_totp(&p.request.tool_name))
                .unwrap_or(false);
            if tool_needs_totp {
                let uid = user_id.unwrap_or("unknown");
                if !self.is_within_totp_grace(uid, &policy) && !totp_verified {
                    return Err("TOTP code required for approval (second_factor = totp)".into());
                }
            }
        }

        match self.pending.remove(&request_id) {
            Some((_, pending)) => {
                // Record TOTP grace on successful approval with TOTP.
                if decision.is_approved() && totp_verified {
                    if let Some(uid) = user_id {
                        if policy.tool_requires_totp(&pending.request.tool_name) {
                            self.record_totp_grace(uid);
                        }
                    }
                }

                let response = ApprovalResponse {
                    request_id,
                    decision: decision.clone(),
                    decided_at: Utc::now(),
                    decided_by,
                };
                self.push_recent(
                    pending.request.clone(),
                    response.decision.clone(),
                    response.decided_by.clone(),
                    response.decided_at,
                    totp_verified,
                );
                // Send decision to waiting agent via oneshot if present (blocking path)
                info!(request_id = %request_id, decision = ?response.decision, "Approval request resolved");
                if let Some(sender) = pending.sender {
                    let _ = sender.send(decision);
                }
                Ok((response, pending.deferred))
            }
            None => {
                // Check recent records for who already handled this
                let recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
                let handler_info = recent.iter().find(|r| r.request.id == request_id).map(|r| {
                    let who = r.decided_by.as_deref().unwrap_or("unknown");
                    let decision = r.decision.as_str();
                    format!("Already {decision} by {who}")
                });
                drop(recent);
                Err(handler_info.unwrap_or_else(|| {
                    format!("Approval request {request_id} not found or expired")
                }))
            }
        }
    }

    /// Resolve multiple pending requests in batch.
    ///
    /// Batch resolution does not support TOTP — callers must resolve
    /// individually when second_factor is enabled.
    pub fn resolve_batch(
        &self,
        ids: Vec<Uuid>,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    ) -> Vec<(Uuid, Result<ApprovalResponse, String>)> {
        ids.into_iter()
            .map(|id| {
                let result = self
                    .resolve(id, decision.clone(), decided_by.clone(), false, None)
                    .map(|(resp, _deferred)| resp);
                (id, result)
            })
            .collect()
    }

    /// Resolve all pending requests belonging to a specific session.
    ///
    /// This mirrors Hermes-Agent's `resolve_gateway_approval(session_key,
    /// choice, resolve_all=True)`: every request whose `session_id` matches
    /// is resolved atomically with the given decision.
    ///
    /// Returns the number of requests resolved (0 if the session had nothing
    /// pending).  This method does NOT spawn handle_approval_resolution for
    /// deferred payloads — callers that need deferred execution handling should
    /// use resolve_tool_approval (kernel) in a loop instead.
    ///
    /// TOTP is not enforced here: resolve() is called with totp_verified=false,
    /// so TOTP-required requests will return Err and not be counted.  Callers
    /// who want TOTP pre-enforcement should check policy.tool_requires_totp()
    /// before calling this method.
    pub fn resolve_all_for_session(
        &self,
        session_id: &str,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    ) -> usize {
        // Collect matching IDs without holding a long-lived lock on `pending`.
        let ids: Vec<Uuid> = self
            .pending
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .request
                    .session_id
                    .as_deref()
                    .is_some_and(|s| s == session_id)
            })
            .map(|entry| *entry.key())
            .collect();

        let mut count = 0usize;
        for id in ids {
            // Count only requests that were successfully resolved; ignore
            // individual errors (request may have expired or required TOTP
            // between the collect and the resolve call).
            if self
                .resolve(id, decision.clone(), decided_by.clone(), false, None)
                .is_ok()
            {
                count += 1;
            }
        }
        count
    }

    /// List all pending requests belonging to a specific session.
    ///
    /// Useful for dashboard views scoped to a single conversation/session,
    /// mirroring `has_blocking_approval(session_key)` from Hermes-Agent.
    pub fn list_pending_for_session(&self, session_id: &str) -> Vec<ApprovalRequest> {
        self.pending
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .request
                    .session_id
                    .as_deref()
                    .is_some_and(|s| s == session_id)
            })
            .map(|entry| entry.value().request.clone())
            .collect()
    }

    /// Check whether a session has one or more blocking approval requests
    /// currently pending.
    ///
    /// Mirrors `has_blocking_approval(session_key)` from Hermes-Agent.
    pub fn has_pending_for_session(&self, session_id: &str) -> bool {
        self.pending.iter().any(|entry| {
            entry
                .value()
                .request
                .session_id
                .as_deref()
                .is_some_and(|s| s == session_id)
        })
    }

    /// List all pending requests (for API/dashboard display).
    pub fn list_pending(&self) -> Vec<ApprovalRequest> {
        self.pending
            .iter()
            .map(|r| r.value().request.clone())
            .collect()
    }

    /// List recent non-pending approvals, newest first.
    pub fn list_recent(&self, limit: usize) -> Vec<ApprovalRecord> {
        let recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        recent.iter().take(limit).cloned().collect()
    }

    /// Get a single pending request by ID.
    pub fn get_pending(&self, id: Uuid) -> Option<ApprovalRequest> {
        self.pending.get(&id).map(|r| r.request.clone())
    }

    /// Number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Query the persistent audit log with pagination and optional filters.
    pub fn query_audit(
        &self,
        limit: usize,
        offset: usize,
        agent_id: Option<&str>,
        tool_name: Option<&str>,
    ) -> Vec<ApprovalAuditEntry> {
        let Some(db) = &self.audit_db else {
            return Vec::new();
        };
        let Ok(conn) = db.lock() else {
            return Vec::new();
        };

        let mut sql = String::from(
            "SELECT id, request_id, agent_id, tool_name, description, action_summary, risk_level, decision, decided_by, decided_at, requested_at, feedback, COALESCE(second_factor_used, 0) FROM approval_audit WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(aid) = agent_id {
            sql.push_str(" AND agent_id = ?");
            params.push(Box::new(aid.to_string()));
        }
        if let Some(tn) = tool_name {
            sql.push_str(" AND tool_name = ?");
            params.push(Box::new(tn.to_string()));
        }
        sql.push_str(" ORDER BY decided_at DESC LIMIT ? OFFSET ?");
        params.push(Box::new(limit as i64));
        params.push(Box::new(offset as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let Ok(mut stmt) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(ApprovalAuditEntry {
                id: row.get(0)?,
                request_id: row.get(1)?,
                agent_id: row.get(2)?,
                tool_name: row.get(3)?,
                description: row.get(4)?,
                action_summary: row.get(5)?,
                risk_level: row.get(6)?,
                decision: row.get(7)?,
                decided_by: row.get(8)?,
                decided_at: row.get(9)?,
                requested_at: row.get(10)?,
                feedback: row.get(11)?,
                second_factor_used: row.get::<_, bool>(12).unwrap_or(false),
            })
        });
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Count total audit entries (with optional filters).
    pub fn audit_count(&self, agent_id: Option<&str>, tool_name: Option<&str>) -> usize {
        let Some(db) = &self.audit_db else {
            return 0;
        };
        let Ok(conn) = db.lock() else {
            return 0;
        };

        let mut sql = String::from("SELECT COUNT(*) FROM approval_audit WHERE 1=1");
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(aid) = agent_id {
            sql.push_str(" AND agent_id = ?");
            params.push(Box::new(aid.to_string()));
        }
        if let Some(tn) = tool_name {
            sql.push_str(" AND tool_name = ?");
            params.push(Box::new(tn.to_string()));
        }

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        conn.query_row(&sql, param_refs.as_slice(), |row| row.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    /// Update the approval policy (for hot-reload).
    pub fn update_policy(&self, policy: ApprovalPolicy) {
        *self.policy.write().unwrap_or_else(|e| e.into_inner()) = policy;
    }

    /// Get a copy of the current policy.
    pub fn policy(&self) -> ApprovalPolicy {
        self.policy
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Classify the risk level of a tool invocation.
    pub fn classify_risk(tool_name: &str) -> RiskLevel {
        match tool_name {
            "shell_exec" => RiskLevel::Critical,
            "file_write" | "file_delete" | "apply_patch" => RiskLevel::High,
            "web_fetch" | "browser_navigate" => RiskLevel::Medium,
            _ => RiskLevel::Low,
        }
    }

    // -----------------------------------------------------------------------
    // TOTP helpers
    // -----------------------------------------------------------------------

    /// Check whether the current policy requires TOTP verification.
    pub fn requires_totp(&self) -> bool {
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
        policy.second_factor == SecondFactor::Totp
    }

    /// Verify a TOTP code against a base32-encoded secret.
    ///
    /// Uses RFC 6238 with SHA-1, 6 digits, 30-second step, and +-1 window tolerance.
    /// `issuer` should match the value used during enrollment (from `totp_issuer` in
    /// the approval policy); it is included in the TOTP struct for consistency but
    /// does not affect the HMAC computation.
    pub fn verify_totp_code(secret_base32: &str, code: &str) -> Result<bool, String> {
        Self::verify_totp_code_with_issuer(secret_base32, code, "LibreFang")
    }

    /// Like `verify_totp_code` but uses the provided issuer label.
    pub fn verify_totp_code_with_issuer(
        secret_base32: &str,
        code: &str,
        issuer: &str,
    ) -> Result<bool, String> {
        let secret = Secret::Encoded(secret_base32.to_string());
        let raw = secret
            .to_bytes()
            .map_err(|e| format!("Invalid TOTP secret: {e}"))?;
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            raw,
            Some(issuer.to_string()),
            String::new(),
        )
        .map_err(|e| format!("TOTP init error: {e}"))?;
        Ok(totp.check_current(code).unwrap_or(false))
    }

    /// Generate a new TOTP secret and return (base32_secret, otpauth_uri, qr_base64_png).
    pub fn generate_totp_secret(
        issuer: &str,
        account: &str,
    ) -> Result<(String, String, String), String> {
        let secret = Secret::generate_secret();
        let base32 = secret.to_encoded().to_string();
        let raw = secret
            .to_bytes()
            .map_err(|e| format!("Secret encoding error: {e}"))?;
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            raw,
            Some(issuer.to_string()),
            account.to_string(),
        )
        .map_err(|e| format!("TOTP init error: {e}"))?;
        let uri = totp.get_url();
        let qr_b64 = totp
            .get_qr_base64()
            .map_err(|e| format!("QR generation error: {e}"))?;
        Ok((base32, uri, qr_b64))
    }

    /// Generate 8 random recovery codes (format: xxxx-xxxx).
    pub fn generate_recovery_codes() -> Vec<String> {
        use rand::RngExt;
        let mut rng = rand::rng();
        (0..8)
            .map(|_| {
                let a: u32 = rng.random_range(0..10000);
                let b: u32 = rng.random_range(0..10000);
                format!("{a:04}-{b:04}")
            })
            .collect()
    }

    /// Check if a string matches the recovery code format: exactly `DDDD-DDDD`.
    pub fn is_recovery_code_format(code: &str) -> bool {
        let trimmed = code.trim();
        trimmed.len() == 9
            && trimmed.as_bytes()[4] == b'-'
            && trimmed[..4].chars().all(|c| c.is_ascii_digit())
            && trimmed[5..].chars().all(|c| c.is_ascii_digit())
    }

    /// Verify a recovery code against the stored list, consuming it on success.
    ///
    /// Returns `Ok(true)` if the code matched and was consumed, `Ok(false)` if
    /// no match, `Err` if the stored codes are malformed.
    pub fn verify_recovery_code(stored_json: &str, code: &str) -> Result<(bool, String), String> {
        let mut codes: Vec<String> = serde_json::from_str(stored_json)
            .map_err(|e| format!("Invalid recovery codes JSON: {e}"))?;
        let normalized = code.trim().to_lowercase();
        if let Some(pos) = codes.iter().position(|c| c == &normalized) {
            codes.remove(pos);
            let updated = serde_json::to_string(&codes)
                .map_err(|e| format!("Failed to serialize codes: {e}"))?;
            Ok((true, updated))
        } else {
            let unchanged = serde_json::to_string(&codes)
                .map_err(|e| format!("Failed to serialize codes: {e}"))?;
            Ok((false, unchanged))
        }
    }

    /// Check if a sender is within the TOTP grace period.
    fn is_within_totp_grace(&self, sender_id: &str, policy: &ApprovalPolicy) -> bool {
        if policy.totp_grace_period_secs == 0 {
            return false;
        }
        let grace = self.totp_grace.lock().unwrap_or_else(|e| e.into_inner());
        grace
            .get(sender_id)
            .is_some_and(|last| last.elapsed().as_secs() < policy.totp_grace_period_secs)
    }

    /// Record a successful TOTP verification for grace period tracking.
    fn record_totp_grace(&self, sender_id: &str) {
        let mut grace = self.totp_grace.lock().unwrap_or_else(|e| e.into_inner());
        grace.insert(sender_id.to_string(), Instant::now());
        // Clear failure counter on success
        let mut failures = self.totp_failures.lock().unwrap_or_else(|e| e.into_inner());
        failures.remove(sender_id);
        drop(failures);
        self.persist_totp_lockout_clear(sender_id);
    }

    /// Check if a sender is locked out due to too many TOTP failures.
    pub fn is_totp_locked_out(&self, sender_id: &str) -> bool {
        let failures = self.totp_failures.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((count, lockout_start)) = failures.get(sender_id) {
            if *count >= TOTP_MAX_FAILURES {
                // Locked out if within lockout window (measured from when threshold was reached)
                return lockout_start
                    .map(|t| t.elapsed().as_secs() < TOTP_LOCKOUT_SECS)
                    .unwrap_or(false);
            }
        }
        false
    }

    /// Record a TOTP verification failure.
    ///
    /// Returns `Ok(())` if the failure was recorded and persisted to the
    /// database, or `Err(())` if the database write failed. Callers MUST
    /// treat `Err(())` as a rejection — if we cannot persist the counter we
    /// cannot enforce the brute-force cap across restarts, so the safest
    /// course is to deny the request (fail-secure, fix for #3372 / #3584).
    pub fn record_totp_failure(&self, sender_id: &str) -> Result<(), ()> {
        let mut failures = self.totp_failures.lock().unwrap_or_else(|e| e.into_inner());
        let entry = failures.entry(sender_id.to_string()).or_insert((0, None));
        // Reset counter if lockout window expired
        if entry
            .1
            .map(|t| t.elapsed().as_secs() >= TOTP_LOCKOUT_SECS)
            .unwrap_or(false)
        {
            *entry = (0, None);
        }
        entry.0 += 1;
        if entry.0 >= TOTP_MAX_FAILURES {
            // Record lockout start time when threshold is first reached
            if entry.1.is_none() {
                entry.1 = Some(Instant::now());
            }
            warn!(
                sender_id,
                "TOTP locked out: {} consecutive failures", entry.0
            );
        }
        let (count, locked_at_instant) = *entry;
        drop(failures);
        // Persist lockout state so it survives a daemon restart.
        // If the DB write fails, return Err so callers can reject fail-secure.
        let locked_at_unix = locked_at_instant.map(|t| {
            let elapsed = t.elapsed().as_secs();
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(elapsed) as i64
        });
        self.persist_totp_lockout_save(sender_id, count, locked_at_unix)
    }

    fn persist_totp_lockout_save(
        &self,
        sender_id: &str,
        failures: u32,
        locked_at: Option<i64>,
    ) -> Result<(), ()> {
        let Some(db) = &self.audit_db else {
            // No DB configured — in-memory only, accept the failure record.
            return Ok(());
        };
        let Ok(conn) = db.lock() else {
            warn!(sender_id, "TOTP lockout DB unavailable; rejecting attempt fail-secure");
            return Err(());
        };
        let result = conn.execute(
            "INSERT INTO totp_lockout (sender_id, failures, locked_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(sender_id) DO UPDATE SET
                 failures  = excluded.failures,
                 locked_at = excluded.locked_at",
            rusqlite::params![sender_id, failures as i64, locked_at],
        );
        if let Err(e) = result {
            warn!(
                sender_id,
                "Failed to persist TOTP failure counter to DB: {e}; rejecting attempt fail-secure"
            );
            return Err(());
        }
        Ok(())
    }

    fn persist_totp_lockout_clear(&self, sender_id: &str) {
        let Some(db) = &self.audit_db else { return };
        let Ok(conn) = db.lock() else { return };
        let _ = conn.execute(
            "DELETE FROM totp_lockout WHERE sender_id = ?1",
            rusqlite::params![sender_id],
        );
    }

    // -----------------------------------------------------------------------
    // TOTP replay prevention (issue #3359)
    // -----------------------------------------------------------------------

    /// Hash a TOTP code for replay-prevention storage.
    ///
    /// Stores `sha256(code)` in hex so the raw digit string is never persisted.
    fn totp_code_hash(code: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(code.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Check whether a TOTP code has already been used within the replay window.
    ///
    /// The window is 60 seconds (two 30-second TOTP steps) to cover both the
    /// current step and the immediately preceding one.
    pub fn is_totp_code_used(&self, code: &str) -> bool {
        let Some(db) = &self.audit_db else {
            return false;
        };
        let Ok(conn) = db.lock() else { return false };
        let hash = Self::totp_code_hash(code);
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let window_start = now_unix - 60;
        conn.query_row(
            "SELECT COUNT(*) FROM totp_used_codes WHERE code_hash = ?1 AND used_at >= ?2",
            rusqlite::params![hash, window_start],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
            > 0
    }

    /// Record a successfully-verified TOTP code to prevent replay.
    ///
    /// Also prunes entries older than 120 seconds from the table to keep it small.
    pub fn record_totp_code_used(&self, code: &str) {
        let Some(db) = &self.audit_db else { return };
        let Ok(conn) = db.lock() else { return };
        let hash = Self::totp_code_hash(code);
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        // Upsert the used-code entry.
        let _ = conn.execute(
            "INSERT INTO totp_used_codes (code_hash, used_at)
             VALUES (?1, ?2)
             ON CONFLICT(code_hash) DO UPDATE SET used_at = excluded.used_at",
            rusqlite::params![hash, now_unix],
        );
        // Prune entries older than 120 seconds.
        let prune_before = now_unix - 120;
        let _ = conn.execute(
            "DELETE FROM totp_used_codes WHERE used_at < ?1",
            rusqlite::params![prune_before],
        );
    }

    /// Write an audit entry to the persistent database.
    fn audit_log_write(&self, entry: &ApprovalAuditEntry) {
        let Some(db) = &self.audit_db else { return };
        let Ok(conn) = db.lock() else { return };
        let result = conn.execute(
            "INSERT OR IGNORE INTO approval_audit (id, request_id, agent_id, tool_name, description, action_summary, risk_level, decision, decided_by, decided_at, requested_at, feedback, second_factor_used) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                entry.id,
                entry.request_id,
                entry.agent_id,
                entry.tool_name,
                entry.description,
                entry.action_summary,
                entry.risk_level,
                entry.decision,
                entry.decided_by,
                entry.decided_at,
                entry.requested_at,
                entry.feedback,
                entry.second_factor_used,
            ],
        );
        if let Err(e) = result {
            warn!("Failed to write approval audit entry: {e}");
        }
    }

    fn push_recent(
        &self,
        request: ApprovalRequest,
        decision: ApprovalDecision,
        decided_by: Option<String>,
        decided_at: chrono::DateTime<Utc>,
        second_factor_used: bool,
    ) {
        let feedback = match &decision {
            ApprovalDecision::ModifyAndRetry { feedback } => Some(feedback.clone()),
            _ => None,
        };
        let entry = ApprovalAuditEntry {
            id: Uuid::new_v4().to_string(),
            request_id: request.id.to_string(),
            agent_id: request.agent_id.clone(),
            tool_name: request.tool_name.clone(),
            description: request.description.clone(),
            action_summary: request.action_summary.clone(),
            risk_level: serde_json::to_string(&request.risk_level)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string(),
            decision: decision.as_str().to_string(),
            decided_by: decided_by.clone(),
            decided_at: decided_at.to_rfc3339(),
            requested_at: request.requested_at.to_rfc3339(),
            feedback,
            second_factor_used,
        };
        self.audit_log_write(&entry);

        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        recent.push_front(ApprovalRecord {
            request,
            decision,
            decided_at,
            decided_by,
        });
        while recent.len() > MAX_RECENT_APPROVALS {
            recent.pop_back();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExpiryOutcome {
    Escalate,
    Resolve(ApprovalDecision),
}

fn effective_timeout_secs(request: &ApprovalRequest, fallback: &TimeoutFallback) -> u64 {
    match fallback {
        TimeoutFallback::Escalate { extra_timeout_secs } => {
            request.timeout_secs + (*extra_timeout_secs * request.escalation_count as u64)
        }
        _ => request.timeout_secs,
    }
}

fn timeout_decision(request: &ApprovalRequest, fallback: &TimeoutFallback) -> ExpiryOutcome {
    match fallback {
        TimeoutFallback::Escalate { .. } if request.escalation_count < MAX_ESCALATIONS => {
            ExpiryOutcome::Escalate
        }
        TimeoutFallback::Skip => ExpiryOutcome::Resolve(ApprovalDecision::Skipped),
        _ => ExpiryOutcome::Resolve(ApprovalDecision::TimedOut),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::approval::ApprovalPolicy;
    use std::sync::Arc;

    fn default_manager() -> ApprovalManager {
        ApprovalManager::new(ApprovalPolicy::default())
    }

    fn make_request(agent_id: &str, tool_name: &str, timeout_secs: u64) -> ApprovalRequest {
        ApprovalRequest {
            id: Uuid::new_v4(),
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            description: "test operation".to_string(),
            action_summary: "test action".to_string(),
            risk_level: RiskLevel::High,
            requested_at: Utc::now(),
            timeout_secs,
            sender_id: None,
            channel: None,
            route_to: Vec::new(),
            escalation_count: 0,
            session_id: None,
        }
    }

    // -----------------------------------------------------------------------
    // requires_approval
    // -----------------------------------------------------------------------

    #[test]
    fn test_requires_approval_default() {
        let mgr = default_manager();
        assert!(mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("file_read"));
    }

    #[test]
    fn test_requires_approval_custom_policy() {
        let policy = ApprovalPolicy {
            require_approval: vec!["file_write".to_string(), "file_delete".to_string()],
            timeout_secs: 30,
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);
        assert!(mgr.requires_approval("file_write"));
        assert!(mgr.requires_approval("file_delete"));
        assert!(!mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("file_read"));
    }

    // -----------------------------------------------------------------------
    // classify_risk
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_risk() {
        assert_eq!(
            ApprovalManager::classify_risk("shell_exec"),
            RiskLevel::Critical
        );
        assert_eq!(
            ApprovalManager::classify_risk("file_write"),
            RiskLevel::High
        );
        assert_eq!(
            ApprovalManager::classify_risk("file_delete"),
            RiskLevel::High
        );
        assert_eq!(
            ApprovalManager::classify_risk("apply_patch"),
            RiskLevel::High
        );
        assert_eq!(
            ApprovalManager::classify_risk("web_fetch"),
            RiskLevel::Medium
        );
        assert_eq!(
            ApprovalManager::classify_risk("browser_navigate"),
            RiskLevel::Medium
        );
        assert_eq!(ApprovalManager::classify_risk("file_read"), RiskLevel::Low);
        assert_eq!(
            ApprovalManager::classify_risk("unknown_tool"),
            RiskLevel::Low
        );
    }

    // -----------------------------------------------------------------------
    // resolve nonexistent
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_nonexistent() {
        let mgr = default_manager();
        let result = mgr.resolve(
            Uuid::new_v4(),
            ApprovalDecision::Approved,
            None,
            false,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found or expired"));
    }

    // -----------------------------------------------------------------------
    // list_pending empty
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_pending_empty() {
        let mgr = default_manager();
        assert!(mgr.list_pending().is_empty());
        assert!(mgr.list_recent(10).is_empty());
    }

    // -----------------------------------------------------------------------
    // update_policy
    // -----------------------------------------------------------------------

    #[test]
    fn test_update_policy() {
        let mgr = default_manager();
        assert!(mgr.requires_approval("shell_exec"));
        // file_write is now in the default require_approval list
        assert!(mgr.requires_approval("file_write"));

        let new_policy = ApprovalPolicy {
            require_approval: vec!["file_write".to_string()],
            timeout_secs: 120,
            auto_approve_autonomous: true,
            ..Default::default()
        };
        mgr.update_policy(new_policy);

        assert!(!mgr.requires_approval("shell_exec"));
        assert!(mgr.requires_approval("file_write"));

        let policy = mgr.policy();
        assert_eq!(policy.timeout_secs, 120);
        assert!(policy.auto_approve_autonomous);
    }

    // -----------------------------------------------------------------------
    // pending_count
    // -----------------------------------------------------------------------

    #[test]
    fn test_pending_count() {
        let mgr = default_manager();
        assert_eq!(mgr.pending_count(), 0);
    }

    // -----------------------------------------------------------------------
    // request_approval — timeout
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_request_approval_timeout() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 10);
        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::TimedOut);
        // After timeout, pending map should be cleaned up
        assert_eq!(mgr.pending_count(), 0);
        let recent = mgr.list_recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].decision, ApprovalDecision::TimedOut);
        assert_eq!(recent[0].request.tool_name, "shell_exec");
    }

    // -----------------------------------------------------------------------
    // request_approval — approve
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_request_approval_approve() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            // Small delay to let the request register
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let result = mgr2.resolve(
                request_id,
                ApprovalDecision::Approved,
                Some("admin".to_string()),
                false,
                None,
            );
            assert!(result.is_ok());
            let (resp, _deferred) = result.unwrap();
            assert_eq!(resp.decision, ApprovalDecision::Approved);
            assert_eq!(resp.decided_by, Some("admin".to_string()));
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Approved);
        let recent = mgr.list_recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].decision, ApprovalDecision::Approved);
        assert_eq!(recent[0].decided_by.as_deref(), Some("admin"));
    }

    // -----------------------------------------------------------------------
    // request_approval — deny
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_request_approval_deny() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let result = mgr2.resolve(request_id, ApprovalDecision::Denied, None, false, None);
            assert!(result.is_ok());
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Denied);
        let recent = mgr.list_recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].decision, ApprovalDecision::Denied);
    }

    // -----------------------------------------------------------------------
    // max pending per agent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_max_pending_per_agent() {
        let mgr = Arc::new(default_manager());

        // Fill up 5 pending requests for agent-1 (they will all be waiting)
        let mut ids = Vec::new();
        for _ in 0..MAX_PENDING_PER_AGENT {
            let req = make_request("agent-1", "shell_exec", 300);
            ids.push(req.id);
            let mgr_clone = Arc::clone(&mgr);
            tokio::spawn(async move {
                mgr_clone.request_approval(req).await;
            });
        }

        // Give spawned tasks time to register
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(mgr.pending_count(), MAX_PENDING_PER_AGENT);

        // 6th request for the same agent should be immediately denied
        let req6 = make_request("agent-1", "shell_exec", 300);
        let decision = mgr.request_approval(req6).await;
        assert_eq!(decision, ApprovalDecision::Denied);

        // A different agent should still be able to submit
        let req_other = make_request("agent-2", "shell_exec", 300);
        let other_id = req_other.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            mgr2.request_approval(req_other).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(mgr.pending_count(), MAX_PENDING_PER_AGENT + 1);

        // Cleanup: resolve all pending to avoid hanging tasks
        for id in &ids {
            let _ = mgr.resolve(*id, ApprovalDecision::Denied, None, false, None);
        }
        let _ = mgr.resolve(other_id, ApprovalDecision::Denied, None, false, None);
    }

    // -----------------------------------------------------------------------
    // get_pending
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_pending_not_found() {
        let mgr = default_manager();
        assert!(mgr.get_pending(Uuid::new_v4()).is_none());
    }

    #[tokio::test]
    async fn test_get_pending_found() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 300);
        let id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            mgr2.request_approval(req).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let found = mgr.get_pending(id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, id);

        // Cleanup
        let _ = mgr.resolve(id, ApprovalDecision::Denied, None, false, None);
    }

    // -----------------------------------------------------------------------
    // policy defaults
    // -----------------------------------------------------------------------

    #[test]
    fn test_policy_defaults() {
        let mgr = default_manager();
        let policy = mgr.policy();
        assert_eq!(
            policy.require_approval,
            vec![
                "shell_exec",
                "file_write",
                "file_delete",
                "apply_patch",
                "skill_evolve_*",
            ]
        );
        assert_eq!(policy.timeout_secs, 60);
        assert!(!policy.auto_approve_autonomous);
    }

    // -----------------------------------------------------------------------
    // requires_approval_with_context
    // -----------------------------------------------------------------------

    #[test]
    fn test_context_trusted_sender_bypasses_approval() {
        let policy = ApprovalPolicy {
            require_approval: vec!["shell_exec".to_string()],
            trusted_senders: vec!["admin_123".to_string()],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        // Trusted sender should bypass even for shell_exec
        assert!(!mgr.requires_approval_with_context("shell_exec", Some("admin_123"), None));

        // Untrusted sender still requires approval
        assert!(mgr.requires_approval_with_context("shell_exec", Some("random_user"), None));

        // No sender context falls back to default
        assert!(mgr.requires_approval_with_context("shell_exec", None, None));
    }

    #[test]
    fn test_context_channel_rule_denies_tool() {
        let policy = ApprovalPolicy {
            require_approval: vec!["shell_exec".to_string()],
            channel_rules: vec![librefang_types::approval::ChannelToolRule {
                channel: "telegram".to_string(),
                allowed_tools: vec![],
                denied_tools: vec!["file_write".to_string()],
            }],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        // file_write is not in require_approval, but telegram channel denies it
        assert!(mgr.is_tool_denied_with_context("file_write", None, Some("telegram")));
        assert!(mgr.requires_approval_with_context("file_write", None, Some("telegram")));

        // file_write from other channels is not gated
        assert!(!mgr.is_tool_denied_with_context("file_write", None, Some("discord")));
        assert!(!mgr.requires_approval_with_context("file_write", None, Some("discord")));
    }

    #[test]
    fn test_context_channel_rule_allows_tool() {
        let policy = ApprovalPolicy {
            require_approval: vec!["shell_exec".to_string()],
            channel_rules: vec![librefang_types::approval::ChannelToolRule {
                channel: "admin_cli".to_string(),
                allowed_tools: vec!["shell_exec".to_string()],
                denied_tools: vec![],
            }],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        // shell_exec from admin_cli channel is explicitly allowed — bypass approval
        assert!(!mgr.requires_approval_with_context("shell_exec", None, Some("admin_cli")));

        // shell_exec from other channels still requires approval
        assert!(mgr.requires_approval_with_context("shell_exec", None, Some("telegram")));
    }

    #[test]
    fn test_context_trusted_sender_overrides_channel_deny() {
        let policy = ApprovalPolicy {
            require_approval: vec!["shell_exec".to_string()],
            trusted_senders: vec!["admin_123".to_string()],
            channel_rules: vec![librefang_types::approval::ChannelToolRule {
                channel: "telegram".to_string(),
                allowed_tools: vec![],
                denied_tools: vec!["shell_exec".to_string()],
            }],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        // Trusted sender bypasses even channel deny rules
        assert!(!mgr.is_tool_denied_with_context(
            "shell_exec",
            Some("admin_123"),
            Some("telegram")
        ));
        assert!(!mgr.requires_approval_with_context(
            "shell_exec",
            Some("admin_123"),
            Some("telegram")
        ));

        // Untrusted sender from telegram is denied
        assert!(mgr.is_tool_denied_with_context(
            "shell_exec",
            Some("random_user"),
            Some("telegram")
        ));
        assert!(mgr.requires_approval_with_context(
            "shell_exec",
            Some("random_user"),
            Some("telegram")
        ));
    }

    #[test]
    fn test_context_no_context_falls_back_to_default() {
        let mgr = default_manager();

        // No sender/channel context: behaves like requires_approval()
        assert!(mgr.requires_approval_with_context("shell_exec", None, None));
        assert!(!mgr.requires_approval_with_context("file_read", None, None));
    }

    // -----------------------------------------------------------------------
    // Wildcard support in require_approval
    // -----------------------------------------------------------------------

    #[test]
    fn test_requires_approval_wildcard_prefix() {
        let policy = ApprovalPolicy {
            require_approval: vec!["file_*".to_string()],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        assert!(mgr.requires_approval("file_read"));
        assert!(mgr.requires_approval("file_write"));
        assert!(mgr.requires_approval("file_delete"));
        assert!(!mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("web_fetch"));
    }

    #[test]
    fn test_requires_approval_wildcard_star_all() {
        let policy = ApprovalPolicy {
            require_approval: vec!["*".to_string()],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        assert!(mgr.requires_approval("file_read"));
        assert!(mgr.requires_approval("shell_exec"));
        assert!(mgr.requires_approval("anything"));
    }

    #[test]
    fn test_requires_approval_wildcard_suffix() {
        let policy = ApprovalPolicy {
            require_approval: vec!["*_exec".to_string()],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        assert!(mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("shell_read"));
        assert!(!mgr.requires_approval("file_write"));
    }

    #[test]
    fn test_requires_approval_with_context_wildcard() {
        let policy = ApprovalPolicy {
            require_approval: vec!["file_*".to_string()],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        assert!(mgr.requires_approval_with_context("file_write", None, None));
        assert!(mgr.requires_approval_with_context("file_delete", None, None));
        assert!(!mgr.requires_approval_with_context("shell_exec", None, None));
    }

    #[test]
    fn test_requires_approval_mixed_wildcard_and_exact() {
        let policy = ApprovalPolicy {
            require_approval: vec!["shell_exec".to_string(), "file_*".to_string()],
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        assert!(mgr.requires_approval("shell_exec"));
        assert!(mgr.requires_approval("file_read"));
        assert!(mgr.requires_approval("file_write"));
        assert!(!mgr.requires_approval("web_fetch"));
    }

    // -----------------------------------------------------------------------
    // submit_request (non-blocking approval)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_submit_request_returns_uuid_immediately() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 300);
        let deferred = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };

        let result = mgr.submit_request(req, deferred);
        assert!(result.is_ok());
        let id = result.unwrap();
        assert!(!id.is_nil());

        // Cleanup
        let _ = mgr.resolve(id, ApprovalDecision::Denied, None, false, None);
    }

    #[tokio::test]
    async fn test_submit_request_stores_deferred_payload() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 300);
        let deferred = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: Some(vec!["shell_exec".to_string()]),
            allowed_env_vars: Some(vec!["OPENAI_API_KEY".to_string()]),
            exec_policy: Some(librefang_types::config::ExecPolicy {
                mode: librefang_types::config::ExecSecurityMode::Full,
                ..Default::default()
            }),
            sender_id: Some("user-123".to_string()),
            channel: Some("telegram".to_string()),
            workspace_root: Some(std::path::PathBuf::from("/tmp")),
            force_human: false,
        };

        let id = mgr.submit_request(req, deferred.clone()).unwrap();

        // Verify deferred is stored by resolving and checking the returned deferred
        let (response, returned_deferred) = mgr
            .resolve(id, ApprovalDecision::Denied, None, false, None)
            .unwrap();
        assert_eq!(response.decision, ApprovalDecision::Denied);
        assert!(returned_deferred.is_some());
        let stored = returned_deferred.unwrap();
        assert_eq!(stored.agent_id, "agent-1");
        assert_eq!(stored.tool_use_id, "tool-1");
        assert_eq!(stored.tool_name, "shell_exec");
        assert_eq!(
            stored.allowed_env_vars,
            Some(vec!["OPENAI_API_KEY".to_string()])
        );
        assert_eq!(
            stored.exec_policy.as_ref().map(|p| p.mode),
            Some(librefang_types::config::ExecSecurityMode::Full)
        );
        assert_eq!(stored.sender_id, Some("user-123".to_string()));
        assert_eq!(stored.channel, Some("telegram".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_returns_deferred_atomically() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "shell_exec", 300);
        let deferred = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };

        let id = mgr.submit_request(req, deferred.clone()).unwrap();

        // Resolve and verify atomic return
        let (response, returned_deferred) = mgr
            .resolve(
                id,
                ApprovalDecision::Approved,
                Some("admin".to_string()),
                false,
                None,
            )
            .unwrap();
        assert_eq!(response.decision, ApprovalDecision::Approved);
        assert!(returned_deferred.is_some());
        assert_eq!(returned_deferred.unwrap().agent_id, "agent-1");
    }

    #[test]
    fn test_expire_pending_requests_skip_fallback() {
        let policy = ApprovalPolicy {
            timeout_fallback: TimeoutFallback::Skip,
            ..ApprovalPolicy::default()
        };
        let mgr = ApprovalManager::new(policy);
        let req = make_request("agent-1", "shell_exec", 1);
        let id = req.id;
        let deferred = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };
        mgr.submit_request(req, deferred).unwrap();
        mgr.pending.get_mut(&id).unwrap().submitted_at = Utc::now() - chrono::Duration::seconds(5);

        let (escalated, expired) = mgr.expire_pending_requests();
        assert!(escalated.is_empty());
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].1, ApprovalDecision::Skipped);
        assert_eq!(mgr.pending_count(), 0);
    }

    #[test]
    fn test_expire_pending_requests_escalates_then_times_out() {
        let policy = ApprovalPolicy {
            timeout_fallback: TimeoutFallback::Escalate {
                extra_timeout_secs: 5,
            },
            ..ApprovalPolicy::default()
        };
        let mgr = ApprovalManager::new(policy);
        let req = make_request("agent-1", "shell_exec", 1);
        let id = req.id;
        let deferred = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };
        mgr.submit_request(req, deferred).unwrap();

        for expected in 1..=MAX_ESCALATIONS {
            mgr.pending.get_mut(&id).unwrap().submitted_at =
                Utc::now() - chrono::Duration::seconds(60);
            let (escalated, expired) = mgr.expire_pending_requests();
            assert_eq!(escalated.len(), 1);
            assert_eq!(escalated[0].request_id, id);
            assert!(expired.is_empty());
            let pending = mgr.get_pending(id).unwrap();
            assert_eq!(pending.escalation_count, expected);
        }

        mgr.pending.get_mut(&id).unwrap().submitted_at = Utc::now() - chrono::Duration::seconds(60);
        let (escalated, expired) = mgr.expire_pending_requests();
        assert!(escalated.is_empty());
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].1, ApprovalDecision::TimedOut);
        assert_eq!(mgr.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_submit_request_duplicate_guard() {
        let mgr = Arc::new(default_manager());
        let req1 = make_request("agent-1", "shell_exec", 300);
        let deferred1 = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };

        let id1 = mgr.submit_request(req1, deferred1).unwrap();

        // Try to submit duplicate tool_use_id
        let req2 = make_request("agent-1", "shell_exec", 300);
        let deferred2 = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };

        let result = mgr.submit_request(req2, deferred2);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Duplicate"));

        // Cleanup
        let _ = mgr.resolve(id1, ApprovalDecision::Denied, None, false, None);
    }

    #[tokio::test]
    async fn test_submit_request_allows_identical_input_with_distinct_tool_use_ids() {
        let mgr = Arc::new(default_manager());
        let req1 = make_request("agent-1", "shell_exec", 300);
        let deferred1 = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-1".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };
        let id1 = mgr.submit_request(req1, deferred1).unwrap();

        let req2 = make_request("agent-1", "shell_exec", 300);
        let deferred2 = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-2".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };

        let id2 = mgr.submit_request(req2, deferred2).unwrap();

        let _ = mgr.resolve(id1, ApprovalDecision::Denied, None, false, None);
        let _ = mgr.resolve(id2, ApprovalDecision::Denied, None, false, None);
    }

    #[tokio::test]
    async fn test_per_agent_limit_enforced() {
        let mgr = Arc::new(default_manager());

        // Submit MAX_PENDING_PER_AGENT requests for agent-1
        let mut ids = Vec::new();
        for i in 0..MAX_PENDING_PER_AGENT {
            let req = make_request("agent-1", "shell_exec", 300);
            let deferred = DeferredToolExecution {
                agent_id: "agent-1".to_string(),
                tool_use_id: format!("tool-{i}"),
                tool_name: "shell_exec".to_string(),
                input: serde_json::json!({"cmd": format!("ls {i}")}),
                allowed_tools: None,
                allowed_env_vars: None,
                exec_policy: None,
                sender_id: None,
                channel: None,
                workspace_root: None,
                force_human: false,
            };
            let id = mgr.submit_request(req, deferred).unwrap();
            ids.push(id);
        }

        // Try to submit one more for same agent
        let req = make_request("agent-1", "shell_exec", 300);
        let deferred = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "tool-extra".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls extra"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };
        let result = mgr.submit_request(req, deferred);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Too many pending"));

        // Different agent should still be able to submit
        let req = make_request("agent-2", "shell_exec", 300);
        let deferred = DeferredToolExecution {
            agent_id: "agent-2".to_string(),
            tool_use_id: "tool-other".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls other"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };
        let result = mgr.submit_request(req, deferred);
        assert!(result.is_ok());

        // Cleanup
        for id in ids {
            let _ = mgr.resolve(id, ApprovalDecision::Denied, None, false, None);
        }
        let _ = mgr.resolve(result.unwrap(), ApprovalDecision::Denied, None, false, None);
    }

    #[test]
    fn test_expire_pending_requests_respects_deny_fallback() {
        let mgr = ApprovalManager::new(ApprovalPolicy::default());
        let req = make_request("agent-1", "shell_exec", 60);
        let request_id = req.id;
        mgr.pending.insert(
            request_id,
            PendingRequest {
                request: req,
                sender: None,
                deferred: Some(DeferredToolExecution {
                    agent_id: "agent-1".to_string(),
                    tool_use_id: "tool-1".to_string(),
                    tool_name: "shell_exec".to_string(),
                    input: serde_json::json!({"cmd": "ls"}),
                    allowed_tools: None,
                    allowed_env_vars: None,
                    exec_policy: None,
                    sender_id: None,
                    channel: None,
                    workspace_root: None,
                    force_human: false,
                }),
                submitted_at: Utc::now() - chrono::Duration::seconds(120),
            },
        );

        let (escalated, expired) = mgr.expire_pending_requests();

        assert!(escalated.is_empty());
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, request_id);
        assert_eq!(expired[0].1, ApprovalDecision::TimedOut);
        assert!(mgr.get_pending(request_id).is_none());
    }

    #[test]
    fn test_expire_pending_requests_escalates_only_under_escalate_fallback() {
        let policy = ApprovalPolicy {
            timeout_fallback: TimeoutFallback::Escalate {
                extra_timeout_secs: 30,
            },
            ..ApprovalPolicy::default()
        };
        let mgr = ApprovalManager::new(policy);
        let req = make_request("agent-1", "shell_exec", 60);
        let request_id = req.id;
        mgr.pending.insert(
            request_id,
            PendingRequest {
                request: req,
                sender: None,
                deferred: Some(DeferredToolExecution {
                    agent_id: "agent-1".to_string(),
                    tool_use_id: "tool-1".to_string(),
                    tool_name: "shell_exec".to_string(),
                    input: serde_json::json!({"cmd": "ls"}),
                    allowed_tools: None,
                    allowed_env_vars: None,
                    exec_policy: None,
                    sender_id: None,
                    channel: None,
                    workspace_root: None,
                    force_human: false,
                }),
                submitted_at: Utc::now() - chrono::Duration::seconds(120),
            },
        );

        let (escalated, expired) = mgr.expire_pending_requests();

        assert_eq!(escalated.len(), 1);
        assert_eq!(escalated[0].request_id, request_id);
        assert!(expired.is_empty());
        let pending = mgr
            .get_pending(request_id)
            .expect("request should remain pending");
        assert_eq!(pending.escalation_count, 1);
    }

    // -----------------------------------------------------------------------
    // TOTP
    // -----------------------------------------------------------------------

    use librefang_types::approval::SecondFactor;

    #[test]
    fn test_requires_totp_default_is_false() {
        let mgr = default_manager();
        assert!(!mgr.requires_totp());
    }

    #[test]
    fn test_requires_totp_when_enabled() {
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);
        assert!(mgr.requires_totp());
    }

    #[tokio::test]
    async fn test_resolve_requires_totp_when_enabled() {
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            ..Default::default()
        };
        let mgr = Arc::new(ApprovalManager::new(policy));
        let req = make_request("agent-1", "shell_exec", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            // Without totp_verified=true, resolve should fail
            let result = mgr2.resolve(request_id, ApprovalDecision::Approved, None, false, None);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("TOTP"));

            // With totp_verified=true, resolve should succeed
            let result = mgr2.resolve(
                request_id,
                ApprovalDecision::Approved,
                None,
                true,
                Some("admin"),
            );
            assert!(result.is_ok());
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Approved);
    }

    #[tokio::test]
    async fn test_totp_grace_period() {
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            totp_grace_period_secs: 300,
            ..Default::default()
        };
        let mgr = Arc::new(ApprovalManager::new(policy));

        // First request: need totp_verified=true
        let req1 = make_request("agent-1", "shell_exec", 60);
        let id1 = req1.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let result = mgr2.resolve(id1, ApprovalDecision::Approved, None, true, Some("admin"));
            assert!(result.is_ok());
        });
        mgr.request_approval(req1).await;

        // Second request: grace period should allow without totp_verified
        let req2 = make_request("agent-1", "shell_exec", 60);
        let id2 = req2.id;
        let mgr3 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let result = mgr3.resolve(
                id2,
                ApprovalDecision::Approved,
                None,
                false,
                Some("admin"), // Same user_id → grace applies
            );
            assert!(result.is_ok());
        });
        let decision = mgr.request_approval(req2).await;
        assert_eq!(decision, ApprovalDecision::Approved);
    }

    #[test]
    fn test_totp_grace_zero_means_always_require() {
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            totp_grace_period_secs: 0,
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy.clone());
        // Even after recording grace, zero period means no grace
        mgr.record_totp_grace("admin");
        assert!(!mgr.is_within_totp_grace("admin", &policy));
    }

    #[test]
    fn test_reject_does_not_require_totp() {
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);
        // Reject should work without TOTP (request won't exist, but the TOTP
        // gate should not block it — error should be "not found", not "TOTP")
        let result = mgr.resolve(Uuid::new_v4(), ApprovalDecision::Denied, None, false, None);
        assert!(result.is_err());
        assert!(!result.unwrap_err().contains("TOTP"));
    }

    #[test]
    fn test_verify_totp_code_invalid_secret() {
        let result = ApprovalManager::verify_totp_code("not-valid-base32!!!", "123456");
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_totp_secret() {
        let (secret, uri, qr) =
            ApprovalManager::generate_totp_secret("LibreFang", "admin").unwrap();
        assert!(!secret.is_empty());
        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains("LibreFang"));
        assert!(!qr.is_empty()); // base64-encoded PNG
    }

    // -----------------------------------------------------------------------
    // End-to-end TOTP flow
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_e2e_totp_setup_verify_approve_grace() {
        // 1. Generate secret
        let (secret, uri, _qr) =
            ApprovalManager::generate_totp_secret("LibreFang", "test").unwrap();
        assert!(uri.contains("LibreFang"));

        // 2. Generate a valid code from the secret
        let totp_secret = Secret::Encoded(secret.clone());
        let raw = totp_secret.to_bytes().unwrap();
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            raw,
            Some("LibreFang".to_string()),
            "test".to_string(),
        )
        .unwrap();
        let valid_code = totp.generate_current().unwrap();

        // 3. Verify the code against our verify function
        assert!(ApprovalManager::verify_totp_code(&secret, &valid_code).unwrap());
        assert!(!ApprovalManager::verify_totp_code(&secret, "000000").unwrap());

        // 4. Full approval flow with TOTP
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            totp_grace_period_secs: 300,
            ..Default::default()
        };
        let mgr = Arc::new(ApprovalManager::new(policy));

        let req = make_request("agent-e2e", "shell_exec", 60);
        let id = req.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            // Without TOTP → rejected
            let err = mgr2
                .resolve(id, ApprovalDecision::Approved, None, false, Some("user1"))
                .unwrap_err();
            assert!(err.contains("TOTP"));

            // With TOTP → approved
            let ok = mgr2.resolve(id, ApprovalDecision::Approved, None, true, Some("user1"));
            assert!(ok.is_ok());
        });
        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Approved);

        // 5. Grace period — second approval without TOTP should work
        let req2 = make_request("agent-e2e", "shell_exec", 60);
        let id2 = req2.id;
        let mgr3 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let ok = mgr3.resolve(
                id2,
                ApprovalDecision::Approved,
                None,
                false,
                Some("user1"), // same user → grace applies
            );
            assert!(ok.is_ok());
        });
        let d2 = mgr.request_approval(req2).await;
        assert_eq!(d2, ApprovalDecision::Approved);

        // 6. Different user has no grace
        let req3 = make_request("agent-e2e", "shell_exec", 60);
        let id3 = req3.id;
        let mgr4 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let err = mgr4
                .resolve(id3, ApprovalDecision::Approved, None, false, Some("user2"))
                .unwrap_err();
            assert!(err.contains("TOTP"));
            // Clean up
            let _ = mgr4.resolve(id3, ApprovalDecision::Denied, None, false, None);
        });
        mgr.request_approval(req3).await;
    }

    #[test]
    fn test_recovery_code_generate_and_verify() {
        let codes = ApprovalManager::generate_recovery_codes();
        assert_eq!(codes.len(), 8);
        // Each code is xxxx-xxxx format
        for code in &codes {
            assert_eq!(code.len(), 9);
            assert!(code.contains('-'));
        }

        // Verify and consume
        let json = serde_json::to_string(&codes).unwrap();
        let (matched, remaining_json) =
            ApprovalManager::verify_recovery_code(&json, &codes[0]).unwrap();
        assert!(matched);
        let remaining: Vec<String> = serde_json::from_str(&remaining_json).unwrap();
        assert_eq!(remaining.len(), 7);
        assert!(!remaining.contains(&codes[0]));

        // Same code should not match again
        let (matched2, _) =
            ApprovalManager::verify_recovery_code(&remaining_json, &codes[0]).unwrap();
        assert!(!matched2);
    }

    #[test]
    fn test_totp_rate_limiting() {
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            ..Default::default()
        };
        let mgr = ApprovalManager::new(policy);

        // Not locked out initially
        assert!(!mgr.is_totp_locked_out("user1"));

        // Record failures up to threshold
        for _ in 0..5 {
            mgr.record_totp_failure("user1");
        }
        assert!(mgr.is_totp_locked_out("user1"));

        // Different user is not locked out
        assert!(!mgr.is_totp_locked_out("user2"));
    }

    #[test]
    fn test_per_tool_totp() {
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            totp_tools: vec!["shell_exec".to_string()],
            ..Default::default()
        };
        // shell_exec needs TOTP
        assert!(policy.tool_requires_totp("shell_exec"));
        // file_write does not
        assert!(!policy.tool_requires_totp("file_write"));

        // Empty totp_tools → all tools need TOTP
        let policy2 = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            ..Default::default()
        };
        assert!(policy2.tool_requires_totp("shell_exec"));
        assert!(policy2.tool_requires_totp("file_write"));
        assert!(policy2.tool_requires_totp("anything"));
    }

    // -----------------------------------------------------------------------
    // TOTP replay prevention (#3359)
    // -----------------------------------------------------------------------

    fn make_manager_with_db() -> ApprovalManager {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        librefang_memory::migration::run_migrations(&conn).unwrap();
        let conn = Arc::new(StdMutex::new(conn));
        ApprovalManager::new_with_db(ApprovalPolicy::default(), conn)
    }

    #[test]
    fn test_totp_replay_prevention_code_not_used_initially() {
        let mgr = make_manager_with_db();
        // A fresh code should not be marked as used.
        assert!(!mgr.is_totp_code_used("123456"));
    }

    #[test]
    fn test_totp_replay_prevention_code_rejected_after_use() {
        let mgr = make_manager_with_db();
        mgr.record_totp_code_used("123456");
        // The same code must now be detected as already used.
        assert!(mgr.is_totp_code_used("123456"));
    }

    #[test]
    fn test_totp_replay_prevention_different_code_not_blocked() {
        let mgr = make_manager_with_db();
        mgr.record_totp_code_used("123456");
        // A different code must not be blocked.
        assert!(!mgr.is_totp_code_used("654321"));
    }

    #[test]
    fn test_totp_code_hash_does_not_store_plaintext() {
        // The hash of "123456" must not equal "123456".
        let hash = ApprovalManager::totp_code_hash("123456");
        assert_ne!(hash, "123456");
        // It must be a 64-character hex string (SHA-256 output).
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // -----------------------------------------------------------------------
    // Per-session queue methods (Hermes-Agent parity)
    // -----------------------------------------------------------------------

    fn make_session_request(agent_id: &str, session_id: &str) -> ApprovalRequest {
        let mut req = make_request(agent_id, "shell_exec", 300);
        req.session_id = Some(session_id.to_string());
        req
    }

    #[test]
    fn test_list_pending_for_session_empty() {
        let mgr = default_manager();
        assert!(mgr.list_pending_for_session("sess-1").is_empty());
        assert!(!mgr.has_pending_for_session("sess-1"));
    }

    #[tokio::test]
    async fn test_list_pending_for_session_scoped() {
        let mgr = Arc::new(default_manager());

        // Submit two requests for sess-A and one for sess-B
        let req_a1 = make_session_request("agent-1", "sess-A");
        let req_a2 = make_session_request("agent-1", "sess-A");
        let req_b1 = make_session_request("agent-2", "sess-B");

        let id_a1 = req_a1.id;
        let id_a2 = req_a2.id;
        let id_b1 = req_b1.id;

        for req in [req_a1, req_a2, req_b1] {
            let m = Arc::clone(&mgr);
            tokio::spawn(async move { m.request_approval(req).await });
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let a_pending = mgr.list_pending_for_session("sess-A");
        assert_eq!(a_pending.len(), 2);
        assert!(a_pending
            .iter()
            .all(|r| r.session_id.as_deref() == Some("sess-A")));

        let b_pending = mgr.list_pending_for_session("sess-B");
        assert_eq!(b_pending.len(), 1);
        assert!(mgr.has_pending_for_session("sess-A"));
        assert!(mgr.has_pending_for_session("sess-B"));
        assert!(!mgr.has_pending_for_session("sess-C"));

        // Cleanup
        for id in [id_a1, id_a2, id_b1] {
            let _ = mgr.resolve(id, ApprovalDecision::Denied, None, false, None);
        }
    }

    #[tokio::test]
    async fn test_resolve_all_for_session_approves_only_matching() {
        let mgr = Arc::new(default_manager());

        let req_a = make_session_request("agent-1", "sess-target");
        let req_b = make_session_request("agent-2", "sess-other");
        let id_a = req_a.id;
        let id_b = req_b.id;

        for req in [req_a, req_b] {
            let m = Arc::clone(&mgr);
            tokio::spawn(async move { m.request_approval(req).await });
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        // Resolve only the target session
        let resolved = mgr.resolve_all_for_session(
            "sess-target",
            ApprovalDecision::Approved,
            Some("test".to_string()),
        );
        assert_eq!(resolved, 1);

        // sess-other should still be pending
        assert!(mgr.has_pending_for_session("sess-other"));
        assert!(!mgr.has_pending_for_session("sess-target"));

        // Cleanup remaining
        let _ = mgr.resolve(id_b, ApprovalDecision::Denied, None, false, None);
        let _ = id_a; // already resolved above
    }

    #[tokio::test]
    async fn test_resolve_all_for_session_returns_zero_when_none_pending() {
        let mgr = default_manager();
        let resolved =
            mgr.resolve_all_for_session("nonexistent-session", ApprovalDecision::Approved, None);
        assert_eq!(resolved, 0);
    }

    #[tokio::test]
    async fn test_resolve_all_for_session_unblocks_waiting_agents() {
        // Verify that agents blocking on request_approval() are properly
        // unblocked when resolve_all_for_session resolves their request.
        let mgr = Arc::new(default_manager());

        let req = make_session_request("agent-1", "sess-unblock");
        let id = req.id;

        let mgr2 = Arc::clone(&mgr);
        let handle = tokio::spawn(async move { mgr2.request_approval(req).await });

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let resolved = mgr.resolve_all_for_session(
            "sess-unblock",
            ApprovalDecision::Approved,
            Some("batch".to_string()),
        );
        assert_eq!(resolved, 1);

        let decision = handle.await.unwrap();
        assert_eq!(decision, ApprovalDecision::Approved);

        let _ = id; // consumed by resolve_all
                    // Verify appears in recent
        let recent = mgr.list_recent(10);
        assert!(
            recent.iter().any(|r| r.request.id == id),
            "resolved approval should appear in recent list"
        );
    }

    // -----------------------------------------------------------------------
    // Additional session resolution tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_cross_agent_same_session_resolved() {
        // Requests from different agents in the same session are all resolved.
        let mgr = Arc::new(default_manager());

        let req_a1 = make_session_request("agent-1", "sess-shared");
        let req_a2 = make_session_request("agent-2", "sess-shared");
        let id_a1 = req_a1.id;
        let id_a2 = req_a2.id;

        for req in [req_a1, req_a2] {
            let m = Arc::clone(&mgr);
            tokio::spawn(async move { m.request_approval(req).await });
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let resolved = mgr.resolve_all_for_session(
            "sess-shared",
            ApprovalDecision::Approved,
            Some("batch".to_string()),
        );
        assert_eq!(resolved, 2);

        // Both agents should be unblocked.
        let recent = mgr.list_recent(10);
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().any(|r| r.request.id == id_a1));
        assert!(recent.iter().any(|r| r.request.id == id_a2));
    }

    #[tokio::test]
    async fn test_resolve_all_for_session_totp_blocks_approval() {
        // When TOTP is required, resolve_all_for_session skips the item
        // (returns 0 for that item) and the item remains pending.
        let policy = ApprovalPolicy {
            second_factor: SecondFactor::Totp,
            ..Default::default()
        };
        let mgr = Arc::new(ApprovalManager::new(policy));

        let req = make_session_request("agent-1", "sess-totp");
        let id = req.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move { mgr2.request_approval(req).await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Without totp_verified, resolve returns Err → item is skipped.
        let resolved = mgr.resolve_all_for_session(
            "sess-totp",
            ApprovalDecision::Approved,
            Some("batch".to_string()),
        );
        assert_eq!(resolved, 0); // Skipped, not approved.

        // Request should still be pending (not consumed).
        assert!(mgr.get_pending(id).is_some());

        // With totp_verified=true, it should succeed.
        let (resp, _) = mgr
            .resolve(
                id,
                ApprovalDecision::Approved,
                Some("admin".to_string()),
                true,
                Some("admin"),
            )
            .unwrap();
        assert!(resp.decision.is_approved());

        let recent = mgr.list_recent(10);
        assert!(recent.iter().any(|r| r.request.id == id));
    }

    #[tokio::test]
    async fn test_submit_request_dedup_same_tool_use_id() {
        // Duplicate tool_use_id in same session is rejected.
        let mgr = Arc::new(default_manager());
        let deferred = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "dedup-id".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };

        let req1 = make_session_request("agent-1", "sess-dedup");
        let id1 = mgr.submit_request(req1, deferred.clone()).unwrap();

        // Same tool_use_id: rejected.
        let req2 = make_session_request("agent-1", "sess-dedup");
        let result = mgr.submit_request(req2, deferred.clone());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Duplicate"));

        // Different tool_use_id: allowed even if same session.
        let deferred2 = DeferredToolExecution {
            agent_id: "agent-1".to_string(),
            tool_use_id: "dedup-id-2".to_string(),
            tool_name: "shell_exec".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            allowed_tools: None,
            allowed_env_vars: None,
            exec_policy: None,
            sender_id: None,
            channel: None,
            workspace_root: None,
            force_human: false,
        };
        let req3 = make_session_request("agent-1", "sess-dedup");
        let id3 = mgr.submit_request(req3, deferred2).unwrap();
        assert_ne!(id1, id3);

        // Cleanup.
        let _ = mgr.resolve(id1, ApprovalDecision::Denied, None, false, None);
        let _ = mgr.resolve(id3, ApprovalDecision::Denied, None, false, None);
    }
}
