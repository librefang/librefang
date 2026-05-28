//! HTML sanitiser for Telegram-bound text.
//!
//! Mirrors the Python adapter's `sanitize_telegram_html`: drop tags not on the allowlist (or escape them so the parser sees literal `<foo>` text), normalise `<a>`'s href to a safe scheme, keep `<code class="...">`, balance unclosed tags at end-of-input.
//!
//! Telegram accepts only this tag set per <https://core.telegram.org/bots/api#html-style>:
//! `<b>`, `<i>`, `<u>`, `<s>`, `<em>`, `<strong>`, `<a>`, `<code>`, `<pre>`, `<blockquote>`, `<tg-spoiler>`, `<tg-emoji>`.

use once_cell::sync::Lazy;
use regex::Regex;

const ALLOWED_TAGS: &[&str] = &[
    "b",
    "i",
    "u",
    "s",
    "em",
    "strong",
    "a",
    "code",
    "pre",
    "blockquote",
    "tg-spoiler",
    "tg-emoji",
];
const ALLOWED_HREF_SCHEMES: &[&str] = &["https:", "http:", "mailto:", "tg:"];

static RE_TAG: Lazy<Regex> = Lazy::new(|| {
    // Match either an opening tag `<name attrs>` or a closing tag `</name>` or a self-closing variant.
    Regex::new(r"<(/?)([a-zA-Z][a-zA-Z0-9-]*)([^>]*)>").expect("tag regex")
});

static RE_ATTR: Lazy<Regex> = Lazy::new(|| {
    // Parse `key="value"` or `key='value'` from the attribute-string portion of a tag.
    Regex::new(r#"([a-zA-Z][a-zA-Z0-9-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).expect("attr regex")
});

fn href_is_safe(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    ALLOWED_HREF_SCHEMES.iter().any(|s| lower.starts_with(s))
}

fn escape_attr_value(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn rebuild_attrs(tag_name: &str, attr_str: &str) -> Option<String> {
    let mut kept = Vec::new();
    for caps in RE_ATTR.captures_iter(attr_str) {
        let key = caps.get(1).unwrap().as_str().to_ascii_lowercase();
        let value = caps
            .get(2)
            .or_else(|| caps.get(3))
            .map(|m| m.as_str())
            .unwrap_or("");
        match (tag_name, key.as_str()) {
            ("a", "href") => {
                if href_is_safe(value) {
                    kept.push(format!(" href=\"{}\"", escape_attr_value(value)));
                } else {
                    return None;
                }
            }
            ("code", "class") => {
                kept.push(format!(" class=\"{}\"", escape_attr_value(value)));
            }
            ("tg-emoji", "emoji-id") => {
                kept.push(format!(" emoji-id=\"{}\"", escape_attr_value(value)));
            }
            _ => {}
        }
    }
    Some(kept.join(""))
}

/// Defense-in-depth sanitiser. Returns Telegram-safe HTML with disallowed tags either dropped or HTML-escaped, and any tags left open at the end are closed in reverse order.
pub fn sanitize_telegram_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut stack: Vec<String> = Vec::new();
    let mut cursor = 0usize;

    for m in RE_TAG.find_iter(text) {
        // Append any literal text between the last cursor and this tag.
        if m.start() > cursor {
            out.push_str(&text[cursor..m.start()]);
        }
        cursor = m.end();

        let captures = RE_TAG.captures(m.as_str()).unwrap();
        let closing = captures
            .get(1)
            .map(|s| !s.as_str().is_empty())
            .unwrap_or(false);
        let tag = captures.get(2).unwrap().as_str().to_ascii_lowercase();
        let attrs = captures.get(3).map(|s| s.as_str()).unwrap_or("");

        if !ALLOWED_TAGS.contains(&tag.as_str()) {
            // Drop the tag entirely (no inline escape — the surrounding text already had its `<` / `>` HTML-escaped by the caller / markdown converter).
            continue;
        }

        if closing {
            // Find the most recent matching open tag and close down through it.
            if let Some(pos) = stack.iter().rposition(|t| t == &tag) {
                // Close every tag above the match (drop them silently — sanitiser priority is "produce valid HTML" not "preserve nesting depth").
                for unclosed in stack.drain(pos..).rev() {
                    out.push_str("</");
                    out.push_str(&unclosed);
                    out.push('>');
                }
            }
            // No matching open tag → drop the closing tag.
            continue;
        }

        match rebuild_attrs(&tag, attrs) {
            Some(rebuilt) => {
                out.push('<');
                out.push_str(&tag);
                out.push_str(&rebuilt);
                out.push('>');
                stack.push(tag);
            }
            None => {
                // Tag's required attribute (e.g. <a href>) failed the safety check → drop the tag.
            }
        }
    }

    // Trailing literal text.
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }

    // Auto-close anything still open.
    for unclosed in stack.into_iter().rev() {
        out.push_str("</");
        out.push_str(&unclosed);
        out.push('>');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_allowed_tag() {
        let s = sanitize_telegram_html("<b>hi</b>");
        assert_eq!(s, "<b>hi</b>");
    }

    #[test]
    fn drops_disallowed_tag() {
        let s = sanitize_telegram_html("<script>alert(1)</script>");
        assert_eq!(s, "alert(1)");
    }

    #[test]
    fn rejects_unsafe_href() {
        let s = sanitize_telegram_html("<a href=\"javascript:bad\">x</a>");
        // <a> dropped because href failed → only inner text survives.
        assert_eq!(s, "x");
    }

    #[test]
    fn accepts_safe_href() {
        let s = sanitize_telegram_html("<a href=\"https://example.com\">x</a>");
        assert!(s.contains("<a href=\"https://example.com\">"));
    }

    #[test]
    fn auto_closes_unclosed_tag() {
        let s = sanitize_telegram_html("<b>unclosed");
        assert_eq!(s, "<b>unclosed</b>");
    }

    #[test]
    fn keeps_code_class() {
        let s = sanitize_telegram_html("<code class=\"rust\">x</code>");
        assert!(s.contains("class=\"rust\""));
    }
}
