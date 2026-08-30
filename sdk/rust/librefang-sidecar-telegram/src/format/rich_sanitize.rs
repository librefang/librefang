//! Neutralise "active" constructs in agent-authored Rich Markdown before it is
//! handed to `sendRichMessage`.
//!
//! Rich Markdown "can contain arbitrary HTML" (Bot API 10.1+). That is a problem for
//! us: the text we send is model output, and model output routinely *quotes* untrusted
//! content — a fetched web page, an email body, a file the agent read. Without this
//! pass, quoted content could render itself inline buttons:
//!
//! ```text
//! <tg-button type="callback_data" data="anything">Click me</tg-button>
//! ```
//!
//! A tap then arrives back at the adapter as a `ButtonCallback` event with an
//! attacker-chosen payload. Interactive buttons must stay an explicit
//! `ChannelContent::Interactive` feature, never a side effect of formatting text.
//!
//! # Why this escapes every `<` with no exceptions
//!
//! Earlier versions of this module kept exemptions — inline code spans, fenced blocks
//! and allowed HTML tags were copied through verbatim so a user's code sample would
//! render as written. Each exemption needs to know where the construct *ends*, which
//! means reimplementing a piece of CommonMark. Five rounds of adversarial review found
//! five separate inputs where that reimplementation disagreed with a real parser, and
//! each disagreement copied a live `<tg-button>` through: a code span pairing across a
//! blank line, then across a lone `\r`, then across an escaped backtick; a tag scan
//! running past its tag; a four-space-indented ` ``` ` mistaken for a fence. Two of the
//! five were introduced *by the fix for the previous one*.
//!
//! The general rule, from the wider Markdown/HTML sanitiser world, is that you cannot
//! decide what a parser will do without being that parser — anything less is an
//! illusion of security. So this pass no longer tries. Every `<` becomes `&lt;`,
//! unconditionally, with no attempt to find code spans, fences or well-formed tags.
//!
//! What that guarantees, and what it costs:
//!
//! * **Guaranteed:** no raw HTML reaches Telegram, so no message text can produce a
//!   button, a media fetch or any other active element. This holds by construction and
//!   needs no agreement with any parser.
//! * **Cost:** a `<` inside a user's code sample renders as the literal `&lt;`, because
//!   Markdown does not decode entities inside code. `Vec<String>` in a fenced block will
//!   read `Vec&lt;String>`. This is the price of the guarantee, and it is the reason to
//!   move to `InputRichMessage.blocks`, where a preformatted block's text is a plain
//!   string that Telegram never parses and nothing needs escaping at all.
//! * **Also lost:** the Rich HTML tags with no Markdown equivalent (`<u>`, `<sub>`,
//!   `<sup>`). Emphasis, strikethrough, spoilers, tables, lists and headings are all
//!   plain Markdown and unaffected.
//!
//! Link destinations are still checked against `sanitize`'s scheme allowlist, but that
//! check is **best-effort**, not a guarantee: locating a Markdown link exactly has the
//! same problem as everything above. It is defence in depth. The property this module
//! actually promises is the HTML one.

use std::ops::Range;

/// Schemes a link destination may carry. Mirrors `sanitize::ALLOWED_HREF_SCHEMES`.
const ALLOWED_HREF_SCHEMES: &[&str] = &["https:", "http:", "mailto:", "tg:"];

/// CommonMark caps a link label at 999 characters. Bounding the label scan keeps this
/// pass linear — scanning to end of input for every unmatched `[` was quadratic, and a
/// megabyte of them stalled the sidecar for hours.
const MAX_LINK_LABEL: usize = 999;

