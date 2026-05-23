//! PII (Personally Identifiable Information) filter for LLM context.
//!
//! Provides regex-based detection and redaction/pseudonymization of PII
//! in user messages and sender context before they are sent to LLM providers.
//!
//! Built-in patterns detect:
//! - Email addresses
//! - Phone numbers (E.164 and common formats)
//! - Credit card numbers (Visa, Mastercard, Amex, Discover)
//! - US Social Security Numbers (SSN)
//!
//! Additional patterns can be configured via `PrivacyConfig::redact_patterns`.

use librefang_channels::types::SenderContext;
use librefang_types::config::PrivacyMode;
use parking_lot::RwLock;
// Audit: pii-filter-regex-no-size-cap. Switched from `regex_lite`
// to the full `regex` crate so we can use `RegexBuilder` with
// `size_limit` / `dfa_size_limit` on operator-supplied patterns.
// regex_lite has no equivalent guard — an adversarial pattern like
// `(a|a|...|a){50}` would push the parser into O(n²)+ compile
// loops with the lite variant.
use regex::{Regex, RegexBuilder};
use std::collections::{HashMap, VecDeque};

/// Placeholder used in `Redact` mode.
const REDACTED_PLACEHOLDER: &str = "[REDACTED]";

/// Compiled-NFA size cap for operator-supplied PII patterns. 1 MiB
/// is generous for any realistic PII rule; alternation-bomb /
/// nested-quantifier patterns that would otherwise blow the
/// compiler's working set hit this ceiling and are rejected.
const PII_REGEX_SIZE_LIMIT_BYTES: usize = 1 << 20; // 1 MiB

/// Compiled-DFA cache cap. Same intent as `size_limit` but for the
/// lazy-DFA cache `regex` uses to amortise scan cost. Bounded so a
/// single pathological pattern can't dominate the cache.
const PII_REGEX_DFA_SIZE_LIMIT_BYTES: usize = 1 << 20; // 1 MiB

/// Maximum source length for a PII regex. Compilation cost grows
/// with pattern length; a hard cap on the source closes the
/// `(a|a|…|a){50}` style alternation-bomb at parse time, before
/// the cap-limited builder even runs.
pub(crate) const PII_REGEX_MAX_SOURCE_LEN: usize = 4 * 1024; // 4 KiB

/// Compile a PII regex with the size / source-length caps applied.
/// Returns the compiled regex on success, or a human-readable error
/// when the source is too long or the cap was exceeded during
/// compilation. Used for both built-in and operator-supplied
/// patterns so the bound is uniform.
///
/// Audit: pii-filter-regex-no-size-cap.
pub(crate) fn compile_pii_regex(pat: &str) -> Result<Regex, String> {
    if pat.len() > PII_REGEX_MAX_SOURCE_LEN {
        return Err(format!(
            "pattern source ({} bytes) exceeds PII_REGEX_MAX_SOURCE_LEN \
             ({} bytes) — refusing to compile",
            pat.len(),
            PII_REGEX_MAX_SOURCE_LEN
        ));
    }
    RegexBuilder::new(pat)
        .size_limit(PII_REGEX_SIZE_LIMIT_BYTES)
        .dfa_size_limit(PII_REGEX_DFA_SIZE_LIMIT_BYTES)
        .build()
        .map_err(|e| e.to_string())
}

/// Maximum number of distinct PII values cached for stable pseudonyms.
/// When exceeded, the oldest entries are evicted in FIFO order.
/// At ~100 bytes per entry this caps the map at ~1 MB — bounded enough
/// to prevent multi-GB growth on long-running daemons (#3489).
const PSEUDONYM_MAP_CAP: usize = 10_000;

