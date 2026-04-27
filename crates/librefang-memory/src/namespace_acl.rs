//! Per-user memory namespace ACL guard (RBAC M3, issue #3054 Phase 2).
//!
//! Memory in LibreFang is partitioned by *namespace*. Today that means:
//! - `proactive` — proactive-memory store (mem0-style fragments)
//! - `kv:<key>` — structured key-value entries (one entry per key)
//! - `shared:<scope>` — peer-scoped shared memory
//! - `kg` — knowledge graph
//!
//! The kernel resolves an inbound request to a [`UserMemoryAccess`]
//! (via `AuthManager::memory_acl_for`) and wraps it in a
//! [`MemoryNamespaceGuard`]. Every memory call site is then expected to
//! ask the guard before reading/writing/deleting/exporting.
//!
//! This crate intentionally stops at *checking and redacting*. The
//! kernel owns the call sites and decides which namespace string to
//! pass — the guard doesn't know about session IDs, agent IDs, or
//! channel routing.

use librefang_types::memory::MemoryItem;
use librefang_types::taint::{redact_pii_in_text, TaintLabel};
use librefang_types::user_policy::UserMemoryAccess;
use std::collections::HashMap;

const PII_REDACTION: &str = "[REDACTED:PII]";
const PII_METADATA_KEY: &str = "taint_labels";

/// Outcome of a guarded memory call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceGate {
    /// The call is permitted. Caller continues normally.
    Allow,
    /// The user lacks the required namespace permission. The string is a
    /// human-readable reason that surfaces back to the LLM tool result.
    Deny(String),
}

impl NamespaceGate {
    /// Convenience constructor for the deny path.
    pub fn deny(reason: impl Into<String>) -> Self {
        NamespaceGate::Deny(reason.into())
    }

    /// Returns true when the call is allowed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, NamespaceGate::Allow)
    }
}

/// Stateful guard wrapping a [`UserMemoryAccess`] ACL.
///
/// Cheap to clone (one `UserMemoryAccess` is just a few `Vec<String>` +
/// three bools) so the kernel can hand a fresh guard to each call site.
#[derive(Debug, Clone)]
pub struct MemoryNamespaceGuard {
    acl: UserMemoryAccess,
}

impl MemoryNamespaceGuard {
    /// Construct a guard from a resolved per-user ACL.
    pub fn new(acl: UserMemoryAccess) -> Self {
        Self { acl }
    }

    /// Borrow the underlying ACL (for inspection / serialisation).
    pub fn acl(&self) -> &UserMemoryAccess {
        &self.acl
    }

    /// Gate a read against `namespace`.
    pub fn check_read(&self, namespace: &str) -> NamespaceGate {
        if self.acl.can_read(namespace) {
            NamespaceGate::Allow
        } else {
            NamespaceGate::deny(format!(
                "memory namespace '{namespace}' is not readable for the current user"
            ))
        }
    }

    /// Gate a write against `namespace`.
    pub fn check_write(&self, namespace: &str) -> NamespaceGate {
        if self.acl.can_write(namespace) {
            NamespaceGate::Allow
        } else {
            NamespaceGate::deny(format!(
                "memory namespace '{namespace}' is not writable for the current user"
            ))
        }
    }

    /// Gate a delete against `namespace`. Requires both write access AND
    /// the explicit `delete_allowed` flag.
    pub fn check_delete(&self, namespace: &str) -> NamespaceGate {
        if !self.acl.delete_allowed {
            return NamespaceGate::deny("memory delete is not permitted for the current user");
        }
        self.check_write(namespace)
    }

    /// Gate a bulk export against `namespace`. Requires both read access
    /// AND the explicit `export_allowed` flag.
    pub fn check_export(&self, namespace: &str) -> NamespaceGate {
        if !self.acl.export_allowed {
            return NamespaceGate::deny("memory export is not permitted for the current user");
        }
        self.check_read(namespace)
    }

    /// Returns `true` when the user is permitted to see PII-tagged
    /// content. When `false`, callers MUST run [`redact_item`] before
    /// returning fragments to the user.
    pub fn pii_access_allowed(&self) -> bool {
        self.acl.pii_access
    }

