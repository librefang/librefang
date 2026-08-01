//! MoA privacy redaction.
//!
//! A focused, standalone redactor for the advisory view and traces. Unlike the
//! channel-facing [`crate::pii_filter`] (which is driven by `PrivacyConfig` and
//! `SenderContext`), this is a plain `&str -> String` used at MoA emission
//! boundaries per [`MoaPrivacyFilter`].
//!
//! Patterns target obvious secrets and PII: emails, formatted phone numbers
//! (delimited — NOT bare 10-digit runs, which would mangle ids/hashes), E.164
//! numbers, API keys, JWTs, private key blocks, and DB connection strings.

use regex_lite::Regex;
use std::sync::LazyLock;

/// Placeholder substituted for redacted spans.
const REDACTED: &str = "[REDACTED]";

/// A labelled redaction pattern.
#[allow(dead_code)] // `label` documents the pattern; used in compile warnings.
struct Pattern {
    label: &'static str,
    re: Regex,
}

static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    let specs: &[(&str, &str)] = &[
        // Private key blocks (PEM) — redact the whole block.
        (
            "private_key",
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        ),
        // JWTs: three base64url segments separated by dots.
        (
            "jwt",
            r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        ),
        // Common API-key / token assignment patterns.
        (
            "api_key",
            r#"(?i)(?:api[_-]?key|secret|token|passwd|password|authorization)\s*[:=]\s*["']?[A-Za-z0-9_\-./+]{16,}["']?"#,
        ),
        // Bearer tokens.
        ("bearer", r"(?i)\bbearer\s+[A-Za-z0-9_\-\.]{16,}"),
        // Provider-prefixed keys (sk-..., ghp_..., xoxb-..., AKIA...).
        (
            "provider_key",
            r"\b(?:sk-[A-Za-z0-9]{16,}|ghp_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16})\b",
        ),
        // DB connection strings.
        (
            "db_conn",
            r#"(?i)\b(?:postgres|postgresql|mysql|mongodb(?:\+srv)?|redis|amqp)://[^\s"']+"#,
        ),
        // Email addresses.
        ("email", r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
        // E.164 phone numbers (+country, 8-15 digits).
        ("e164", r"\+\d{8,15}\b"),
        // Formatted phone numbers: delimited digit groups, NOT bare 10-digit runs.
        (
            "phone_fmt",
            r"\b\d{3}[-.\s]\d{3}[-.\s]\d{4}\b|\(\d{3}\)\s*\d{3}[-.\s]\d{4}",
        ),
    ];

    specs
        .iter()
        .filter_map(|(label, pat)| match Regex::new(pat) {
            Ok(re) => Some(Pattern { label, re }),
            Err(e) => {
                tracing::warn!(label, error = %e, "MoA privacy pattern failed to compile");
                None
            }
        })
        .collect()
});

/// Redact obvious secrets and PII from `text`.
pub fn redact_pii(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in PATTERNS.iter() {
        result = pattern.re.replace_all(&result, REDACTED).to_string();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_email() {
        assert!(redact_pii("contact me at jane.doe@example.com now").contains(REDACTED));
    }

    #[test]
    fn redacts_jwt() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert!(redact_pii(jwt).contains(REDACTED));
    }

    #[test]
    fn redacts_api_key_assignment() {
        assert!(redact_pii("api_key=abcdef1234567890abcdef").contains(REDACTED));
    }

    #[test]
    fn redacts_provider_key() {
        assert!(redact_pii("use sk-abcdefghijklmnopqrstuvwxyz123456").contains(REDACTED));
    }

    #[test]
    fn redacts_db_connection_string() {
        assert!(redact_pii("url postgres://user:pass@host:5432/db").contains(REDACTED));
    }

    #[test]
    fn redacts_e164() {
        assert!(redact_pii("call +14155552671 today").contains(REDACTED));
    }

    #[test]
    fn redacts_formatted_phone() {
        assert!(redact_pii("tel 415-555-2671 ok").contains(REDACTED));
        assert!(redact_pii("tel (415) 555-2671 ok").contains(REDACTED));
    }

    #[test]
    fn does_not_redact_bare_ten_digit_run() {
        // A bare 10-digit run (e.g. an id) must survive.
        let out = redact_pii("order 1234567890 placed");
        assert!(out.contains("1234567890"), "got: {out}");
    }

    #[test]
    fn leaves_clean_text_untouched() {
        let text = "The quick brown fox jumps over the lazy dog.";
        assert_eq!(redact_pii(text), text);
    }
}