/// Built-in PII regex patterns (label, pattern).
///
/// These are compiled once at `PiiFilter` construction time.
const BUILTIN_PATTERNS: &[(&str, &str)] = &[
    // WhatsApp JIDs: `<digits>@lid` / `<digits>@s.whatsapp.net` / `<digits>@c.us`
    // / `<digits>@g.us` / `<digits>@broadcast` / `<digits>@newsletter`. Also
    // multi-device addressing — `<digits>:<device>@<domain>` (baileys /
    // whatsmeow both emit these) — handled by the optional `(?::\d+)?` group
    // before `@`. Without it the `phone` pattern would partial-match the
    // leading digits and leave `:5@s.whatsapp.net` visible (houko #5469
    // review — same partial-redact class this PR exists to eliminate).
    //
    // Must come BEFORE `phone` so the digit prefix is consumed as part of the
    // JID and not partial-matched by `phone` (which has no lookahead support
    // in `regex_lite`). A partial match would leave the trailing digits and
    // `@<domain>` suffix visible (e.g. `[REDACTED]257@lid`), which is both an
    // information leak and breaks downstream consumers that parse JIDs from
    // the redacted text.
    (
        "whatsapp_jid",
        r"\b\d{5,20}(?::\d+)?@(?:lid|s\.whatsapp\.net|c\.us|g\.us|broadcast|newsletter)\b",
    ),
    // Email addresses
    ("email", r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}"),
    // Phone numbers: E.164 (+1234567890), US formats, international with spaces/dashes
    (
        "phone",
        r"(?:\+\d{1,3}[\s\-]?)?\(?\d{2,4}\)?[\s.\-]?\d{3,4}[\s.\-]?\d{3,4}",
    ),
    // Credit card: Visa(4), MC(51-55), Amex(34/37), Discover(6011/65) with spaces/dashes
    (
        "credit_card",
        r"\b(?:4\d{3}|5[1-5]\d{2}|3[47]\d{2}|6(?:011|5\d{2}))[\s\-]?\d{4}[\s\-]?\d{4}[\s\-]?\d{4}(?:\d{3})?\b",
    ),
    // US Social Security Numbers (123-45-6789 or 123456789)
    ("ssn", r"\b\d{3}[\-\s]?\d{2}[\-\s]?\d{4}\b"),
];

/// PII filter that detects and replaces personally identifiable information.
///
/// Maintains a pseudonym map for stable replacements within a session
/// (e.g. the same email always maps to the same pseudonym).
///
/// The map is bounded (`PSEUDONYM_MAP_CAP`) and evicts FIFO-style — long-lived
/// pseudonyms for hot values stay stable; cold values may be re-pseudonymized
/// once evicted, which is acceptable for the redaction use case.
pub struct PiiFilter {
    /// Compiled built-in + custom regex patterns with their labels.
    patterns: Vec<(String, Regex)>,
    /// Bounded LRU-ish state for stable pseudonyms, behind a single
    /// RwLock so reads (the common case: existing PII value lookup) are
    /// concurrent and writes serialize.
    state: RwLock<PseudonymState>,
}

/// Combined state for the bounded pseudonym map. Kept under one lock so
/// insertion order and counters cannot drift apart under concurrent writes.
struct PseudonymState {
    /// `category:value` -> pseudonym label.
    map: HashMap<String, String>,
    /// Insertion order — used to evict the oldest entry when the cap is hit.
    order: VecDeque<String>,
    /// Per-category sequential counter for generating new pseudonyms.
    counters: HashMap<String, u32>,
}

impl PiiFilter {
    /// Create a new PII filter with built-in patterns and optional custom patterns.
    ///
    /// Invalid custom regex patterns are logged and skipped.
    pub fn new(custom_patterns: &[String]) -> Self {
        let mut patterns = Vec::with_capacity(BUILTIN_PATTERNS.len() + custom_patterns.len());

        for (label, pat) in BUILTIN_PATTERNS {
            // Built-in patterns go through the same `compile_pii_regex`
            // size-cap path as operator input — keeps the bound uniform
            // and means any future built-in that bumps against the
            // ceiling shows up in CI test rather than at runtime.
            match compile_pii_regex(pat) {
                Ok(re) => patterns.push((label.to_string(), re)),
                Err(e) => {
                    tracing::warn!(pattern = pat, error = %e, "Failed to compile built-in PII pattern");
                }
            }
        }

        for (i, pat) in custom_patterns.iter().enumerate() {
            // Operator-supplied patterns: `compile_pii_regex` rejects
            // any source > PII_REGEX_MAX_SOURCE_LEN at parse time and
            // any compilation that would exceed PII_REGEX_SIZE_LIMIT_BYTES
            // / PII_REGEX_DFA_SIZE_LIMIT_BYTES during build. The error
            // path stays "log + skip" so a single bad rule does not
            // disable the whole filter — but it now produces a tracing
            // warn explicitly tagged with the cap that fired.
            // Audit: pii-filter-regex-no-size-cap.
            match compile_pii_regex(pat) {
                Ok(re) => patterns.push((format!("custom_{i}"), re)),
                Err(e) => {
                    tracing::warn!(pattern = pat, error = %e, "Failed to compile custom PII pattern — skipping");
                }
            }
        }

        Self {
            patterns,
            state: RwLock::new(PseudonymState {
                map: HashMap::new(),
                order: VecDeque::new(),
                counters: HashMap::new(),
            }),
        }
    }

