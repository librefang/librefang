//! `Retry-After` HTTP header parsing.
//!
//! Per RFC 7231 §7.1.3 the `Retry-After` header value can be either:
//!
//! 1. **Delta-seconds** — a non-negative integer giving the number of
//!    seconds to wait, e.g. `Retry-After: 120`.
//! 2. **HTTP-date** — an absolute timestamp in IMF-fixdate format,
//!    e.g. `Retry-After: Wed, 21 Oct 2015 07:28:00 GMT`.
//!
//! Real LLM providers send delta-seconds in practice, but the spec
//! permits either form and a few middleware layers / CDNs do emit
//! HTTP-date.  Drivers that only handle the delta-seconds form silently
//! fall back to a hardcoded default whenever the server returns an
//! HTTP-date, which defeats the rate-limit signal entirely.
//!
//! These helpers handle both forms and return a fallback when the
//! header is missing, malformed, or in the past.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::header::HeaderMap;

/// Parse the `Retry-After` header into a [`Duration`].
///
/// Returns `Duration::from_millis(fallback_ms)` when the header is
/// missing, not valid UTF-8, or fails to parse as either delta-seconds
/// or an HTTP-date.
///
/// Returns [`Duration::ZERO`] when the header value is the literal
/// `0`, an HTTP-date in the past, or `fallback_ms` is `0`. Callers
/// that need to distinguish "wait 0 ms" from "no signal" should use
/// [`duration_to_ms_or_fallback`] to collapse the zero case back to a
/// caller-supplied default.
pub fn parse_retry_after(headers: &HeaderMap, fallback_ms: u64) -> Duration {
    parse_retry_after_value(headers.get(reqwest::header::RETRY_AFTER))
        .unwrap_or_else(|| Duration::from_millis(fallback_ms))
}

/// Same as [`parse_retry_after`] but returns the result in
/// milliseconds — a convenience for the
/// `LlmError::RateLimited { retry_after_ms, .. }` field.
pub fn parse_retry_after_ms(headers: &HeaderMap, fallback_ms: u64) -> u64 {
    let d = parse_retry_after(headers, fallback_ms);
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Convert a `Duration` (typically returned by [`parse_retry_after`])
/// into a `u64` millisecond value, substituting `fallback_ms` whenever
/// the duration is zero.
///
/// `LlmError::RateLimited.retry_after_ms == 0` is interpreted by the
/// failover layer as "no Retry-After signal at all" rather than "wait
/// zero ms" (see `LlmError::failover_reason` in
/// `librefang-llm-driver`). For drivers that have already exhausted
/// their internal retry loop and want to surface a sensible
/// human-facing default, this helper collapses both the
/// missing-header case and the explicit-zero case (delta-seconds `0`,
/// past-dated HTTP-date) into the same fallback.
pub fn duration_to_ms_or_fallback(d: Duration, fallback_ms: u64) -> u64 {
    if d.is_zero() {
        return fallback_ms;
    }
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn parse_retry_after_value(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let raw = value?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }

    // Form 1: delta-seconds (a non-negative decimal integer).
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }

    // Form 2: HTTP-date (RFC 7231 IMF-fixdate ≈ RFC 2822 date).
    if let Ok(when) = DateTime::parse_from_rfc2822(raw) {
        let now = Utc::now();
        let when_utc = when.with_timezone(&Utc);
        if when_utc > now {
            let delta = when_utc - now;
            // chrono::Duration → std::time::Duration (saturating on
            // negative / overflow values).
            if let Ok(std) = delta.to_std() {
                return Some(std);
            }
        } else {
            // Past timestamp — server is telling us we may retry now.
            return Some(Duration::ZERO);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use reqwest::header::{HeaderValue, RETRY_AFTER};

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn delta_seconds_parsed() {
        let h = headers_with("120");
        assert_eq!(parse_retry_after(&h, 5000), Duration::from_secs(120));
        assert_eq!(parse_retry_after_ms(&h, 5000), 120_000);
    }

    #[test]
    fn delta_seconds_zero() {
        let h = headers_with("0");
        assert_eq!(parse_retry_after(&h, 5000), Duration::ZERO);
    }

    #[test]
    fn http_date_in_future_parsed() {
        let future = Utc::now() + ChronoDuration::seconds(90);
        // RFC 2822 format matches IMF-fixdate when offset is +0000.
        let formatted = future.to_rfc2822();
        let h = headers_with(&formatted);
        let d = parse_retry_after(&h, 5000);
        // Allow ±5s slack for clock drift between the two `Utc::now()`
        // calls in this test and the parser.
        assert!(
            d >= Duration::from_secs(85) && d <= Duration::from_secs(95),
            "expected ~90s, got {d:?}"
        );
    }

    #[test]
    fn http_date_in_past_returns_zero() {
        let past = Utc::now() - ChronoDuration::seconds(60);
        let h = headers_with(&past.to_rfc2822());
        assert_eq!(parse_retry_after(&h, 5000), Duration::ZERO);
    }

    #[test]
    fn missing_header_uses_fallback() {
        let h = HeaderMap::new();
        assert_eq!(parse_retry_after(&h, 5000), Duration::from_millis(5000));
        assert_eq!(parse_retry_after_ms(&h, 5000), 5000);
    }

    #[test]
    fn invalid_value_uses_fallback() {
        let h = headers_with("not a real value");
        assert_eq!(parse_retry_after(&h, 5000), Duration::from_millis(5000));
    }

    #[test]
    fn empty_value_uses_fallback() {
        let h = headers_with("   ");
        assert_eq!(parse_retry_after(&h, 5000), Duration::from_millis(5000));
    }

    #[test]
    fn negative_delta_seconds_uses_fallback() {
        // A leading "-" makes u64::parse fail and the value is not a
        // valid HTTP-date, so we fall back.
        let h = headers_with("-30");
        assert_eq!(parse_retry_after(&h, 5000), Duration::from_millis(5000));
    }

    #[test]
    fn imf_fixdate_literal_in_past() {
        // The exact example from RFC 7231 — definitely in the past.
        let h = headers_with("Wed, 21 Oct 2015 07:28:00 GMT");
        assert_eq!(parse_retry_after(&h, 5000), Duration::ZERO);
    }

    #[test]
    fn duration_to_ms_or_fallback_zero_uses_fallback() {
        assert_eq!(duration_to_ms_or_fallback(Duration::ZERO, 5000), 5000);
        assert_eq!(duration_to_ms_or_fallback(Duration::ZERO, 0), 0);
    }

    #[test]
    fn duration_to_ms_or_fallback_nonzero_passes_through() {
        assert_eq!(
            duration_to_ms_or_fallback(Duration::from_millis(120_000), 5000),
            120_000
        );
        assert_eq!(
            duration_to_ms_or_fallback(Duration::from_millis(1), 5000),
            1
        );
    }

    #[test]
    fn duration_to_ms_or_fallback_saturates_overflow() {
        // Durations beyond u64::MAX milliseconds saturate rather than
        // wrap or panic.
        assert_eq!(
            duration_to_ms_or_fallback(Duration::from_secs(u64::MAX), 5000),
            u64::MAX
        );
    }
}
