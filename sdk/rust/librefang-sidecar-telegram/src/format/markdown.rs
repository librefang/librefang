//! Lightweight Markdown → Telegram-HTML converter.
//!
//! Mirrors the Python adapter's `markdown_to_telegram_html` + `_render_inline_markdown`.
//! Block-level: code fences, headings, blockquotes, unordered + ordered lists, plain paragraphs.
//! Inline: `**bold**`, single-star `*italic*`, `` `code` ``, `[text](url)`.
//! Not a general-purpose Markdown engine — only the constructs the Python adapter supports, so the wire-formatted output stays byte-equivalent across languages.

use once_cell::sync::Lazy;
use regex::Regex;

/// Escape `&`, `<`, `>` for HTML, and pre-emptively strip the Private-Use sentinels that `render_inline_markdown` uses for inline-code placeholders. Without the strip, adversarial input containing those code points would survive escape_html and collide with a real placeholder during the restore pass, letting the attacker inject `<code>` via `sanitize_telegram_html`'s allowlist.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            CODE_PLACEHOLDER_OPEN | CODE_PLACEHOLDER_CLOSE => {}
            other => out.push(other),
        }
    }
    out
}

/// Private-Use Area code points used as inline-code placeholder bookends. These survive `escape_html` only because `escape_html` strips them on input — see the doc comment on `escape_html`.
const CODE_PLACEHOLDER_OPEN: char = '\u{E000}';
const CODE_PLACEHOLDER_CLOSE: char = '\u{E001}';

