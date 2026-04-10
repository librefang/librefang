use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tenant isolation boundary. Every resource belongs to exactly one account.
///
/// Uses `Option<String>` — NOT `Option<Uuid>` — matching openfang-ai's proven pattern.
/// This keeps a single representation across extractor, storage, migration, and comparison.
///
/// - `AccountId(Some("uuid-string"))` = multi-tenant request (SaaS, team isolation)
/// - `AccountId(None)` = compatibility/migration state, not a valid tenant-facing
///   runtime identity
///
/// The string is opaque to the type system. Callers may use UUIDs, slugs, or any
/// format — the only invariant is: trimmed, non-empty, case-sensitive equality.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Option<String>);

impl AccountId {
    /// The compatibility sentinel for callers that still need a concrete string
    /// in legacy/single-tenant contexts.
    pub const SYSTEM: &'static str = "system";

    /// Create a new random account ID (UUID v4).
    pub fn new() -> Self {
        Self(Some(Uuid::new_v4().to_string()))
    }

    /// Returns true if this is a scoped (non-None) request.
    pub fn is_scoped(&self) -> bool {
        self.0.is_some()
    }

    /// Returns the inner string, or the compatibility sentinel for legacy
    /// callers that still need a concrete string.
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
    pub id: String,
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
    use std::collections::HashSet;

    #[test]
    fn test_account_id_default_is_none() {
        assert_eq!(AccountId::default().0, None);
    }

    #[test]
    fn test_account_id_new_generates_uuid() {
        let account = AccountId::new();
        assert!(account.0.is_some(), "new() must generate Some value");
        let id_str = account.0.as_ref().unwrap();
        assert_eq!(id_str.len(), 36, "UUID v4 string must be exactly 36 chars");
    }

    #[test]
    fn test_account_id_new_generates_different_uuids() {
        let a1 = AccountId::new();
        let a2 = AccountId::new();
        assert_ne!(a1, a2);
    }

    #[test]
    fn test_account_id_is_scoped_true_for_some() {
        let scoped = AccountId(Some("tenant-123".to_string()));
        assert!(scoped.is_scoped());
    }

    #[test]
    fn test_account_id_is_scoped_false_for_none() {
        assert!(!AccountId::default().is_scoped());
    }

    #[test]
    fn test_account_id_as_str_or_system() {
        let scoped = AccountId(Some("tenant-123".to_string()));
        let unscoped = AccountId::default();
        assert_eq!(scoped.as_str_or_system(), "tenant-123");
        assert_eq!(unscoped.as_str_or_system(), AccountId::SYSTEM);
    }

    #[test]
    fn test_account_id_scoped_value_preserved() {
        let scoped = AccountId(Some("tenant-123".to_string()));
        assert_eq!(scoped.0.as_deref(), Some("tenant-123"));
    }

    #[test]
    fn test_account_id_same_values_are_equal() {
        let a1 = AccountId(Some("tenant-a".to_string()));
        let a2 = AccountId(Some("tenant-a".to_string()));
        assert_eq!(a1, a2);
    }

    #[test]
    fn test_account_id_different_values_are_not_equal() {
        let a = AccountId(Some("tenant-a".to_string()));
        let b = AccountId(Some("tenant-b".to_string()));
        assert_ne!(a, b);
    }

    #[test]
    fn test_account_id_scoped_and_unscoped_are_not_equal() {
        let scoped = AccountId(Some("tenant-a".to_string()));
        let unscoped = AccountId::default();
        assert_ne!(scoped, unscoped);
    }

    #[test]
    fn test_account_id_all_unscoped_accounts_are_equal() {
        assert_eq!(AccountId::default(), AccountId::default());
        assert_eq!(AccountId(None), AccountId(None));
    }

    #[test]
    fn test_account_id_hash_consistency() {
        let a1 = AccountId(Some("tenant-x".to_string()));
        let a2 = AccountId(Some("tenant-x".to_string()));

        let mut set = HashSet::new();
        set.insert(a1);
        set.insert(a2);

        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_account_id_different_values_hash_differently() {
        let a = AccountId(Some("tenant-a".to_string()));
        let b = AccountId(Some("tenant-b".to_string()));

        let mut set = HashSet::new();
        set.insert(a);
        set.insert(b);

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_account_id_roundtrip_json() {
        let original = AccountId(Some("tenant-123".to_string()));
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AccountId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn test_account_id_default_roundtrip_json() {
        let original = AccountId::default();
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AccountId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn test_account_id_empty_string_is_stored_as_scoped() {
        let empty = AccountId(Some(String::new()));
        assert!(empty.is_scoped());
        assert_eq!(empty.0.as_deref(), Some(""));
    }

    #[test]
    fn test_account_id_whitespace_preserved() {
        let ws = AccountId(Some("  ".to_string()));
        assert_eq!(ws.0.as_deref(), Some("  "));
        assert_ne!(ws, AccountId::default());
    }

    #[test]
    fn test_account_status_all_variants_roundtrip() {
        let statuses = [
            AccountStatus::Active,
            AccountStatus::Suspended,
            AccountStatus::Deleted,
        ];

        for status in &statuses {
            let json = serde_json::to_string(status).expect("serialize");
            let back: AccountStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, *status);
        }
    }

    #[test]
    fn test_account_roundtrip_json() {
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
