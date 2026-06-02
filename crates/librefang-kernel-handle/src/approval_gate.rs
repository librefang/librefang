use async_trait::async_trait;

use super::*;

// ============================================================================
// 7. ApprovalGate — approval policy queries + pending-approval lifecycle
// ============================================================================

#[async_trait]
pub trait ApprovalGate: Send + Sync {
    /// Check if a tool requires approval based on current policy.
    fn requires_approval(&self, tool_name: &str) -> bool {
        let _ = tool_name;
        false
    }

    /// Check if a tool requires approval, taking sender and channel context
    /// into account.  Falls back to `requires_approval()` by default.
    fn requires_approval_with_context(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> bool {
        let _ = (sender_id, channel);
        self.requires_approval(tool_name)
    }

    /// Check whether a tool is hard-denied for the given sender/channel context.
    fn is_tool_denied_with_context(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> bool {
        let _ = (tool_name, sender_id, channel);
        false
    }

    /// Resolve the per-user RBAC gate for a tool invocation (RBAC M3,
    /// issue #3054 Phase 2).
    ///
    /// Combines the user's `UserToolPolicy`, `channel_tool_rules`,
    /// `tool_categories`, and role-based approval escalation into a single
    /// runtime-facing verdict. Returns:
    ///
    /// * `Allow` — no per-user objection; continue with the existing
    ///   approval/capability gates.
    /// * `Deny` — hard deny; the dispatcher refuses without prompting.
    /// * `NeedsApproval` — user's own role would block, but a higher role
    ///   could authorise; route through the approval queue.
    ///
    /// Default impl returns `Allow` so installations without a real
    /// kernel (test stubs, embedded callers without an `AuthManager`)
    /// keep their pre-M3 behaviour. The real kernel always overrides
    /// this; flipping the default to `NeedsApproval` was discussed
    /// during PR #3205 review but rejected because it broke ~8 unrelated
    /// runtime tests that rely on the default mock — the loudness gain
    /// is not worth a fragile contract for stub kernels.
    fn resolve_user_tool_decision(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> librefang_types::user_policy::UserToolGate {
        let _ = (tool_name, sender_id, channel);
        librefang_types::user_policy::UserToolGate::Allow
    }

    /// Request approval for a tool execution. Blocks until approved/denied/timed out.
    async fn request_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
        session_id: Option<&str>,
    ) -> Result<librefang_types::approval::ApprovalDecision, KernelOpError> {
        let _ = (agent_id, tool_name, action_summary, session_id);
        Ok(librefang_types::approval::ApprovalDecision::Approved)
    }

    /// Submit a tool for approval without blocking. Returns request UUID immediately.
    async fn submit_tool_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
        deferred: librefang_types::tool::DeferredToolExecution,
        session_id: Option<&str>,
    ) -> Result<librefang_types::tool::ToolApprovalSubmission, KernelOpError> {
        let _ = (agent_id, tool_name, action_summary, deferred, session_id);
        Err(KernelOpError::unavailable("Approval system"))
    }

    /// Resolve an approval request and get the deferred payload.
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
        KernelOpError,
    > {
        let _ = (request_id, decision, decided_by, totp_verified, user_id);
        Err(KernelOpError::unavailable("Approval system"))
    }

    /// Check current status of an approval request.
    fn get_approval_status(
        &self,
        request_id: uuid::Uuid,
    ) -> Result<Option<librefang_types::approval::ApprovalDecision>, KernelOpError> {
        let _ = request_id;
        Ok(None)
    }
}