/// Escape agent text so Telegram's Rich Markdown parser cannot be steered into
/// producing interactive or media-fetching elements.
pub fn sanitize_rich_markdown(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            // The whole security property, in one arm: no `<` survives, so no raw HTML
            // can reach Telegram regardless of what the surrounding text looks like.
            b'<' => {
                out.push_str("&lt;");
                i += 1;
            }
            // `![...](...)` is a real media block in Rich Markdown, fetched from the
            // URL — today it is inert text. Escape the `!` so the link (if any) renders
            // the way the legacy HTML pipeline renders it, without attaching media.
            b'!' if bytes.get(i + 1) == Some(&b'[') => {
                out.push_str("\\!");
                i += 1;
            }
            // Best-effort scheme check on both the inline form `[label](destination)`
            // and the reference definition `[label]: destination`, which supplies the
            // destination for a `[x][label]` elsewhere in the message.
            b'[' => {
                let disallowed = link_destination_at(bytes, i)
                    .or_else(|| reference_definition_at(bytes, i))
                    .is_some_and(|dest| !scheme_is_allowed(&input[dest]));
                out.push_str(if disallowed { "\\[" } else { "[" });
                i += 1;
            }
            b => {
                let len = utf8_char_len(b);
                out.push_str(&input[i..i + len]);
                i += len;
            }
        }
    }

    out
}

/// Length in bytes of the UTF-8 character starting with `first`.
fn utf8_char_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// True when `dest` carries no scheme at all (a relative target or `#anchor`) or carries
/// one on the allowlist. A scheme we do not recognise is rejected.
///
/// The destination is normalised first, because the check has to hold on what the
/// *parser* sees rather than on the literal bytes: `<…>`-wrapped destinations, HTML
/// entities standing in for the colon (`javascript&#58;…`) and embedded whitespace or
/// control characters (`java\tscript:…`) all reach a live scheme otherwise.
fn scheme_is_allowed(dest: &str) -> bool {
    let normalised = normalise_destination(dest);
    let bytes = normalised.as_bytes();
    // `Option::is_none_or` is stable only from 1.82; the crate's MSRV is 1.80.
    if !bytes.first().is_some_and(|c| c.is_ascii_alphabetic()) {
        return true; // no scheme possible
    }
    let mut j = 0;
    while bytes
        .get(j)
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
    {
        j += 1;
    }
    if bytes.get(j) != Some(&b':') {
        return true; // not a scheme, just text before a slash or space
    }
    let scheme = &normalised[..=j];
    ALLOWED_HREF_SCHEMES
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s))
}

/// Strip a `<…>` wrapper, decode HTML entities, and drop whitespace and control
/// characters, so the scheme check sees the destination the parser will resolve.
/// Only the leading run matters, so this stops at the first delimiter.
fn normalise_destination(dest: &str) -> String {
    let trimmed = dest.trim();
    let inner = trimmed
        .strip_prefix('<')
        .map(|rest| rest.strip_suffix('>').unwrap_or(rest))
        .unwrap_or(trimmed);

    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' => {
                let mut entity = String::new();
                while let Some(&next) = chars.peek() {
                    if next == ';' || entity.len() >= 8 {
                        break;
                    }
                    entity.push(next);
                    chars.next();
                }
                if chars.peek() == Some(&';') {
                    chars.next();
                }
                match decode_entity(&entity) {
                    // A decoded character goes through the same filter as a literal one.
                    // Pushing it unconditionally let `&#32;javascript:` start with a
                    // space, which reads as "no scheme here" while the HTML and URL
                    // parsers both discard that space and resolve the scheme.
                    Some(decoded) if decoded.is_whitespace() || decoded.is_control() => {}
                    Some(decoded) => out.push(decoded),
                    // An entity we cannot decode is skipped rather than ending the scan:
                    // `&Tab;` and `&NewLine;` are real HTML5 entities we do not know, and
                    // stopping here truncated the destination so the scheme behind it was
                    // never examined.
                    None => {}
                }
            }
            c if c.is_whitespace() || c.is_control() => {}
            c => out.push(c),
        }
        // A scheme is short; past this the answer cannot change.
        if out.len() > 64 {
            break;
        }
    }
    out
}

/// Decode the entity forms that can stand in for a character inside a scheme:
/// `&#58;`, `&#x3a;` and the named `&colon;`.
fn decode_entity(entity: &str) -> Option<char> {
    if entity.eq_ignore_ascii_case("colon") {
        return Some(':');
    }
    let digits = entity.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    char::from_u32(code)
}