    /// Apply PII redaction to a single [`MemoryItem`] in place.
    ///
    /// A fragment is considered PII-tagged when:
    /// - its `metadata["taint_labels"]` array contains the string
    ///   `"Pii"` (matching [`TaintLabel::Pii`]'s `Display` form), OR
    /// - the regex stack from `taint::redact_pii_in_text` finds e-mail /
    ///   phone / SSN / credit-card patterns inside `content`.
    ///
    /// Both signals are checked because storage layers don't always
    /// propagate the metadata flag, but the regex pass is text-only and
    /// can't see structured taint.
    ///
    /// Returns `true` when redaction was applied.
    pub fn redact_item(&self, item: &mut MemoryItem) -> bool {
        if self.acl.pii_access {
            return false;
        }
        let mut redacted = false;
        if has_pii_label(&item.metadata) {
            item.content = PII_REDACTION.to_string();
            redacted = true;
        } else {
            let scrubbed = redact_pii_in_text(&item.content, PII_REDACTION);
            if scrubbed != item.content {
                item.content = scrubbed;
                redacted = true;
            }
        }
        if redacted {
            // Use insert (not or_insert_with) so the redaction signal is
            // authoritative even if a stale "redacted: false" was already
            // attached upstream.
            item.metadata
                .insert("redacted".to_string(), serde_json::Value::Bool(true));
        }
        redacted
    }

    /// Bulk-apply [`redact_item`](Self::redact_item) to a list of items.
    /// Returns the number of items that were touched.
    pub fn redact_all(&self, items: &mut [MemoryItem]) -> usize {
        let mut count = 0;
        for item in items {
            if self.redact_item(item) {
                count += 1;
            }
        }
        count
    }
}

