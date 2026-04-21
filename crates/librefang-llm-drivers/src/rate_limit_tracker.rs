//! Rate limit header tracking for LLM provider responses.
//!
//! Parses `x-ratelimit-*` and `anthropic-ratelimit-*` response headers into
//! structured buckets and emits `tracing::warn!` when any bucket exceeds 80%
//! of its capacity.
//!
//! ## Supported header schemas
//!
//! ### OpenAI / Groq / OpenRouter / Nous Portal format (12 headers):
//! ```text
//! x-ratelimit-limit-requests          RPM cap
//! x-ratelimit-limit-requests-1h       RPH cap
//! x-ratelimit-limit-tokens            TPM cap
//! x-ratelimit-limit-tokens-1h         TPH cap
//! x-ratelimit-remaining-requests      requests left this minute
//! x-ratelimit-remaining-requests-1h   requests left this hour
//! x-ratelimit-remaining-tokens        tokens left this minute
//! x-ratelimit-remaining-tokens-1h     tokens left this hour
//! x-ratelimit-reset-requests          seconds until minute request window resets
//! x-ratelimit-reset-requests-1h       seconds until hour request window resets
//! x-ratelimit-reset-tokens            seconds until minute token window resets
//! x-ratelimit-reset-tokens-1h         seconds until hour token window resets
//! ```
//!
//! ### Anthropic-specific format:
//! ```text
//! anthropic-ratelimit-requests-limit
//! anthropic-ratelimit-requests-remaining
//! anthropic-ratelimit-requests-reset
//! anthropic-ratelimit-tokens-limit
//! anthropic-ratelimit-tokens-remaining
//! anthropic-ratelimit-tokens-reset
//! anthropic-ratelimit-input-tokens-limit
//! anthropic-ratelimit-input-tokens-remaining
//! anthropic-ratelimit-input-tokens-reset
//! ```

use std::time::Instant;

/// One rate-limit window (e.g. requests-per-minute or tokens-per-hour).
#[derive(Debug, Clone)]
pub struct RateLimitBucket {
    /// Maximum allowed in this window. 0 means no data.
    pub limit: u64,
    /// How many remain in this window.
    pub remaining: u64,
    /// How many seconds until the window resets (as reported by the provider).
    pub reset_after_secs: f64,
    /// Monotonic timestamp of when this bucket was captured, used to calculate
    /// the time-adjusted remaining seconds.
    pub captured_at: Instant,
}

impl Default for RateLimitBucket {
    fn default() -> Self {
        Self {
            limit: 0,
            remaining: 0,
            reset_after_secs: 0.0,
            captured_at: Instant::now(),
        }
    }
}

impl RateLimitBucket {
    /// Fraction of the window already consumed: `(limit - remaining) / limit`.
    ///
    /// Returns `0.0` when `limit` is zero (no data).
    pub fn usage_ratio(&self) -> f64 {
        if self.limit == 0 {
            return 0.0;
        }
        let used = self.limit.saturating_sub(self.remaining);
        used as f64 / self.limit as f64
    }

    /// Seconds remaining until the window resets, adjusted for elapsed wall time.
    pub fn remaining_secs(&self) -> f64 {
        let elapsed = self.captured_at.elapsed().as_secs_f64();
        (self.reset_after_secs - elapsed).max(0.0)
    }

    /// Returns `true` when more than 80% of this bucket has been consumed.
    pub fn is_warning(&self) -> bool {
        self.usage_ratio() > 0.80
    }

    /// Whether this bucket has any data (limit > 0).
    pub fn has_data(&self) -> bool {
        self.limit > 0
    }
}

/// Full rate-limit snapshot parsed from a single LLM response.
#[derive(Debug, Clone, Default)]
pub struct RateLimitSnapshot {
    /// Requests-per-minute window.
    pub requests_per_minute: RateLimitBucket,
    /// Requests-per-hour window.
    pub requests_per_hour: RateLimitBucket,
    /// Tokens-per-minute window.
    pub tokens_per_minute: RateLimitBucket,
    /// Tokens-per-hour window.
    pub tokens_per_hour: RateLimitBucket,
    /// Input-tokens-per-minute window (Anthropic-specific).
    pub input_tokens_per_minute: RateLimitBucket,
}