/// Byte range of the destination in a `[label](destination)` starting at `i` (which must
/// be `[`). `None` when this is not an inline link.
fn link_destination_at(bytes: &[u8], i: usize) -> Option<Range<usize>> {
    let label_end = link_label_end(bytes, i)?;
    if bytes.get(label_end + 1) != Some(&b'(') {
        return None;
    }
    // Whitespace may sit on either side of the destination, with an optional title in
    // between.
    let start = skip_ascii_whitespace(bytes, label_end + 2);
    if bytes.get(start) == Some(&b'<') {
        let mut k = start + 1;
        while bytes.get(k).is_some_and(|&c| c != b'>' && c != b'\n') {
            k += 1;
        }
        return (bytes.get(k) == Some(&b'>')).then_some(start..k + 1);
    }
    let mut k = start;
    let mut parens = 0_i32;
    while let Some(&c) = bytes.get(k) {
        match c {
            b'\\' => k += 1,
            b'(' => parens += 1,
            b')' if parens == 0 => return Some(start..k),
            b')' => parens -= 1,
            c if c.is_ascii_whitespace() => break,
            _ => {}
        }
        k += 1;
    }
    let dest = start..k;
    let mut k = skip_ascii_whitespace(bytes, k);
    if let Some(&quote @ (b'"' | b'\'' | b'(')) = bytes.get(k) {
        let closer = if quote == b'(' { b')' } else { quote };
        k += 1;
        while bytes.get(k).is_some_and(|&c| c != closer) {
            k += 1;
        }
        k = skip_ascii_whitespace(bytes, k + 1);
    }
    (bytes.get(k) == Some(&b')')).then_some(dest)
}

/// Byte range of the destination in a link reference definition `[label]: destination`
/// starting at `i`. `None` when this is not a definition.
fn reference_definition_at(bytes: &[u8], i: usize) -> Option<Range<usize>> {
    let label_end = link_label_end(bytes, i)?;
    if bytes.get(label_end + 1) != Some(&b':') {
        return None;
    }
    let start = skip_ascii_whitespace(bytes, label_end + 2);
    let mut end = start;
    while bytes
        .get(end)
        .is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>')
    {
        end += 1;
    }
    // `<…>` wrapping is allowed here too; `normalise_destination` strips it.
    if bytes.get(end) == Some(&b'>') {
        end += 1;
    }
    (end > start).then_some(start..end)
}