    /// Filter PII from a text message according to the given privacy mode.
    ///
    /// - `Off`: returns the text unchanged.
    /// - `Redact`: replaces all PII matches with `[REDACTED]`.
    /// - `Pseudonymize`: replaces PII with stable pseudonyms (e.g. `[Email-A]`).
    pub fn filter_message(&self, text: &str, mode: &PrivacyMode) -> String {
        match mode {
            PrivacyMode::Off => text.to_string(),
            PrivacyMode::Redact => self.redact(text),
            PrivacyMode::Pseudonymize => self.pseudonymize(text),
        }
    }

    /// Filter PII from a `SenderContext`, replacing user_id and display_name.
    ///
    /// - `Off`: returns the context unchanged.
    /// - `Redact`: replaces user_id and display_name with `[REDACTED]`.
    /// - `Pseudonymize`: replaces with stable pseudonyms (e.g. `User-A`).
    pub fn filter_sender_context(
        &self,
        sender: &SenderContext,
        mode: &PrivacyMode,
    ) -> SenderContext {
        match mode {
            PrivacyMode::Off => sender.clone(),
            PrivacyMode::Redact => SenderContext {
                channel: sender.channel.clone(),
                user_id: REDACTED_PLACEHOLDER.to_string(),
                display_name: REDACTED_PLACEHOLDER.to_string(),
                is_group: sender.is_group,
                was_mentioned: sender.was_mentioned,
                thread_id: sender.thread_id.clone(),
                account_id: sender
                    .account_id
                    .as_ref()
                    .map(|_| REDACTED_PLACEHOLDER.to_string()),
                use_canonical_session: sender.use_canonical_session,
                ..Default::default()
            },
            PrivacyMode::Pseudonymize => {
                let pseudo_name = self.get_or_create_pseudonym(&sender.display_name, "user");
                let pseudo_id = self.get_or_create_pseudonym(&sender.user_id, "user_id");
                SenderContext {
                    channel: sender.channel.clone(),
                    user_id: pseudo_id,
                    display_name: pseudo_name,
                    is_group: sender.is_group,
                    was_mentioned: sender.was_mentioned,
                    thread_id: sender.thread_id.clone(),
                    account_id: sender
                        .account_id
                        .as_ref()
                        .map(|id| self.get_or_create_pseudonym(id, "account")),
                    use_canonical_session: sender.use_canonical_session,
                    ..Default::default()
                }
            }
        }
    }

    /// Replace all PII matches with `[REDACTED]`.
    fn redact(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (_label, re) in &self.patterns {
            result = re.replace_all(&result, REDACTED_PLACEHOLDER).to_string();
        }
        result
    }

    /// Replace all PII matches with stable pseudonyms.
    fn pseudonymize(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (label, re) in &self.patterns {
            // Collect matches first to avoid borrow issues
            let matches: Vec<String> = re
                .find_iter(&result)
                .map(|m| m.as_str().to_string())
                .collect();
            for matched in matches {
                let pseudonym = self.get_or_create_pseudonym(&matched, label);
                result = result.replace(&matched, &pseudonym);
            }
        }
        result
    }

    /// Get or create a stable pseudonym for a given value.
    ///
    /// Pseudonyms follow the pattern `[{Category}-{Letter}]` where the letter
    /// increments (A, B, C, ...) for each new unique value in that category.
    /// Hot path: read-only lookup under a shared lock. Slow path takes a
    /// write lock to insert (and possibly evict the oldest entry).
    fn get_or_create_pseudonym(&self, value: &str, category: &str) -> String {
        // Key includes category to avoid collisions between different PII types
        let key = format!("{category}:{value}");

        // Fast path: shared read lock.
        if let Some(existing) = self.state.read().map.get(&key) {
            return existing.clone();
        }

        // Slow path: take exclusive lock, double-check, insert.
        let mut state = self.state.write();
        if let Some(existing) = state.map.get(&key) {
            return existing.clone();
        }
        let counter = state.counters.entry(category.to_string()).or_insert(0);
        let letter = index_to_label(*counter);
        *counter += 1;

        let label = capitalize_category(category);
        let pseudonym = format!("[{label}-{letter}]");
        state.map.insert(key.clone(), pseudonym.clone());
        state.order.push_back(key);

        // Evict FIFO until under cap.
        while state.map.len() > PSEUDONYM_MAP_CAP {
            if let Some(oldest) = state.order.pop_front() {
                state.map.remove(&oldest);
            } else {
                break;
            }
        }
        pseudonym
    }
}