impl RateLimitSnapshot {
    /// Parse a [`RateLimitSnapshot`] from HTTP response headers.
    ///
    /// Supports both the OpenAI/Groq `x-ratelimit-*` schema and the
    /// Anthropic `anthropic-ratelimit-*` schema. Headers are matched
    /// case-insensitively per RFC 7230.
    ///
    /// Returns `None` when no recognisable rate-limit headers are present.
    pub fn from_headers(headers: &reqwest::header::HeaderMap) -> Option<Self> {
        // Collect all header names into a lowercase map so we can do O(1) lookups
        // without caring about capitalisation.
        let lowered: std::collections::HashMap<String, &str> = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_ascii_lowercase(), v))
            })
            .collect();

        // Quick guard: at least one rate-limit header must be present.
        let has_any = lowered
            .keys()
            .any(|k| k.starts_with("x-ratelimit-") || k.starts_with("anthropic-ratelimit-"));
        if !has_any {
            return None;
        }

        let now = Instant::now();

        // ── Helper closures ───────────────────────────────────────────────

        let get_u64 = |key: &str| -> u64 {
            lowered
                .get(key)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(|f| f as u64)
                .unwrap_or(0)
        };

        let get_f64 = |key: &str| -> f64 {
            lowered
                .get(key)
                .and_then(|v| parse_reset_value(v.trim()))
                .unwrap_or(0.0)
        };

        let make_bucket = |limit: u64, remaining: u64, reset: f64| -> RateLimitBucket {
            RateLimitBucket {
                limit,
                remaining,
                reset_after_secs: reset,
                captured_at: now,
            }
        };

        // ── OpenAI / Groq / OpenRouter / Nous Portal schema ───────────────

        // Requests per minute
        let rpm = make_bucket(
            get_u64("x-ratelimit-limit-requests"),
            get_u64("x-ratelimit-remaining-requests"),
            get_f64("x-ratelimit-reset-requests"),
        );

        // Requests per hour
        let rph = make_bucket(
            get_u64("x-ratelimit-limit-requests-1h"),
            get_u64("x-ratelimit-remaining-requests-1h"),
            get_f64("x-ratelimit-reset-requests-1h"),
        );

        // Tokens per minute
        let tpm = make_bucket(
            get_u64("x-ratelimit-limit-tokens"),
            get_u64("x-ratelimit-remaining-tokens"),
            get_f64("x-ratelimit-reset-tokens"),
        );

        // Tokens per hour
        let tph = make_bucket(
            get_u64("x-ratelimit-limit-tokens-1h"),
            get_u64("x-ratelimit-remaining-tokens-1h"),
            get_f64("x-ratelimit-reset-tokens-1h"),
        );

        // ── Anthropic schema (overrides x-ratelimit-* when present) ──────
        //
        // Anthropic uses a different naming convention:
        //   anthropic-ratelimit-{resource}-{limit|remaining|reset}
        // where resource is "requests", "tokens", or "input-tokens".

        let anthropic_rpm = {
            let limit = get_u64("anthropic-ratelimit-requests-limit");
            if limit > 0 {
                make_bucket(
                    limit,
                    get_u64("anthropic-ratelimit-requests-remaining"),
                    get_f64("anthropic-ratelimit-requests-reset"),
                )
            } else {
                rpm.clone()
            }
        };

        let anthropic_tpm = {
            let limit = get_u64("anthropic-ratelimit-tokens-limit");
            if limit > 0 {
                make_bucket(
                    limit,
                    get_u64("anthropic-ratelimit-tokens-remaining"),
                    get_f64("anthropic-ratelimit-tokens-reset"),
                )
            } else {
                tpm.clone()
            }
        };

        // Input tokens (Anthropic-only concept)
        let input_tpm = make_bucket(
            get_u64("anthropic-ratelimit-input-tokens-limit"),
            get_u64("anthropic-ratelimit-input-tokens-remaining"),
            get_f64("anthropic-ratelimit-input-tokens-reset"),
        );

        Some(RateLimitSnapshot {
            requests_per_minute: anthropic_rpm,
            requests_per_hour: rph,
            tokens_per_minute: anthropic_tpm,
            tokens_per_hour: tph,
            input_tokens_per_minute: input_tpm,
        })
    }

    /// Returns `true` if any tracked bucket is in the warning zone (>80% consumed).
    pub fn has_warning(&self) -> bool {
        self.requests_per_minute.is_warning()
            || self.requests_per_hour.is_warning()
            || self.tokens_per_minute.is_warning()
            || self.tokens_per_hour.is_warning()
            || self.input_tokens_per_minute.is_warning()
    }

    /// Format the snapshot as a multi-line human-readable string with ASCII
    /// progress bars and time-until-reset countdowns.
    ///
    /// Example output:
    /// ```text
    /// Rate Limits:
    ///
    ///   Requests/min   [████████░░░░░░░░░░░░]  40.0%  400/1000 used  (600 left, resets in 42s)
    ///   Requests/hr    (no data)
    ///
    ///   Tokens/min     [██████████████░░░░░░]  70.0%  70.0K/100.0K used  (30.0K left, resets in 42s)
    ///   Tokens/hr      (no data)
    ///   Input tok/min  (no data)
    /// ```
    pub fn display(&self) -> String {
        let buckets: &[(&str, &RateLimitBucket)] = &[
            ("Requests/min ", &self.requests_per_minute),
            ("Requests/hr  ", &self.requests_per_hour),
            ("Tokens/min   ", &self.tokens_per_minute),
            ("Tokens/hr    ", &self.tokens_per_hour),
            ("Input tok/min", &self.input_tokens_per_minute),
        ];

        let mut lines = vec!["Rate Limits:".to_string(), String::new()];

        for (label, bucket) in buckets {
            lines.push(fmt_bucket_line(label, bucket));
        }

        // Warnings section
        let warnings: Vec<String> = buckets
            .iter()
            .filter(|(_, b)| b.is_warning())
            .map(|(label, b)| {
                format!(
                    "  ⚠  {} at {:.0}% — resets in {}",
                    label.trim(),
                    b.usage_ratio() * 100.0,
                    fmt_seconds(b.remaining_secs()),
                )
            })
            .collect();

        if !warnings.is_empty() {
            lines.push(String::new());
            lines.extend(warnings);
        }

        lines.join("\n")
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Parse a reset value that may be expressed as:
/// - A plain number of seconds (`"42.5"`)
/// - An ISO 8601 duration (`"PT42.5S"`, `"PT1M30S"`, `"PT1H"`)
fn parse_reset_value(s: &str) -> Option<f64> {
    // Plain numeric seconds
    if let Ok(v) = s.parse::<f64>() {
        return Some(v);
    }

    // ISO 8601 duration — minimal subset: PT[Nh][Nm][Ns] (no date part)
    if let Some(rest) = s.strip_prefix("PT").or_else(|| s.strip_prefix("pt")) {
        let mut secs = 0.0f64;
        let mut current = String::new();
        for ch in rest.chars() {
            match ch {
                '0'..='9' | '.' => current.push(ch),
                'H' | 'h' => {
                    secs += current.parse::<f64>().unwrap_or(0.0) * 3600.0;
                    current.clear();
                }
                'M' | 'm' => {
                    secs += current.parse::<f64>().unwrap_or(0.0) * 60.0;
                    current.clear();
                }
                'S' | 's' => {
                    secs += current.parse::<f64>().unwrap_or(0.0);
                    current.clear();
                }
                _ => {}
            }
        }
        return Some(secs);
    }

    None
}

/// Human-readable count: `7_999_856` → `"8.0M"`, `33_599` → `"33.6K"`, `799` → `"799"`.
fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Human-readable duration: `"58s"`, `"2m 14s"`, `"1h 2m"`.
fn fmt_seconds(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        let m = s / 60;
        let sec = s % 60;
        if sec == 0 {
            format!("{m}m")
        } else {
            format!("{m}m {sec}s")
        }
    } else {
        let h = s / 3600;
        let m = (s % 3600) / 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m}m")
        }
    }
}

