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

use librefang_channels::SenderContext;
use librefang_types::config::PrivacyMode;
use regex_lite::Regex;
use std::collections::HashMap;
use std::sync::Mutex;

/// Placeholder used in `Redact` mode.
const REDACTED_PLACEHOLDER: &str = "[REDACTED]";

/// Built-in PII regex patterns (label, pattern).
///
/// These are compiled once at `PiiFilter` construction time.
const BUILTIN_PATTERNS: &[(&str, &str)] = &[
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
pub struct PiiFilter {
    /// Compiled built-in + custom regex patterns with their labels.
    patterns: Vec<(String, Regex)>,
    /// Pseudonym mapping: original PII value -> pseudonym label.
    /// Protected by Mutex for interior mutability (pseudonym map grows over time).
    pseudonym_map: Mutex<HashMap<String, String>>,
    /// Counter for generating sequential pseudonyms per category.
    pseudonym_counters: Mutex<HashMap<String, u32>>,
}

impl PiiFilter {
    /// Create a new PII filter with built-in patterns and optional custom patterns.
    ///
    /// Invalid custom regex patterns are logged and skipped.
    pub fn new(custom_patterns: &[String]) -> Self {
        let mut patterns = Vec::with_capacity(BUILTIN_PATTERNS.len() + custom_patterns.len());

        for (label, pat) in BUILTIN_PATTERNS {
            match Regex::new(pat) {
                Ok(re) => patterns.push((label.to_string(), re)),
                Err(e) => {
                    tracing::warn!(pattern = pat, error = %e, "Failed to compile built-in PII pattern");
                }
            }
        }

        for (i, pat) in custom_patterns.iter().enumerate() {
            match Regex::new(pat) {
                Ok(re) => patterns.push((format!("custom_{i}"), re)),
                Err(e) => {
                    tracing::warn!(pattern = pat, error = %e, "Failed to compile custom PII pattern — skipping");
                }
            }
        }

        Self {
            patterns,
            pseudonym_map: Mutex::new(HashMap::new()),
            pseudonym_counters: Mutex::new(HashMap::new()),
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
                thread_id: sender.thread_id.clone(),
                account_id: sender
                    .account_id
                    .as_ref()
                    .map(|_| REDACTED_PLACEHOLDER.to_string()),
            },
            PrivacyMode::Pseudonymize => {
                let pseudo_name = self.get_or_create_pseudonym(&sender.display_name, "user");
                let pseudo_id = self.get_or_create_pseudonym(&sender.user_id, "user_id");
                SenderContext {
                    channel: sender.channel.clone(),
                    user_id: pseudo_id,
                    display_name: pseudo_name,
                    is_group: sender.is_group,
                    thread_id: sender.thread_id.clone(),
                    account_id: sender
                        .account_id
                        .as_ref()
                        .map(|id| self.get_or_create_pseudonym(id, "account")),
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
    fn get_or_create_pseudonym(&self, value: &str, category: &str) -> String {
        let mut map = self.pseudonym_map.lock().unwrap_or_else(|e| e.into_inner());

        // Key includes category to avoid collisions between different PII types
        let key = format!("{category}:{value}");
        if let Some(existing) = map.get(&key) {
            return existing.clone();
        }

        let mut counters = self
            .pseudonym_counters
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let counter = counters.entry(category.to_string()).or_insert(0);
        let letter = index_to_label(*counter);
        *counter += 1;

        let label = capitalize_category(category);
        let pseudonym = format!("[{label}-{letter}]");
        map.insert(key, pseudonym.clone());
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

    // -- SenderContext filtering --

    #[test]
    fn test_filter_sender_context_redact() {
        let filter = make_filter();
        let sender = SenderContext {
            channel: "telegram".to_string(),
            user_id: "12345".to_string(),
            display_name: "Alice Smith".to_string(),
            is_group: false,
            thread_id: None,
            account_id: Some("acct-1".to_string()),
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
            thread_id: Some("thread-1".to_string()),
            account_id: None,
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
            thread_id: None,
            account_id: None,
        };
        let result = filter.filter_sender_context(&sender, &PrivacyMode::Off);
        assert_eq!(result.user_id, "U123");
        assert_eq!(result.display_name, "Charlie");
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
}
