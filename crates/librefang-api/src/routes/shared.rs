//! Shared route helpers: account validation and ownership guards.

use crate::middleware::AccountId;
use axum::http::StatusCode;
use axum::Json;
use librefang_types::agent::AgentEntry;

/// Require a concrete account identity on tenant-facing or admin-only routes.
pub fn require_concrete_account<'a>(
    account: &'a AccountId,
) -> Result<&'a str, (StatusCode, Json<serde_json::Value>)> {
    account.0.as_deref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "X-Account-Id required"})),
        )
    })
}

/// Check that an agent belongs to the requesting owner.
///
/// Callers should require a concrete account first with `require_concrete_account()`.
/// Returns 404 (not 403) to avoid confirming agent existence to the wrong tenant.
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

/// Reject non-admin callers from admin-only endpoints after requiring a
/// concrete account identity.
pub fn require_admin_account(
    account: &AccountId,
    admin_accounts: &[String],
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let account_id = require_concrete_account(account)?;
    if admin_accounts.iter().any(|id| id == account_id) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "This endpoint requires admin access"})),
        ))
    }
}

/// Reject non-admin callers from admin-only endpoints.
///
/// This is the Qwntik-safe admin guard: non-public infrastructure endpoints
/// require a concrete `X-Account-Id`, and that account must be listed in
/// `KernelConfig::admin_accounts`.
///
/// Missing account identity is rejected with 400. Non-admin tenants receive 403.
pub fn require_admin(
    account: &AccountId,
    admin_accounts: &[String],
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    require_admin_account(account, admin_accounts)
}

/// Finalize a newly spawned agent by attaching the account ID.
///
/// This function is called after successfully spawning an agent in multi-tenant
/// contexts. It sets the `account_id` field on the agent entry to enforce
/// ownership boundaries for subsequent access checks.
///
/// When `AccountId(None)` reaches this helper, the agent remains unowned. That
/// state is migration debt and should not occur on tenant-facing request flows.
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
    fn test_require_concrete_account_accepts_present_header() {
        let account = AccountId(Some("tenant-a".to_string()));
        assert_eq!(require_concrete_account(&account).unwrap(), "tenant-a");
    }

    #[test]
    fn test_require_concrete_account_rejects_missing_header() {
        let account = AccountId(None);
        let (code, json) = require_concrete_account(&account).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "X-Account-Id required");
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
    fn test_require_admin_account_accepts_admin_tenant() {
        let account = AccountId(Some("admin".to_string()));
        assert!(require_admin_account(&account, &[String::from("admin")]).is_ok());
    }

    #[test]
    fn test_require_admin_account_rejects_non_admin_tenant() {
        let account = AccountId(Some("tenant-a".to_string()));
        let (code, json) = require_admin_account(&account, &[String::from("admin")]).unwrap_err();
        assert_eq!(code, StatusCode::FORBIDDEN);
        assert_eq!(json["error"], "This endpoint requires admin access");
    }

    #[test]
    fn test_require_admin_account_rejects_missing_header() {
        let account = AccountId(None);
        let (code, json) = require_admin_account(&account, &[String::from("admin")]).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "X-Account-Id required");
    }

    #[test]
    fn test_require_admin_rejects_missing_header() {
        let account = AccountId(None);
        let (code, json) = require_admin(&account, &[String::from("admin")]).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "X-Account-Id required");
    }

    #[test]
    fn test_finalize_spawned_agent_idempotent() {
        let mut entry = make_agent_entry(Some("tenant-a".to_string()));
        let account = AccountId(Some("tenant-a".to_string()));
        finalize_spawned_agent(&mut entry, &account);
        assert_eq!(entry.account_id, Some("tenant-a".to_string()));
    }
}