static RE_LINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("link regex"));
static RE_BOLD: Lazy<Regex> = Lazy::new(|| Regex::new(r"\*\*([^*]+)\*\*").expect("bold regex"));
// Single-star italic — careful not to match `**`.
static RE_ITALIC: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:^|[^*])\*([^*\n]+)\*(?:[^*]|$)").expect("italic regex"));
static RE_CODE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`([^`\n]+)`").expect("code regex"));

/// Render one inline-Markdown chunk to HTML.
fn render_inline_markdown(text: &str) -> String {
    let escaped = escape_html(text);

    // Inline code first so its content is opaque to bold/italic scanning.
    // Placeholders use Private-Use Area sentinels which `escape_html` strips from input, so adversarial text cannot collide with this scheme.
    let mut placeholders: Vec<String> = Vec::new();
    let with_codes = RE_CODE.replace_all(&escaped, |caps: &regex::Captures<'_>| {
        let idx = placeholders.len();
        placeholders.push(format!("<code>{}</code>", &caps[1]));
        format!("{CODE_PLACEHOLDER_OPEN}C{idx}{CODE_PLACEHOLDER_CLOSE}")
    });

    // Bold next (double-star).
    let with_bold = RE_BOLD
        .replace_all(&with_codes, |caps: &regex::Captures<'_>| {
            format!("<b>{}</b>", &caps[1])
        })
        .to_string();

    // Italic — the regex captures a leading non-star char to disambiguate from `**`; we need to preserve that char.
    // Run iteratively so adjacent italics don't get skipped.
    let mut italics_done = with_bold;
    loop {
        let after = RE_ITALIC
            .replace(&italics_done, |caps: &regex::Captures<'_>| {
                let m = caps.get(0).unwrap().as_str();
                let inner = &caps[1];
                // Preserve the leading non-`*` byte (and trailing non-`*` byte) so word boundaries don't drift.
                let leading = m.chars().next().filter(|c| *c != '*').map_or("", |_| {
                    &m[..m.char_indices().nth(1).map(|(i, _)| i).unwrap_or(0)]
                });
                let trailing = if m.ends_with('*') {
                    ""
                } else {
                    let last_idx = m.char_indices().last().map(|(i, _)| i).unwrap_or(m.len());
                    &m[last_idx..]
                };
                format!("{leading}<i>{inner}</i>{trailing}")
            })
            .to_string();
        if after == italics_done {
            break;
        }
        italics_done = after;
    }

    // Links last so `[text](url)` inside bold/italic is recognised.
    let with_links = RE_LINK
        .replace_all(&italics_done, |caps: &regex::Captures<'_>| {
            let label = &caps[1];
            let url = &caps[2];
            format!("<a href=\"{url}\">{label}</a>")
        })
        .to_string();

    // Restore code placeholders.
    let mut restored = with_links;
    for (i, html) in placeholders.iter().enumerate() {
        let placeholder = format!("{CODE_PLACEHOLDER_OPEN}C{i}{CODE_PLACEHOLDER_CLOSE}");
        restored = restored.replace(&placeholder, html);
    }
    restored
}

/// Convert Markdown text to Telegram-compatible HTML.
/// Block constructs: code fences, headings, blockquotes, lists, paragraphs.
pub fn markdown_to_telegram_html(text: &str) -> String {
    // Normalise line endings.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::new();
    let mut lines = text.lines().peekable();

    let mut current_list_kind: Option<ListKind> = None;
    let mut ordered_counter: u32 = 1;

    while let Some(line) = lines.next() {
        // Code fence.
        if let Some(fence) = code_fence(line) {
            let mut body = String::new();
            for inner in lines.by_ref() {
                if inner.trim() == fence {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            // Strip trailing newline added by the loop.
            if body.ends_with('\n') {
                body.pop();
            }
            out.push_str("<pre><code>");
            out.push_str(&escape_html(&body));
            out.push_str("</code></pre>\n");
            current_list_kind = None;
            continue;
        }
        // Heading.
        if let Some(rest) = heading(line) {
            current_list_kind = None;
            out.push_str("<b>");
            out.push_str(&render_inline_markdown(rest));
            out.push_str("</b>\n");
            continue;
        }
        // Blockquote.
        if let Some(content) = blockquote(line) {
            current_list_kind = None;
            out.push_str("<blockquote>");
            out.push_str(&render_inline_markdown(content));
            out.push_str("</blockquote>\n");
            continue;
        }
        // Unordered list.
        if let Some(content) = unordered_list(line) {
            if !matches!(current_list_kind, Some(ListKind::Unordered)) {
                current_list_kind = Some(ListKind::Unordered);
            }
            out.push_str("• ");
            out.push_str(&render_inline_markdown(content));
            out.push('\n');
            continue;
        }
        // Ordered list.
        if let Some(content) = ordered_list(line) {
            if !matches!(current_list_kind, Some(ListKind::Ordered)) {
                current_list_kind = Some(ListKind::Ordered);
                ordered_counter = 1;
            }
            out.push_str(&format!("{ordered_counter}. "));
            out.push_str(&render_inline_markdown(content));
            out.push('\n');
            ordered_counter += 1;
            continue;
        }
        // Blank line resets list context.
        if line.trim().is_empty() {
            current_list_kind = None;
            ordered_counter = 1;
            out.push('\n');
            continue;
        }
        // Plain paragraph line.
        current_list_kind = None;
        out.push_str(&render_inline_markdown(line));
        out.push('\n');
    }
    out
}

#[derive(Debug, Clone, Copy)]
enum ListKind {
    Unordered,
    Ordered,
}

fn code_fence(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = &trimmed[hashes..];
        if rest.starts_with(' ') {
            return Some(rest.trim_start());
        }
    }
    None
}

fn blockquote(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix("> ")
        .or_else(|| trimmed.strip_prefix(">"))
}

fn unordered_list(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(rest);
        }
    }
    None
}

fn ordered_list(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let mut digit_end = 0;
    for (i, c) in trimmed.char_indices() {
        if c.is_ascii_digit() {
            digit_end = i + 1;
        } else {
            break;
        }
    }
    if digit_end == 0 {
        return None;
    }
    let rest = &trimmed[digit_end..];
    for sep in [". ", ") "] {
        if let Some(after) = rest.strip_prefix(sep) {
            return Some(after);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paragraph() {
        let html = markdown_to_telegram_html("hello world");
        assert_eq!(html.trim(), "hello world");
    }

    #[test]
    fn bold_and_italic() {
        let html = markdown_to_telegram_html("**bold** and *italic*");
        assert!(html.contains("<b>bold</b>"));
        assert!(html.contains("<i>italic</i>"));
    }

    #[test]
    fn inline_code() {
        let html = markdown_to_telegram_html("use `cargo build`");
        assert!(html.contains("<code>cargo build</code>"));
    }

    #[test]
    fn code_fence_renders_pre_code() {
        let md = "before\n```\nlet x = 1;\n```\nafter";
        let html = markdown_to_telegram_html(md);
        assert!(
            html.contains("<pre><code>let x = 1;\n</code></pre>")
                || html.contains("<pre><code>let x = 1;</code></pre>")
        );
    }

    #[test]
    fn heading_becomes_bold() {
        let html = markdown_to_telegram_html("# Title");
        assert!(html.contains("<b>Title</b>"));
    }

    #[test]
    fn unordered_list_bullets() {
        let html = markdown_to_telegram_html("- one\n- two");
        assert!(html.contains("• one"));
        assert!(html.contains("• two"));
    }

    #[test]
    fn ordered_list_numbers() {
        let html = markdown_to_telegram_html("1. one\n2. two");
        assert!(html.contains("1. one"));
        assert!(html.contains("2. two"));
    }

    #[test]
    fn escape_html_basic() {
        assert_eq!(escape_html("<a&b>"), "&lt;a&amp;b&gt;");
    }
}
