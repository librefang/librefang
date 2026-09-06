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

/// Metadata key holding the single previous version of a memory's content, as a bare JSON string.
const PREVIOUS_CONTENT_KEY: &str = "previous_content";
/// Metadata key holding the full version chain, as an array of `{"content": …, "replaced_at": …}` objects.
const VERSION_HISTORY_KEY: &str = "version_history";
/// Field inside one version-history entry that carries the superseded text.
const HISTORY_CONTENT_FIELD: &str = "content";
/// Flag stamped on anything this module rewrote, so a consumer can tell a redacted value from a value that happened to look like the marker.
const REDACTED_FLAG: &str = "redacted";

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
    /// Redaction covers `content` **and** the previous-version text the same fragment carries in `metadata` — `previous_content` and the `version_history` chain, both written verbatim by `ProactiveMemoryStore` on every in-place update.
    /// Scrubbing `content` alone left the superseded sentence — the same e-mail, the same phone number — fully readable one key over, and `MemoryItem::metadata` is a plainly-serialized public field, so it shipped to the caller on every list, search and duplicate-group response.
    /// The `export_all*` path is deliberately not part of this: it gates on the stronger `export` capability precisely because an export dumps raw rows.
    ///
    /// Returns `true` when redaction was applied to any of them.
    pub fn redact_item(&self, item: &mut MemoryItem) -> bool {
        if self.acl.pii_access {
            return false;
        }
        // Read the label before mutating: the metadata map is about to be rewritten in place.
        let labelled = has_pii_label(&item.metadata);
        let mut redacted = false;
        if labelled {
            item.content = PII_REDACTION.to_string();
            redacted = true;
        } else {
            let scrubbed = redact_pii_in_text(&item.content, PII_REDACTION);
            if scrubbed != item.content {
                item.content = scrubbed;
                redacted = true;
            }
        }
        if redact_version_metadata(&mut item.metadata, labelled) {
            redacted = true;
        }
        if redacted {
            // Use insert (not or_insert_with) so the redaction signal is
            // authoritative even if a stale "redacted: false" was already
            // attached upstream.
            item.metadata
                .insert(REDACTED_FLAG.to_string(), serde_json::Value::Bool(true));
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

    /// Apply the same redaction to raw `version_history` entries lifted straight out of a fragment's metadata — the shape `ProactiveMemoryStore::history` returns, where no enclosing [`MemoryItem`] exists for [`redact_item`](Self::redact_item) to work on.
    ///
    /// `fragment_metadata` is the owning fragment's metadata, needed for the `taint_labels` signal: the entries themselves carry no labels, so without it a structurally-tagged fragment whose old text has no regex-detectable token would come back verbatim.
    ///
    /// Returns the number of entries rewritten.
    pub fn redact_history_entries(
        &self,
        fragment_metadata: &HashMap<String, serde_json::Value>,
        entries: &mut [serde_json::Value],
    ) -> usize {
        if self.acl.pii_access {
            return 0;
        }
        redact_history_entries_in_place(entries, has_pii_label(fragment_metadata))
    }
}

/// Scrub the previous-version text a fragment's metadata carries, in place.
///
/// `full` mirrors the policy [`MemoryNamespaceGuard::redact_item`] applies to `content`: a fragment carrying the `Pii` taint label has every stored version replaced wholesale, otherwise each version goes through the same regex pass.
///
/// Returns `true` when anything changed.
fn redact_version_metadata(metadata: &mut HashMap<String, serde_json::Value>, full: bool) -> bool {
    let mut changed = false;
    if let Some(value) = metadata.get_mut(PREVIOUS_CONTENT_KEY) {
        changed |= redact_text_value(value, full);
    }
    if let Some(serde_json::Value::Array(entries)) = metadata.get_mut(VERSION_HISTORY_KEY) {
        changed |= redact_history_entries_in_place(entries, full) > 0;
    }
    changed
}

/// Redact the superseded text of each version-history entry and stamp the entry as redacted.
///
/// The canonical entry is `{"content": …, "replaced_at": …}`; a bare string is accepted too because the chain is untyped JSON that an operator can hand-write into an import payload, and dropping such an entry from redaction would be a silent leak rather than a visible error.
///
/// Returns the number of entries rewritten.
fn redact_history_entries_in_place(entries: &mut [serde_json::Value], full: bool) -> usize {
    let mut count = 0;
    for entry in entries {
        match entry {
            serde_json::Value::Object(map) => {
                let touched = map
                    .get_mut(HISTORY_CONTENT_FIELD)
                    .is_some_and(|content| redact_text_value(content, full));
                if touched {
                    map.insert(REDACTED_FLAG.to_string(), serde_json::Value::Bool(true));
                    count += 1;
                }
            }
            bare => {
                if redact_text_value(bare, full) {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Redact one JSON string in place, returning `true` when the value changed.
///
/// Non-string values are left alone: these keys are text fields by contract, and a caller that stored a number or an object there stored something this module cannot read as prose.
fn redact_text_value(value: &mut serde_json::Value, full: bool) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let replacement = if full {
        PII_REDACTION.to_string()
    } else {
        redact_pii_in_text(text, PII_REDACTION)
    };
    if replacement == text {
        return false;
    }
    *value = serde_json::Value::String(replacement);
    true
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

    /// Prior versions of a memory are the same sentence one key over: `ProactiveMemoryStore` writes `previous_content` and the `version_history` chain verbatim on every in-place update, and `MemoryItem::metadata` serializes plainly into every list / search / export response.
    /// Pre-fix `redact_item` rewrote `content` only, so a caller without `pii_access` got the e-mail scrubbed from the current version and handed back intact from the previous one.
    #[test]
    fn redact_scrubs_the_version_chain_in_metadata_not_only_content() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));
        let mut item = MemoryItem::new("reach me at bob@example.com".into(), MemoryLevel::User);
        item.metadata.insert(
            "previous_content".to_string(),
            serde_json::json!("reach me at alice@example.com"),
        );
        item.metadata.insert(
            "version_history".to_string(),
            serde_json::json!([
                {"content": "reach me at alice@example.com", "replaced_at": "2026-01-01T00:00:00Z"},
                {"content": "call me on 555-123-4567", "replaced_at": "2026-01-02T00:00:00Z"},
            ]),
        );

        assert!(g.redact_item(&mut item));

        let serialized = serde_json::to_string(&item.metadata).expect("metadata serializes");
        assert!(
            !serialized.contains("alice@example.com") && !serialized.contains("555-123-4567"),
            "no prior version may survive anywhere in the metadata: {serialized}"
        );
        assert!(
            item.metadata["previous_content"]
                .as_str()
                .is_some_and(|text| text.contains(PII_REDACTION)),
            "previous_content must be redacted: {serialized}"
        );
        let history = item.metadata["version_history"]
            .as_array()
            .expect("the chain stays an array");
        for entry in history {
            assert!(
                entry["content"]
                    .as_str()
                    .is_some_and(|text| text.contains(PII_REDACTION)),
                "every entry's content must be redacted: {entry}"
            );
            assert_eq!(
                entry["redacted"],
                serde_json::Value::Bool(true),
                "a rewritten entry must say so, so a consumer can tell it from stored text that \
                 happened to look like the marker: {entry}"
            );
        }
    }

    /// A structurally-tagged fragment has `content` replaced wholesale rather than regex-scrubbed, and the stored versions must follow the same policy: their text carries no regex-detectable token, so a per-entry regex pass alone would hand it back verbatim.
    #[test]
    fn pii_labelled_fragment_has_every_stored_version_replaced_wholesale() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));
        let mut item = MemoryItem::new(
            "structured payload, no regex hits".into(),
            MemoryLevel::User,
        );
        item.metadata
            .insert("taint_labels".to_string(), serde_json::json!(["Pii"]));
        item.metadata.insert(
            "previous_content".to_string(),
            serde_json::json!("older structured payload"),
        );
        item.metadata.insert(
            "version_history".to_string(),
            serde_json::json!([{"content": "older structured payload", "replaced_at": "2026-01-01T00:00:00Z"}]),
        );

        assert!(g.redact_item(&mut item));
        assert_eq!(item.content, PII_REDACTION);
        assert_eq!(
            item.metadata["previous_content"],
            serde_json::json!(PII_REDACTION)
        );
        assert_eq!(
            item.metadata["version_history"][0]["content"],
            serde_json::json!(PII_REDACTION)
        );
    }

    /// The history route hands over raw `version_history` entries with no enclosing `MemoryItem`, so the same redaction has to be reachable from the entries alone — including the label case, whose signal lives on the owning fragment rather than on any entry.
    #[test]
    fn redact_history_entries_covers_object_and_bare_entries() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], false, false, false));

        let mut entries = vec![
            serde_json::json!({"content": "mail alice@example.com", "replaced_at": "2026-01-01T00:00:00Z"}),
            serde_json::json!("mail bob@example.com"),
        ];
        assert_eq!(g.redact_history_entries(&HashMap::new(), &mut entries), 2);
        let rendered = serde_json::to_string(&entries).expect("entries serialize");
        assert!(
            !rendered.contains("alice@example.com") && !rendered.contains("bob@example.com"),
            "both entry shapes must be scrubbed: {rendered}"
        );

        let mut labelled = HashMap::new();
        labelled.insert("taint_labels".to_string(), serde_json::json!(["Pii"]));
        let mut entries = vec![serde_json::json!({"content": "no regex hits in here"})];
        assert_eq!(g.redact_history_entries(&labelled, &mut entries), 1);
        assert_eq!(entries[0]["content"], serde_json::json!(PII_REDACTION));
    }

    #[test]
    fn redact_history_entries_is_a_noop_when_pii_access_is_granted() {
        let g = MemoryNamespaceGuard::new(acl(&["*"], &[], true, false, false));
        let mut entries = vec![serde_json::json!({"content": "mail alice@example.com"})];
        assert_eq!(g.redact_history_entries(&HashMap::new(), &mut entries), 0);
        assert_eq!(
            entries[0]["content"],
            serde_json::json!("mail alice@example.com")
        );
    }
}
