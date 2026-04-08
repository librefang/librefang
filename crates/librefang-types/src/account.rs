use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tenant isolation boundary. Every resource belongs to exactly one account.
///
/// Uses `Option<String>` — NOT `Option<Uuid>` — matching openfang-ai's proven pattern.
/// This keeps a single representation across extractor, storage, migration, and comparison.
///
/// - `AccountId(Some("uuid-string"))` = multi-tenant request (SaaS, team isolation)
/// - `AccountId(None)` = compatibility/migration state, not a valid Qwntik
///   tenant-facing runtime identity
///
/// The string is opaque to the type system. Callers may use UUIDs, slugs, or any
/// format — the only invariant is: trimmed, non-empty, case-sensitive equality.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Option<String>);

impl AccountId {
    /// Compatibility sentinel for legacy storage defaults.
    /// This is migration debt, not a target runtime identity.
    pub const SYSTEM: &'static str = "system";

    /// Create a new random account ID (UUID v4).
    pub fn new() -> Self {
        Self(Some(Uuid::new_v4().to_string()))
    }

    /// Returns true if this is a scoped (non-None) request.
    pub fn is_scoped(&self) -> bool {
        self.0.is_some()
    }

    /// Returns the inner string, or the legacy `"system"` sentinel for
    /// compatibility layers that still need to round-trip old storage.
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
    use std::collections::HashSet;

    // ─────────────────────────────────────────────────────────────
    // Default Account Behavior (System Mode)
    // ─────────────────────────────────────────────────────────────

    /// Default account is unscoped (None). Validates backward compatibility for
    /// single-tenant / desktop mode where no X-Account-Id header is present.
    #[test]
    fn test_account_id_default_is_none() {
        assert_eq!(AccountId::default().0, None);
    }

    /// System mode fallback: Default account returns "system" string for database
    /// compatibility. Migrations use DEFAULT 'system' — this test ensures the
    /// type system matches the storage layer contract.
    #[test]
    fn test_account_id_as_str_or_system_returns_system_for_default() {
        assert_eq!(AccountId::default().as_str_or_system(), "system");
    }

    // ─────────────────────────────────────────────────────────────
    // Account ID Generation (Scoped Accounts)
    // ─────────────────────────────────────────────────────────────

    /// new() generates a UUID v4. Required for creating unique tenant IDs
    /// in multi-tenant initialization flows.
    #[test]
    fn test_account_id_new_generates_uuid() {
        let account = AccountId::new();
        assert!(account.0.is_some(), "new() must generate Some value");
        let id_str = account.0.as_ref().unwrap();
        // UUID v4: 8-4-4-4-12 = 36 chars with hyphens
        assert_eq!(id_str.len(), 36, "UUID v4 string must be exactly 36 chars");
    }

    /// Each call to new() generates a different UUID. Ensures tenant IDs are
    /// unique and don't collide.
    #[test]
    fn test_account_id_new_generates_different_uuids() {
        let a1 = AccountId::new();
        let a2 = AccountId::new();
        assert_ne!(
            a1, a2,
            "Successive calls to new() must generate different UUIDs"
        );
    }

    // ─────────────────────────────────────────────────────────────
    // Scoped vs Unscoped (is_scoped Check)
    // ─────────────────────────────────────────────────────────────

    /// is_scoped() returns true for Some accounts. Used by API extractors to
    /// distinguish multi-tenant requests (X-Account-Id present) from legacy
    /// mode (no header).
    #[test]
    fn test_account_id_is_scoped_true_for_some() {
        let scoped = AccountId(Some("tenant-123".to_string()));
        assert!(scoped.is_scoped());
    }

    /// is_scoped() returns false for None accounts (system/legacy mode).
    #[test]
    fn test_account_id_is_scoped_false_for_none() {
        assert!(!AccountId::default().is_scoped());
    }

    /// Scoped accounts return inner string from as_str_or_system().
    #[test]
    fn test_account_id_as_str_or_system_returns_inner_for_scoped() {
        let scoped = AccountId(Some("tenant-123".to_string()));
        assert_eq!(scoped.as_str_or_system(), "tenant-123");
    }

    // ─────────────────────────────────────────────────────────────
    // Equality (Ownership Matching for Handler Guards)
    // ─────────────────────────────────────────────────────────────

