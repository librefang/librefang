use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tenant isolation boundary. Every resource belongs to exactly one account.
///
/// Uses `Option<String>` — NOT `Option<Uuid>` — matching openfang-ai's proven pattern.
/// This keeps a single representation across extractor, storage, migration, and comparison.
///
/// - `AccountId(Some("uuid-string"))` = multi-tenant request (SaaS, team isolation)
/// - `AccountId(None)` = legacy/desktop mode (admin, sees everything)
///
/// The string is opaque to the type system. Callers may use UUIDs, slugs, or any
/// format — the only invariant is: trimmed, non-empty, case-sensitive equality.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Option<String>);

impl AccountId {
    /// The implicit account for single-tenant / backward-compatible deployments.
    /// Matches the migration DEFAULT 'system' exactly.
    pub const SYSTEM: &'static str = "system";

    /// Create a new random account ID (UUID v4).
    pub fn new() -> Self {
        Self(Some(Uuid::new_v4().to_string()))
    }

    /// Returns true if this is a scoped (non-None) request.
    pub fn is_scoped(&self) -> bool {
        self.0.is_some()
    }

    /// Returns the inner string, or "system" for legacy/desktop.
    pub fn as_str_or_system(&self) -> &str {
        match &self.0 {
            Some(s) => s.as_str(),
            None => Self::SYSTEM,
        }
    }
}

/// Account metadata. Minimal for Phase 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String, // matches AccountId inner type
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub status: AccountStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    Active,
    Suspended,
    Deleted,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD Micro-cycle 1: System default account returns "system" string
    #[test]
    fn test_account_id_system_default() {
        let default = AccountId::default();
        assert_eq!(default.0, None, "Default should be None");
        assert_eq!(
            default.as_str_or_system(),
            "system",
            "as_str_or_system() must return 'system' for None"
        );
    }

    /// TDD Micro-cycle 2: new() generates a UUID-based account ID
    #[test]
    fn test_account_id_new_generates_uuid() {
        let account = AccountId::new();
        assert!(account.0.is_some(), "new() should create Some(uuid)");
        let id_str = account.0.as_ref().unwrap();
        // UUID v4 is 36 chars: 8-4-4-4-12 with hyphens
        assert_eq!(id_str.len(), 36, "UUID v4 string should be 36 chars");
    }

    /// TDD Micro-cycle 3: Scoped account (Some) returns is_scoped() = true
    #[test]
    fn test_account_id_scoped_is_true() {
        let scoped = AccountId(Some("tenant-123".to_string()));
        assert!(
            scoped.is_scoped(),
            "Scoped account should return is_scoped() = true"
        );
        assert_eq!(
            scoped.as_str_or_system(),
            "tenant-123",
            "as_str_or_system() should return the inner value"
        );
    }

    /// TDD Micro-cycle 4: Unscoped account (None) returns is_scoped() = false
    #[test]
    fn test_account_id_unscoped_is_false() {
        let unscoped = AccountId::default();
        assert!(
            !unscoped.is_scoped(),
            "Unscoped account should return is_scoped() = false"
        );
    }

    /// TDD Micro-cycle 5: Equality works across different tenants
    #[test]
    fn test_account_id_equality() {
        let a = AccountId(Some("tenant-a".to_string()));
        let b = AccountId(Some("tenant-b".to_string()));
        let a2 = AccountId(Some("tenant-a".to_string()));
        let system = AccountId::default();

        assert_eq!(a, a2, "Same tenant IDs should be equal");
        assert_ne!(a, b, "Different tenant IDs should not be equal");
        assert_ne!(a, system, "Scoped and unscoped should not be equal");
        assert_eq!(
            system,
            AccountId::default(),
            "System accounts should be equal"
        );
    }

    /// TDD Micro-cycle 6: Hash consistency for map/set usage
    #[test]
    fn test_account_id_hash_consistency() {
        use std::collections::HashSet;

        let a1 = AccountId(Some("tenant-x".to_string()));
        let a2 = AccountId(Some("tenant-x".to_string()));

        let mut set = HashSet::new();
        set.insert(a1);
        set.insert(a2); // Should be deduplicated

        assert_eq!(
            set.len(),
            1,
            "Identical AccountIds should hash to same value"
        );
    }

    /// TDD Micro-cycle 7: AccountStatus enum serialization
    #[test]
    fn test_account_status_serde() {
        let statuses = [
            AccountStatus::Active,
            AccountStatus::Suspended,
            AccountStatus::Deleted,
        ];

        for status in &statuses {
            let json = serde_json::to_string(status).expect("serialize");
            let back: AccountStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, *status, "Status should roundtrip through JSON");
        }
    }

    /// Integration: Full Account struct serialization
    #[test]
    fn test_account_roundtrip_serde() {
        let account = Account {
            id: "tenant-123".to_string(),
            name: "Acme Corp".to_string(),
            created_at: Utc::now(),
            status: AccountStatus::Active,
        };

        let json = serde_json::to_string(&account).expect("serialize");
        let back: Account = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.id, account.id);
        assert_eq!(back.name, account.name);
        assert_eq!(back.status, account.status);
    }
}