/// Convert a zero-based index to a letter label: 0→A, 1→B, ..., 25→Z, 26→AA, etc.
fn index_to_label(mut idx: u32) -> String {
    let mut label = String::new();
    loop {
        label.insert(0, (b'A' + (idx % 26) as u8) as char);
        if idx < 26 {
            break;
        }
        idx = idx / 26 - 1;
    }
    label
}

/// Capitalize category name for display (e.g. "email" -> "Email", "credit_card" -> "Credit_card").
fn capitalize_category(cat: &str) -> String {
    let mut chars = cat.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_filter() -> PiiFilter {
        PiiFilter::new(&[])
    }

    // -- Mode::Off passthrough --

    #[test]
    fn test_off_mode_passthrough() {
        let filter = make_filter();
        let text = "Call me at +1-555-123-4567 or email john@example.com";
        let result = filter.filter_message(text, &PrivacyMode::Off);
        assert_eq!(result, text);
    }

    // -- Email detection --

    #[test]
    fn test_redact_email() {
        let filter = make_filter();
        let text = "Send mail to alice@example.com please";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("alice@example.com"));
        assert!(result.contains(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn test_pseudonymize_email() {
        let filter = make_filter();
        let text = "Contact alice@example.com or bob@example.com";
        let result = filter.filter_message(text, &PrivacyMode::Pseudonymize);
        assert!(!result.contains("alice@example.com"));
        assert!(!result.contains("bob@example.com"));
        assert!(result.contains("[Email-A]"));
        assert!(result.contains("[Email-B]"));
    }

    // -- Phone detection --

    #[test]
    fn test_redact_phone_e164() {
        let filter = make_filter();
        let text = "Call +14155551234";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("+14155551234"));
        assert!(result.contains(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn test_redact_phone_formatted() {
        let filter = make_filter();
        let text = "Call (415) 555-1234";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("(415) 555-1234"));
    }

    // -- SSN detection --

    #[test]
    fn test_redact_ssn() {
        let filter = make_filter();
        let text = "SSN: 123-45-6789";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("123-45-6789"));
        assert!(result.contains(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn test_redact_ssn_no_dashes() {
        let filter = make_filter();
        let text = "SSN: 123456789";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("123456789"));
    }

    // -- Credit card detection --

    #[test]
    fn test_redact_credit_card() {
        let filter = make_filter();
        let text = "Card: 4111 1111 1111 1111";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("4111 1111 1111 1111"));
        assert!(result.contains(REDACTED_PLACEHOLDER));
    }

    // -- Pseudonym stability --

    #[test]
    fn test_pseudonym_stability() {
        let filter = make_filter();
        let text1 = "Email alice@example.com";
        let text2 = "Again alice@example.com";
        let r1 = filter.filter_message(text1, &PrivacyMode::Pseudonymize);
        let r2 = filter.filter_message(text2, &PrivacyMode::Pseudonymize);
        // Same email should produce the same pseudonym
        assert!(r1.contains("[Email-A]"));
        assert!(r2.contains("[Email-A]"));
    }

    // -- Custom patterns --

    #[test]
    fn test_custom_pattern() {
        let filter = PiiFilter::new(&[r"CUST-\d{6}".to_string()]);
        let text = "Customer CUST-123456 filed a ticket";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("CUST-123456"));
        assert!(result.contains(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn test_invalid_custom_pattern_skipped() {
        // Invalid regex should not panic, just skip
        let filter = PiiFilter::new(&["[invalid".to_string()]);
        let text = "Normal text";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert_eq!(result, text);
    }

    // -- WhatsApp JID --

    #[test]
    fn test_redact_whatsapp_lid_jid() {
        let filter = make_filter();
        let text = "Sender JID: 393511083257@lid sent the message";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("393511083257"));
        assert!(!result.contains("@lid"));
        assert!(result.contains(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn test_redact_whatsapp_phone_jid() {
        let filter = make_filter();
        let text = "From 14155552671@s.whatsapp.net";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("14155552671"));
        assert!(!result.contains("@s.whatsapp.net"));
        assert!(result.contains(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn test_redact_whatsapp_group_jid() {
        let filter = make_filter();
        let text = "Group routed via 123456789012345@g.us today";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("123456789012345"));
        assert!(!result.contains("@g.us"));
        assert!(result.contains(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn test_jid_redact_is_atomic_not_partial() {
        // Regression: the `phone` regex used to greedy-match the digit prefix
        // of a WhatsApp JID, leaving the tail and `@<domain>` suffix in the
        // text (e.g. `[REDACTED]257@lid`). With the `whatsapp_jid` pattern
        // running first the full JID is replaced as a single unit, and no
        // partial digits or the `@<domain>` suffix survive in the output.
        let filter = make_filter();
        let text = "JID 393511083257@lid arrived";
        let result = filter.filter_message(text, &PrivacyMode::Redact);
        assert!(!result.contains("@lid"));
        assert!(!result.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_jid_redact_atomic_for_device_suffixed_jid() {
        // Regression (houko #5469 review): multi-device WhatsApp JIDs
        // (`<phone>:<device>@<domain>`) emitted by baileys / whatsmeow
        // weren't matched by the previous pattern (which required `\d`
        // immediately before `@`), so the `phone` rule partial-matched
        // the leading digits and left `:<device>@<domain>` visible.
        // Same partial-redact class this PR exists to eliminate.
        let filter = make_filter();
        for jid in [
            "15551234567:5@s.whatsapp.net",
            "15551234567:5@lid",
            "393511083257:12@s.whatsapp.net",
            "393511083257:1@c.us",
        ] {
            let text = format!("JID {jid} arrived");
            let result = filter.filter_message(&text, &PrivacyMode::Redact);
            assert!(
                !result.contains("@s.whatsapp.net")
                    && !result.contains("@lid")
                    && !result.contains("@c.us"),
                "device-suffixed JID `{jid}` must be atomically redacted, got: {result}"
            );
            assert!(
                !result.chars().any(|c| c.is_ascii_digit() || c == ':'),
                "no JID digit or device `:N` separator must survive, got: {result}"
            );
        }
    }

    #[test]
    fn test_pseudonymize_whatsapp_jid_is_stable() {
        let filter = make_filter();
        let text1 = "first: 393511083257@lid";
        let text2 = "second: 393511083257@lid";
        let r1 = filter.filter_message(text1, &PrivacyMode::Pseudonymize);
        let r2 = filter.filter_message(text2, &PrivacyMode::Pseudonymize);
        // Same JID must map to the same pseudonym across calls
        assert!(r1.contains("[Whatsapp_jid-A]"));
        assert!(r2.contains("[Whatsapp_jid-A]"));
        assert!(!r1.contains("393511083257"));
        assert!(!r2.contains("393511083257"));
    }

    #[test]
    fn test_jid_does_not_match_plain_email_or_number() {
        let filter = make_filter();
        // Plain email should still go through the `email` pattern, not `whatsapp_jid`.
        let email_text = "Contact alice@example.com";
        let email_result = filter.filter_message(email_text, &PrivacyMode::Redact);
        assert!(!email_result.contains("alice@example.com"));
        assert!(email_result.contains(REDACTED_PLACEHOLDER));
        // A bare phone number (no `@<wa-domain>`) should still match `phone`.
        let phone_text = "Call +1 415 555 2671";
        let phone_result = filter.filter_message(phone_text, &PrivacyMode::Redact);
        assert!(!phone_result.contains("415"));
        assert!(phone_result.contains(REDACTED_PLACEHOLDER));
    }

    // -- SenderContext filtering --

    #[test]
    fn test_filter_sender_context_redact() {
        let filter = make_filter();
        let sender = SenderContext {
            channel: "telegram".to_string(),
            user_id: "12345".to_string(),
            display_name: "Alice Smith".to_string(),
            is_group: false,
            was_mentioned: false,
            thread_id: None,
            account_id: Some("acct-1".to_string()),
            ..Default::default()
        };
        let result = filter.filter_sender_context(&sender, &PrivacyMode::Redact);
        assert_eq!(result.user_id, REDACTED_PLACEHOLDER);
        assert_eq!(result.display_name, REDACTED_PLACEHOLDER);
        assert_eq!(result.account_id, Some(REDACTED_PLACEHOLDER.to_string()));
        // Channel and is_group should be preserved
        assert_eq!(result.channel, "telegram");
        assert!(!result.is_group);
    }

    #[test]
    fn test_filter_sender_context_pseudonymize() {
        let filter = make_filter();
        let sender = SenderContext {
            channel: "discord".to_string(),
            user_id: "uid-999".to_string(),
            display_name: "Bob".to_string(),
            is_group: true,
            was_mentioned: false,
            thread_id: Some("thread-1".to_string()),
            account_id: None,
            ..Default::default()
        };
        let result = filter.filter_sender_context(&sender, &PrivacyMode::Pseudonymize);
        assert_ne!(result.user_id, "uid-999");
        assert_ne!(result.display_name, "Bob");
        assert!(result.display_name.starts_with('['));
        assert!(result.display_name.ends_with(']'));
        assert_eq!(result.channel, "discord");
        assert!(result.is_group);
    }

    #[test]
    fn test_filter_sender_context_off() {
        let filter = make_filter();
        let sender = SenderContext {
            channel: "slack".to_string(),
            user_id: "U123".to_string(),
            display_name: "Charlie".to_string(),
            is_group: false,
            was_mentioned: false,
            thread_id: None,
            account_id: None,
            ..Default::default()
        };
        let result = filter.filter_sender_context(&sender, &PrivacyMode::Off);
        assert_eq!(result.user_id, "U123");
        assert_eq!(result.display_name, "Charlie");
    }

    // -- bounded pseudonym map (#3489) --

    #[test]
    fn test_pseudonym_map_is_bounded() {
        let filter = make_filter();
        // Insert more than the cap of distinct emails — map size must stay
        // under the cap, oldest entries should be evicted.
        let cap = PSEUDONYM_MAP_CAP;
        let total = cap + 100;
        for i in 0..total {
            let text = format!("contact user{i}@example.com");
            filter.filter_message(&text, &PrivacyMode::Pseudonymize);
        }
        let len = filter.state.read().map.len();
        assert!(len <= cap, "pseudonym map should stay <= {cap}, got {len}");
        assert!(len >= cap - 10, "should be near the cap, got {len}");
    }

    // -- index_to_label --

    #[test]
    fn test_index_to_label() {
        assert_eq!(index_to_label(0), "A");
        assert_eq!(index_to_label(1), "B");
        assert_eq!(index_to_label(25), "Z");
        assert_eq!(index_to_label(26), "AA");
        assert_eq!(index_to_label(27), "AB");
    }

    /// Audit: pii-filter-regex-no-size-cap. Sources past
    /// `PII_REGEX_MAX_SOURCE_LEN` must be rejected at parse time,
    /// before the cap-limited builder runs.
    #[test]
    fn compile_pii_regex_rejects_oversize_source() {
        let pat = "a".repeat(PII_REGEX_MAX_SOURCE_LEN + 1);
        let err = compile_pii_regex(&pat).unwrap_err();
        assert!(
            err.contains("PII_REGEX_MAX_SOURCE_LEN"),
            "error message should name the source-length cap that fired: {err}"
        );
    }

    /// Sanity that the `RegexBuilder.size_limit` API is actually
    /// being threaded through. We can't reliably trigger the 1 MiB
    /// PII cap with a small fixed pattern (compiler internals
    /// vary), so this test feeds a known-large pattern through the
    /// builder with a tiny cap to confirm the bound fires AT ALL
    /// — guards against a future regex-crate refactor that
    /// silently drops the parameter. A repeated character class
    /// is sized predictably enough to hit a small cap.
    #[test]
    fn regex_builder_size_limit_path_actually_rejects() {
        let pat = format!("[a-z]{{{}}}", 100); // [a-z]{100}
        let err = RegexBuilder::new(&pat)
            .size_limit(64) // 64 bytes — far below the compiled NFA
            .build()
            .unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("size") || msg.contains("limit") || msg.contains("compiled"),
            "RegexBuilder must surface a size-limit error when the cap fires; got: {err}"
        );
    }

    #[test]
    fn compile_pii_regex_accepts_realistic_pii_patterns() {
        // The actual built-in PII patterns must all compile under
        // the cap — a regression that lowers the limit too far
        // would otherwise silently break the entire filter.
        for (label, pat) in BUILTIN_PATTERNS {
            assert!(
                compile_pii_regex(pat).is_ok(),
                "built-in PII pattern `{label}` must compile under the size cap"
            );
        }
        // And a typical operator-supplied US SSN-like pattern.
        assert!(compile_pii_regex(r"\b\d{3}-\d{2}-\d{4}\b").is_ok());
    }
}