/// ASCII progress bar `[████████░░░░░░░░░░░░]` for a ratio in `[0.0, 1.0]`.
fn ascii_bar(ratio: f64, width: usize) -> String {
    let filled = ((ratio.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    let empty = width - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

/// Format a single bucket as one display line.
fn fmt_bucket_line(label: &str, bucket: &RateLimitBucket) -> String {
    if !bucket.has_data() {
        return format!("  {label}  (no data)");
    }

    let ratio = bucket.usage_ratio();
    let used = bucket.limit.saturating_sub(bucket.remaining);
    let bar = ascii_bar(ratio, 20);
    let reset = fmt_seconds(bucket.remaining_secs());

    format!(
        "  {label}  {bar} {:5.1}%  {}/{} used  ({} left, resets in {})",
        ratio * 100.0,
        fmt_count(used),
        fmt_count(bucket.limit),
        fmt_count(bucket.remaining),
        reset,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    use std::str::FromStr;

    fn headers_from_pairs(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                HeaderName::from_str(k).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[test]
    fn test_no_rate_limit_headers_returns_none() {
        let headers = headers_from_pairs(&[("content-type", "application/json")]);
        assert!(RateLimitSnapshot::from_headers(&headers).is_none());
    }

    #[test]
    fn test_openai_format_parsed() {
        let headers = headers_from_pairs(&[
            ("x-ratelimit-limit-requests", "1000"),
            ("x-ratelimit-remaining-requests", "600"),
            ("x-ratelimit-reset-requests", "42"),
            ("x-ratelimit-limit-tokens", "100000"),
            ("x-ratelimit-remaining-tokens", "30000"),
            ("x-ratelimit-reset-tokens", "35"),
        ]);
        let snap = RateLimitSnapshot::from_headers(&headers).expect("should parse");
        assert_eq!(snap.requests_per_minute.limit, 1000);
        assert_eq!(snap.requests_per_minute.remaining, 600);
        assert!((snap.requests_per_minute.reset_after_secs - 42.0).abs() < 0.01);
        assert_eq!(snap.tokens_per_minute.limit, 100_000);
        assert_eq!(snap.tokens_per_minute.remaining, 30_000);
    }

    #[test]
    fn test_anthropic_format_overrides_x_ratelimit() {
        // Anthropic headers should win over x-ratelimit-* when both are present.
        let headers = headers_from_pairs(&[
            ("x-ratelimit-limit-requests", "500"),
            ("anthropic-ratelimit-requests-limit", "2000"),
            ("anthropic-ratelimit-requests-remaining", "1800"),
            ("anthropic-ratelimit-requests-reset", "30"),
        ]);
        let snap = RateLimitSnapshot::from_headers(&headers).expect("should parse");
        assert_eq!(
            snap.requests_per_minute.limit, 2000,
            "anthropic header should override x-ratelimit"
        );
        assert_eq!(snap.requests_per_minute.remaining, 1800);
    }

    #[test]
    fn test_input_tokens_bucket_parsed() {
        let headers = headers_from_pairs(&[
            ("anthropic-ratelimit-input-tokens-limit", "50000"),
            ("anthropic-ratelimit-input-tokens-remaining", "10000"),
            ("anthropic-ratelimit-input-tokens-reset", "60"),
        ]);
        let snap = RateLimitSnapshot::from_headers(&headers).expect("should parse");
        assert_eq!(snap.input_tokens_per_minute.limit, 50_000);
        assert_eq!(snap.input_tokens_per_minute.remaining, 10_000);
    }

    #[test]
    fn test_usage_ratio() {
        let bucket = RateLimitBucket {
            limit: 1000,
            remaining: 200,
            ..Default::default()
        };
        assert!((bucket.usage_ratio() - 0.8).abs() < 1e-9);
    }

    #[test]
    fn test_is_warning_threshold() {
        let not_warn = RateLimitBucket {
            limit: 100,
            remaining: 20, // 80% used — NOT over threshold
            ..Default::default()
        };
        assert!(!not_warn.is_warning());

        let warn = RateLimitBucket {
            limit: 100,
            remaining: 19, // 81% used — over threshold
            ..Default::default()
        };
        assert!(warn.is_warning());
    }

    #[test]
    fn test_zero_limit_bucket_usage_is_zero() {
        let b = RateLimitBucket::default();
        assert_eq!(b.usage_ratio(), 0.0);
        assert!(!b.is_warning());
    }

    #[test]
    fn test_has_warning_false_when_all_ok() {
        let snap = RateLimitSnapshot::default();
        assert!(!snap.has_warning());
    }

    #[test]
    fn test_has_warning_true_when_one_hot() {
        let mut snap = RateLimitSnapshot::default();
        snap.tokens_per_minute = RateLimitBucket {
            limit: 100,
            remaining: 5, // 95% used
            ..Default::default()
        };
        assert!(snap.has_warning());
    }

    #[test]
    fn test_parse_reset_value_plain_number() {
        assert!((parse_reset_value("42.5").unwrap() - 42.5).abs() < 1e-9);
    }

    #[test]
    fn test_parse_reset_value_iso_duration_seconds() {
        assert!((parse_reset_value("PT42.5S").unwrap() - 42.5).abs() < 1e-9);
    }

    #[test]
    fn test_parse_reset_value_iso_duration_minutes() {
        assert!((parse_reset_value("PT1M30S").unwrap() - 90.0).abs() < 1e-9);
    }

    #[test]
    fn test_parse_reset_value_iso_duration_hours() {
        assert!((parse_reset_value("PT1H").unwrap() - 3600.0).abs() < 1e-9);
    }

    #[test]
    fn test_parse_reset_value_iso_duration_mixed() {
        assert!((parse_reset_value("PT1H2M3S").unwrap() - 3723.0).abs() < 1e-9);
    }

    #[test]
    fn test_fmt_count() {
        assert_eq!(fmt_count(799), "799");
        assert_eq!(fmt_count(1_500), "1.5K");
        assert_eq!(fmt_count(33_599), "33.6K");
        assert_eq!(fmt_count(7_999_856), "8.0M");
    }

    #[test]
    fn test_fmt_seconds() {
        assert_eq!(fmt_seconds(0.0), "0s");
        assert_eq!(fmt_seconds(58.0), "58s");
        assert_eq!(fmt_seconds(90.0), "1m 30s");
        assert_eq!(fmt_seconds(120.0), "2m");
        assert_eq!(fmt_seconds(3661.0), "1h 1m");
        assert_eq!(fmt_seconds(7200.0), "2h");
    }

    #[test]
    fn test_display_with_data() {
        let mut snap = RateLimitSnapshot::default();
        snap.requests_per_minute = RateLimitBucket {
            limit: 1000,
            remaining: 600,
            reset_after_secs: 42.0,
            captured_at: Instant::now(),
        };
        let s = snap.display();
        assert!(s.contains("Rate Limits:"));
        assert!(s.contains("Requests/min"));
        assert!(s.contains("40.0%"));
        assert!(s.contains("400/1.0K"));
    }

    #[test]
    fn test_display_shows_warning_section() {
        let mut snap = RateLimitSnapshot::default();
        snap.tokens_per_minute = RateLimitBucket {
            limit: 100,
            remaining: 5, // 95% used
            reset_after_secs: 20.0,
            captured_at: Instant::now(),
        };
        let s = snap.display();
        assert!(s.contains('⚠'));
        assert!(s.contains("95%"));
    }

    #[test]
    fn test_one_hour_buckets_parsed() {
        let headers = headers_from_pairs(&[
            ("x-ratelimit-limit-requests-1h", "10000"),
            ("x-ratelimit-remaining-requests-1h", "9000"),
            ("x-ratelimit-reset-requests-1h", "3540"),
            ("x-ratelimit-limit-tokens-1h", "5000000"),
            ("x-ratelimit-remaining-tokens-1h", "4000000"),
            ("x-ratelimit-reset-tokens-1h", "3540"),
        ]);
        let snap = RateLimitSnapshot::from_headers(&headers).expect("should parse");
        assert_eq!(snap.requests_per_hour.limit, 10_000);
        assert_eq!(snap.tokens_per_hour.limit, 5_000_000);
    }
}
