//! Shared route helpers: account validation and ownership guards.
//!
//! Provides multi-tenant isolation via the `check_account()` function, which validates
//! that an agent belongs to the requesting owner based on the `X-Account-Id` header.

use crate::middleware::AccountId;
use axum::http::StatusCode;
use axum::Json;
use librefang_types::agent::AgentEntry;

/// Check that an agent belongs to the requesting owner.
///
/// When `AccountId(Some(owner))` is present, the agent's `account_id` must match.
/// Returns 404 (not 403) to avoid confirming agent existence to wrong owner.
/// When `AccountId(None)`, all agents are visible (admin/legacy behavior).
///
/// # Security Note
///
/// Returns 404 instead of 403 to prevent information disclosure — an attacker
/// cannot distinguish between "agent does not exist" and "agent exists but you
/// don't own it".
pub fn check_account(
    entry: &AgentEntry,
    account: &AccountId,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if let Some(ref account_id) = account.0 {
        if entry.account_id.as_deref() != Some(account_id.as_str()) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            ));
        }
    }
    Ok(())
}

/// Reject non-admin callers from admin-only endpoints.
///
/// Passes through when:
/// - `AccountId(None)` — single-tenant / desktop mode (no X-Account-Id header)
/// - `AccountId(Some(id))` where `id` is in `admin_accounts` — elevated tenant
///
/// Returns 403 Forbidden for all other scoped tenants.
///
/// `admin_accounts` comes from `KernelConfig::admin_accounts`. In multi-tenant
/// mode, `require_account_id` middleware forces all requests to carry an
/// `X-Account-Id`, so without this list admin-guarded endpoints would be
/// unreachable.
pub fn require_admin(
    account: &AccountId,
    admin_accounts: &[String],
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match &account.0 {
        None => Ok(()),                                    // single-tenant / desktop mode
        Some(id) if admin_accounts.contains(id) => Ok(()), // elevated admin tenant
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "This endpoint requires admin access"})),
        )),
    }
}

/// Finalize a newly spawned agent by attaching the account ID.
///
/// This function is called after successfully spawning an agent in multi-tenant
/// contexts. It sets the `account_id` field on the agent entry to enforce
/// ownership boundaries for subsequent access checks.
///
/// When `AccountId(None)` (system/admin mode), the agent remains unowned (legacy
/// behavior). When `AccountId(Some(id))`, the agent is assigned to that tenant.
#[allow(dead_code)] // Available for channels/config route scoping in Phase 2
pub fn finalize_spawned_agent(entry: &mut AgentEntry, account: &AccountId) {
    if let Some(ref account_id) = account.0 {
        entry.account_id = Some(account_id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use librefang_types::agent::{AgentId, AgentIdentity, AgentManifest, AgentMode, AgentState};

    fn make_agent_entry(account_id: Option<String>) -> AgentEntry {
        AgentEntry {
            id: AgentId::new(),
            account_id,
            name: "test-agent".to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Created,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: Default::default(),
            source_toml_path: None,
            tags: vec![],
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
        }
    }

    #[test]
    fn test_check_account_allows_matching_ownership() {
        let entry = make_agent_entry(Some("tenant-a".to_string()));
        let account = AccountId(Some("tenant-a".to_string()));
        assert!(check_account(&entry, &account).is_ok());
    }

    #[test]
    fn test_check_account_rejects_different_ownership() {
        let entry = make_agent_entry(Some("tenant-a".to_string()));
        let account = AccountId(Some("tenant-b".to_string()));
        let result = check_account(&entry, &account);
        assert!(result.is_err());
        let (code, json) = result.unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert_eq!(json["error"], "Agent not found");
    }

    #[test]
    fn test_check_account_allows_system_to_see_all() {
        let entry = make_agent_entry(Some("tenant-a".to_string()));
        let account = AccountId(None); // system/legacy mode
        assert!(check_account(&entry, &account).is_ok());
    }

    #[test]
    fn test_check_account_hides_existence_with_404() {
        let entry = make_agent_entry(Some("tenant-a".to_string()));
        let account = AccountId(Some("tenant-b".to_string()));
        let (code, _) = check_account(&entry, &account).unwrap_err();
        // Must be 404 (not found), not 403 (forbidden) to hide existence
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_check_account_unowned_agent_with_scoped_request() {
        let entry = make_agent_entry(None); // agent has no account_id (legacy)
        let account = AccountId(Some("tenant-a".to_string()));
        let result = check_account(&entry, &account);
        assert!(
            result.is_err(),
            "Scoped request should reject agent with no owner"
        );
        let (code, _) = result.unwrap_err();
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_finalize_spawned_agent_attaches_account_id() {
        let mut entry = make_agent_entry(None);
        let account = AccountId(Some("tenant-a".to_string()));
        finalize_spawned_agent(&mut entry, &account);
        assert_eq!(entry.account_id, Some("tenant-a".to_string()));
    }

    #[test]
    fn test_finalize_spawned_agent_leaves_legacy_unowned() {
        let mut entry = make_agent_entry(None);
        let account = AccountId(None); // system/legacy mode
        finalize_spawned_agent(&mut entry, &account);
        assert_eq!(entry.account_id, None);
    }

    #[test]
    fn test_finalize_spawned_agent_idempotent() {
        let mut entry = make_agent_entry(Some("tenant-a".to_string()));
        let account = AccountId(Some("tenant-a".to_string()));
        finalize_spawned_agent(&mut entry, &account);
        assert_eq!(entry.account_id, Some("tenant-a".to_string()));
    }
}