/// Inspect a fragment metadata map for the `"taint_labels": [..]`
/// signal carrying [`TaintLabel::Pii`].
///
/// Match is **case-insensitive** so writers that hand-stamp labels like
/// `"PII"` (the conventional uppercase form many external services use)
/// or `"pii"` produce the same redaction outcome as the canonical
/// `"Pii"` we emit ourselves. Without the lowercase normalisation a
/// fragment tagged `"PII"` would slip past the metadata path and only
/// trigger the regex backstop — fine for free-form text but a leak for
/// structured PII (e-mail/phone we wrote into a custom field name).
///
/// Uses ASCII lowercasing — `TaintLabel` variants are pure-ASCII
/// identifiers, so locale-aware `to_lowercase()` is unnecessary cost
/// and risks edge cases (Turkish locale `I → ı`).
fn has_pii_label(metadata: &HashMap<String, serde_json::Value>) -> bool {
    let target = TaintLabel::Pii.to_string().to_ascii_lowercase();
    let Some(value) = metadata.get(PII_METADATA_KEY) else {
        return false;
    };
    let matches = |s: &str| s.eq_ignore_ascii_case(&target);
    match value {
        serde_json::Value::Array(arr) => arr.iter().any(|v| v.as_str().is_some_and(matches)),
        serde_json::Value::String(s) => matches(s),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::memory::{MemoryItem, MemoryLevel};

    fn acl(
        read: &[&str],
        write: &[&str],
        pii: bool,
        delete: bool,
        export: bool,
    ) -> UserMemoryAccess {
        UserMemoryAccess {
            readable_namespaces: read.iter().map(|s| s.to_string()).collect(),
            writable_namespaces: write.iter().map(|s| s.to_string()).collect(),
            pii_access: pii,
            delete_allowed: delete,
            export_allowed: export,
        }
    }

    #[test]
    fn namespace_read_allowlist() {
        let g = MemoryNamespaceGuard::new(acl(&["proactive", "kv:*"], &[], false, false, false));
        assert!(g.check_read("proactive").is_allowed());
        assert!(g.check_read("kv:user_alice").is_allowed());
        assert!(!g.check_read("shared:secrets").is_allowed());
    }

    #[test]
    fn namespace_write_allowlist_independent_from_read() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &["kv:scratch"], false, false, false));
        assert!(g.check_read("anything").is_allowed());
        assert!(g.check_write("kv:scratch").is_allowed());
        assert!(!g.check_write("kv:secrets").is_allowed());
    }

    #[test]
    fn namespace_delete_requires_flag_and_write() {
        // delete_allowed=false → deny even with write access.
        let no_delete = MemoryNamespaceGuard::new(acl(&["*"], &["kv:*"], false, false, false));
        assert!(matches!(
            no_delete.check_delete("kv:foo"),
            NamespaceGate::Deny(_)
        ));

        // delete_allowed=true but no write access → still denied.
        let no_write = MemoryNamespaceGuard::new(acl(&["*"], &[], false, true, false));
        assert!(matches!(
            no_write.check_delete("kv:foo"),
            NamespaceGate::Deny(_)
        ));

        // both → allowed.
        let ok = MemoryNamespaceGuard::new(acl(&["*"], &["kv:*"], false, true, false));
        assert!(ok.check_delete("kv:foo").is_allowed());
    }

    #[test]
    fn namespace_export_requires_flag_and_read() {
        let no_flag = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));
        assert!(matches!(
            no_flag.check_export("proactive"),
            NamespaceGate::Deny(_)
        ));

        let no_read = MemoryNamespaceGuard::new(acl(&[], &[], false, false, true));
        assert!(matches!(
            no_read.check_export("proactive"),
            NamespaceGate::Deny(_)
        ));

        let ok = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, true));
        assert!(ok.check_export("proactive").is_allowed());
    }

    #[test]
    fn redact_via_metadata_label_replaces_full_content() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));
        let mut item = MemoryItem::new(
            "alice's home address: 123 Main St".into(),
            MemoryLevel::User,
        );
        item.metadata
            .insert("taint_labels".to_string(), serde_json::json!(["Pii"]));
        assert!(g.redact_item(&mut item));
        assert_eq!(item.content, "[REDACTED:PII]");
        assert_eq!(
            item.metadata.get("redacted").unwrap(),
            &serde_json::Value::Bool(true)
        );
    }

    #[test]
    fn redact_via_metadata_label_matches_case_insensitively() {
        // External writers commonly use uppercase "PII" or lowercase
        // "pii"; both must trigger redaction even though the canonical
        // Display form is "Pii". Without case-insensitive matching, a
        // structured PII fragment whose `content` had no regex-detectable
        // tokens (custom field names, synthesised payloads, …) would
        // silently leak.
        for label in ["PII", "pii", "Pii"] {
            let g = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));
            let mut item =
                MemoryItem::new("structured payload no regex hits".into(), MemoryLevel::User);
            item.metadata.insert(
                "taint_labels".to_string(),
                serde_json::json!([label.to_string()]),
            );
            assert!(
                g.redact_item(&mut item),
                "label {label:?} must trigger redaction"
            );
            assert_eq!(item.content, "[REDACTED:PII]");
        }
        // Same coverage for the scalar-string metadata shape.
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));
        let mut item = MemoryItem::new("structured payload".into(), MemoryLevel::User);
        item.metadata
            .insert("taint_labels".to_string(), serde_json::json!("PII"));
        assert!(g.redact_item(&mut item));
        assert_eq!(item.content, "[REDACTED:PII]");
    }

    #[test]
    fn redact_via_regex_replaces_email_and_phone() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));
        let mut item = MemoryItem::new(
            "alice@example.com booked 555-123-4567".into(),
            MemoryLevel::User,
        );
        assert!(g.redact_item(&mut item));
        assert!(!item.content.contains("alice@example.com"));
        assert!(item.content.contains("[REDACTED:PII]"));
    }

    #[test]
    fn redact_skips_when_pii_access_granted() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], true, false, false));
        let mut item = MemoryItem::new("ssn 123-45-6789".into(), MemoryLevel::User);
        assert!(!g.redact_item(&mut item));
        assert!(item.content.contains("123-45-6789"));
    }

    #[test]
    fn redact_all_counts_touched_items() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));
        let mut items = vec![
            MemoryItem::new("nothing here".into(), MemoryLevel::User),
            MemoryItem::new("call 555-123-4567".into(), MemoryLevel::User),
            MemoryItem::new("email me at b@c.com".into(), MemoryLevel::User),
        ];
        assert_eq!(g.redact_all(&mut items), 2);
    }
}