/// Index of the `]` closing the link label opening at `i`, honouring balanced brackets
/// and backslash escapes. Bounded by [`MAX_LINK_LABEL`], which is CommonMark's own cap.
fn link_label_end(bytes: &[u8], i: usize) -> Option<usize> {
    let mut depth = 0_i32;
    let mut j = i;
    let limit = i.saturating_add(MAX_LINK_LABEL);
    while j <= limit {
        match bytes.get(j)? {
            b'\\' => j += 1,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

fn skip_ascii_whitespace(bytes: &[u8], mut k: usize) -> usize {
    while bytes.get(k).is_some_and(u8::is_ascii_whitespace) {
        k += 1;
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const BUTTON: &str = r#"<tg-button type="callback_data" data="wipe">Tap</tg-button>"#;

    /// The guarantee: no `<` survives, so no raw HTML can reach Telegram. These are the
    /// five inputs that five rounds of review found against the exemption-based design;
    /// none of them needs a special case now.
    #[test]
    fn no_raw_html_survives_any_context() {
        for context in [
            BUTTON.to_string(),
            format!("a `hello\n\n{BUTTON}\n\n` b"),
            format!("a `hello\r> {BUTTON}\r> ` b"),
            format!("a \\` {BUTTON} \\` b"),
            format!("<b {BUTTON}"),
            format!("    ```\n{BUTTON}\n"),
            format!("```a`b\n{BUTTON}\n"),
            format!("```\n{BUTTON}\n```\n"),
            format!(r#"<b a="1"b="{BUTTON}">"#),
            format!("<a title=\"`\" href=\"https://ok\">t</a> {BUTTON}"),
        ] {
            let out = sanitize_rich_markdown(&context);
            assert!(!out.contains('<'), "raw `<` survived: {out}");
        }
    }

    #[test]
    fn markdown_formatting_is_untouched() {
        for source in [
            "| a | b |\n|:--|--:|\n| _x_ | **y _z_** |\n",
            "**bold _italic_ bold** ~~strike~~ ||spoiler|| `code`",
            "# Heading\n\n- item\n- item\n\n1. one\n2. two\n",
            "> quote\n> more\n\n---\n",
            "```rust\nfn main() {}\n```\n",
            "[x](https://example.com/a_(b)_c)",
            "[x][ref]\n\n[ref]: https://example.com",
        ] {
            assert_eq!(sanitize_rich_markdown(source), source);
        }
    }

    /// The documented cost of the guarantee: a `<` in a code sample is escaped like any
    /// other, and Markdown does not decode entities inside code. Pinned so the trade-off
    /// is visible rather than discovered.
    #[test]
    fn a_less_than_in_a_code_sample_is_escaped_too() {
        assert_eq!(
            sanitize_rich_markdown("```rust\nlet v: Vec<String>;\n```"),
            "```rust\nlet v: Vec&lt;String>;\n```"
        );
    }

    #[test]
    fn disallowed_link_schemes_are_escaped() {
        for bad in [
            "[click](javascript:alert(1))",
            "[x](<javascript:alert(1)>)",
            "[x](javascript&#58;alert(1))",
            "[x](javascript&#x3a;alert(1))",
            "[x](javascript&colon;alert(1))",
            "[x](&#32;javascript:alert(1))",
            "[x](&Tab;javascript:alert(1))",
            "[a[b]c](javascript:alert(1))",
            "[x]( javascript:alert(1) )",
            "[x](javascript:alert(1)\n)",
            "[x](javascript:alert(1) \"title\")",
        ] {
            let out = sanitize_rich_markdown(bad);
            assert!(out.starts_with("\\["), "not escaped: {out}");
        }
    }

    #[test]
    fn reference_definitions_are_scheme_checked_at_any_indent() {
        for indent in ["", " ", "  ", "   ", "\t"] {
            let input = format!("[x][ref]\n\n{indent}[ref]: javascript:alert(1)");
            let out = sanitize_rich_markdown(&input);
            assert!(out.contains("\\[ref]:"), "indent {indent:?} skipped: {out}");
        }
        let ok = "[x][ref]\n\n[ref]: https://example.com";
        assert_eq!(sanitize_rich_markdown(ok), ok);
    }

    #[test]
    fn image_syntax_is_escaped_so_media_is_not_fetched() {
        assert_eq!(
            sanitize_rich_markdown("see ![alt](https://evil.example/x.jpg)"),
            "see \\![alt](https://evil.example/x.jpg)"
        );
    }

    /// Bounding the label scan at CommonMark's own 999-character cap keeps the pass
    /// linear. Unbounded, a megabyte of `[` took hours.
    #[test]
    fn unmatched_brackets_are_processed_linearly() {
        let input = "[".repeat(200_000);
        let start = Instant::now();
        let out = sanitize_rich_markdown(&input);
        assert_eq!(out.len(), input.len());
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "took {:?} — scan is superlinear",
            start.elapsed()
        );
    }

    /// CommonMark caps a link label at 999 characters, so a longer one is not a label
    /// and the text is inert. Bounding the scan there is what keeps a run of unmatched
    /// brackets cheap.
    #[test]
    fn label_longer_than_the_commonmark_cap_is_not_a_link() {
        let long_label = format!("[{}](javascript:alert(1))", "x".repeat(1500));
        assert_eq!(sanitize_rich_markdown(&long_label), long_label);
        let short_label = format!("[{}](javascript:alert(1))", "x".repeat(500));
        assert!(sanitize_rich_markdown(&short_label).starts_with("\\["));
    }

    #[test]
    fn multibyte_text_is_preserved() {
        let s = "таблица — да, 🎉 <tg-button>нет</tg-button>";
        let out = sanitize_rich_markdown(s);
        assert!(out.contains("таблица — да"));
        assert!(out.contains('🎉'));
        assert!(!out.contains('<'));
    }

    #[test]
    fn edge_inputs_do_not_panic() {
        for input in ["", " ", "<", "[", "![", "\\", "`", "\u{feff}", "\r", "&#"] {
            let _ = sanitize_rich_markdown(input);
        }
    }
}