    /// Same AccountId values are equal. Used in Round 3 handlers to verify
    /// that request.account_id == agent.account_id.
    #[test]
    fn test_account_id_same_values_are_equal() {
        let a1 = AccountId(Some("tenant-a".to_string()));
        let a2 = AccountId(Some("tenant-a".to_string()));
        assert_eq!(a1, a2);
    }

    /// Different scoped accounts are not equal. Critical for preventing
    /// cross-tenant access: if handler receives account_id = "tenant-a"
    /// but agent has "tenant-b", they must not be equal.
    #[test]
    fn test_account_id_different_values_are_not_equal() {
        let a = AccountId(Some("tenant-a".to_string()));
        let b = AccountId(Some("tenant-b".to_string()));
        assert_ne!(a, b);
    }

    /// Scoped and unscoped accounts are not equal. Prevents treating system
    /// mode as a valid tenant ID.
    #[test]
    fn test_account_id_scoped_and_unscoped_are_not_equal() {
        let scoped = AccountId(Some("tenant-a".to_string()));
        let unscoped = AccountId::default();
        assert_ne!(scoped, unscoped);
    }

    /// All unscoped (None) accounts are equal to each other. System mode
    /// doesn't have tenant boundaries.
    #[test]
    fn test_account_id_all_unscoped_accounts_are_equal() {
        assert_eq!(AccountId::default(), AccountId::default());
        assert_eq!(AccountId(None), AccountId(None));
    }

    // ─────────────────────────────────────────────────────────────
    // Hashing (DashMap Lookups in Registry)
    // ─────────────────────────────────────────────────────────────

    /// Identical AccountIds hash to the same value. Required for use as
    /// DashMap keys in registry.list_by_account().
    #[test]
    fn test_account_id_hash_consistency() {
        let a1 = AccountId(Some("tenant-x".to_string()));
        let a2 = AccountId(Some("tenant-x".to_string()));

        let mut set = HashSet::new();
        set.insert(a1);
        set.insert(a2);

        assert_eq!(
            set.len(),
            1,
            "Identical AccountIds must hash to same bucket"
        );
    }

    /// Different AccountIds produce different hashes (usually). At minimum,
    /// they must not collide in the common case.
    #[test]
    fn test_account_id_different_values_hash_differently() {
        let a = AccountId(Some("tenant-a".to_string()));
        let b = AccountId(Some("tenant-b".to_string()));

        let mut set = HashSet::new();
        set.insert(a);
        set.insert(b);

        assert_eq!(
            set.len(),
            2,
            "Different AccountIds should hash to different buckets"
        );
    }

    // ─────────────────────────────────────────────────────────────
    // Serialization (JSON API Responses)
    // ─────────────────────────────────────────────────────────────

    /// AccountId roundtrips through JSON. Required for API responses where
    /// account_id is included in agent JSON.
    #[test]
    fn test_account_id_roundtrip_json() {
        let original = AccountId(Some("tenant-123".to_string()));
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AccountId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    /// Default (None) AccountId roundtrips through JSON.
    #[test]
    fn test_account_id_default_roundtrip_json() {
        let original = AccountId::default();
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AccountId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    // ─────────────────────────────────────────────────────────────
    // Edge Cases (Production Robustness)
    // ─────────────────────────────────────────────────────────────

    /// Empty string is stored as Some(""). This is technically scoped but
    /// has no usable value. Round 3 handlers must handle gracefully.
    #[test]
    fn test_account_id_empty_string_is_stored_as_scoped() {
        let empty = AccountId(Some(String::new()));
        assert!(
            empty.is_scoped(),
            "Empty string is Some, so is_scoped() = true"
        );
        assert_eq!(empty.as_str_or_system(), "");
    }

    /// Whitespace-only account IDs are preserved (not trimmed). Comparison
    /// will be strict: "  " != "system" != "".
    #[test]
    fn test_account_id_whitespace_preserved() {
        let ws = AccountId(Some("  ".to_string()));
        assert_eq!(ws.as_str_or_system(), "  ");
        assert_ne!(ws, AccountId::default());
    }

    // ─────────────────────────────────────────────────────────────
    // AccountStatus Enum
    // ─────────────────────────────────────────────────────────────

    /// All AccountStatus variants serialize and deserialize correctly.
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

    // ─────────────────────────────────────────────────────────────
    // Account Struct (Full Integration)
    // ─────────────────────────────────────────────────────────────

    /// Full Account struct roundtrips through JSON. Integration test verifying
    /// all fields (id, name, created_at, status) work together.
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
        // Note: created_at may lose nanosecond precision in JSON, don't compare directly
    }
}
