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
//! The pass mirrors the posture of `sanitize::sanitize_telegram_html`: an **allowlist**
//! of passive formatting tags, everything else escaped to literal text. An allowlist
//! (rather than a denylist of known-active tags) means any tag a future Bot API version
//! adds is inert here until someone deliberately allows it.
//!
//! Link destinations are held to `sanitize`'s scheme allowlist on both `<a href>` and
//! `[label](destination)`, so the guarantee that `javascript:` and `data:` never reach a
//! live tag holds on this path too. `tg:` stays allowed, matching the legacy path; its
//! media forms (`tg://photo?id=` and friends) only resolve against the
//! `InputRichMessage.media` array, which we never populate, so they are inert.
//!
//! Content inside code spans and fenced code blocks is copied verbatim. Markdown does
//! not interpret HTML there, so it is already inert — and escaping it would surface a
//! literal `&lt;` to the user inside their code sample. Both are bounded to a single
//! block: Markdown resolves block structure *before* inline structure, so a span that
//! appeared to cross a blank line, blockquote or heading would copy that whole region
//! verbatim and hand an attacker a way to smuggle raw HTML through.

/// Passive formatting tags that are safe to let Telegram parse. Everything outside this
/// list is escaped. Kept deliberately close to `sanitize::ALLOWED_TAGS`, plus the
/// structural/inline tags Rich HTML adds that carry no action and no media fetch.
const ALLOWED_RICH_TAGS: &[&str] = &[
    // inline emphasis
    "b",
    "strong",
    "i",
    "em",
    "u",
    "ins",
    "s",
    "strike",
    "del",
    "mark",
    "sub",
    "sup",
    "tg-spoiler",
    "tg-emoji",
    // code
    "code",
    "pre",
    // links (href scheme is checked separately, see `anchor_href_allowed`)
    "a",
    // block structure
    "p",
    "br",
    "hr",
    "blockquote",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tbody",
    "tr",
    "td",
    "th",
];

/// Escape agent text so Telegram's Rich Markdown parser cannot be steered into
/// producing interactive or media-fetching elements.
pub fn sanitize_rich_markdown(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut at_line_start = true;

    while i < bytes.len() {
        // Fenced code block: copy the whole fence verbatim, including the fence lines.
        if at_line_start {
            if let Some(fence) = fence_at(bytes, i) {
                let end = copy_fenced_block(input, bytes, i, fence, &mut out);
                i = end;
                at_line_start = true;
                continue;
            }
        }

        let b = bytes[i];

        // Inline code span: copy verbatim so `<tg-button>` in a code sample stays readable.
        if b == b'`' {
            let end = copy_code_span(input, bytes, i, &mut out);
            at_line_start = false;
            i = end;
            continue;
        }

        // `![...](...)` is a real media block in Rich Markdown, fetched from the URL —
        // today it is inert text. Escape the `!` so the link (if any) still renders the
        // way the current HTML pipeline renders it, without attaching media.
        if b == b'!' && bytes.get(i + 1) == Some(&b'[') {
            out.push_str("\\!");
            at_line_start = false;
            i += 1;
            continue;
        }

        // A Markdown link whose destination carries a scheme we do not allow is
        // escaped whole, so `[x](javascript:...)` stays literal text. `sanitize`
        // drops such links on the legacy path; without this the rich path would
        // be the weaker of the two.
        if b == b'[' {
            if let Some(dest) = link_destination_at(bytes, i) {
                if !scheme_is_allowed(&input[dest]) {
                    out.push_str("\\[");
                    at_line_start = false;
                    i += 1;
                    continue;
                }
            }
        }

        if b == b'<' {
            let allowed = match tag_name_at(bytes, i) {
                // `<a>` is allowed only when its href scheme is. Telegram does not
                // filter schemes for us, and `sanitize::ALLOWED_HREF_SCHEMES`
                // promises `javascript:` / `data:` never reach a live tag.
                Some(name) if name == "a" => anchor_href_allowed(input, bytes, i),
                Some(name) => ALLOWED_RICH_TAGS.contains(&name.as_str()),
                // Not a tag at all, e.g. a bare `a < b`.
                None => false,
            };
            out.push_str(if allowed { "<" } else { "&lt;" });
            at_line_start = false;
            i += 1;
            continue;
        }

        let ch_len = utf8_char_len(b);
        out.push_str(&input[i..i + ch_len]);
        at_line_start = b == b'\n';
        i += ch_len;
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

/// A code-fence line at `i`: marker byte, run length, and whether an info string
/// follows the run. CommonMark allows an info string only on the *opening* fence, so
/// callers matching a closing fence must reject `has_info`.
fn fence_at(bytes: &[u8], i: usize) -> Option<(u8, usize, bool)> {
    // Up to three leading spaces still open a fence in CommonMark.
    let mut j = i;
    let mut spaces = 0;
    while j < bytes.len() && bytes[j] == b' ' && spaces < 3 {
        j += 1;
        spaces += 1;
    }
    let marker = *bytes.get(j)?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let mut run = 0;
    while bytes.get(j + run) == Some(&marker) {
        run += 1;
    }
    if run < 3 {
        return None;
    }
    let rest = &bytes[j + run..line_end(bytes, i).min(bytes.len())];
    let has_info = rest
        .iter()
        .any(|c| !matches!(c, b' ' | b'\t' | b'\r' | b'\n'));
    Some((marker, run, has_info))
}

/// Schemes a link destination may carry. Mirrors `sanitize::ALLOWED_HREF_SCHEMES`, whose
/// guarantee (`javascript:` / `data:` never reach a live tag) the legacy path enforces
/// and this path must not weaken.
const ALLOWED_HREF_SCHEMES: &[&str] = &["https:", "http:", "mailto:", "tg:"];

/// True when `dest` carries no scheme at all (a relative target or `#anchor`) or carries
/// one on the allowlist. A scheme we do not recognise is rejected.
fn scheme_is_allowed(dest: &str) -> bool {
    let dest = dest.trim();
    let bytes = dest.as_bytes();
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
    let scheme = &dest[..=j];
    ALLOWED_HREF_SCHEMES
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s))
}

/// Byte range of the destination in a `[label](destination)` starting at `i` (which must
/// be `[`). `None` when this is not an inline link.
fn link_destination_at(bytes: &[u8], i: usize) -> Option<std::ops::Range<usize>> {
    let mut j = i + 1;
    while bytes.get(j).is_some_and(|&c| c != b']' && c != b'\n') {
        j += 1;
    }
    if bytes.get(j) != Some(&b']') || bytes.get(j + 1) != Some(&b'(') {
        return None;
    }
    let start = j + 2;
    let mut k = start;
    while bytes.get(k).is_some_and(|&c| c != b')' && c != b'\n') {
        k += 1;
    }
    (bytes.get(k) == Some(&b')')).then_some(start..k)
}

/// True when the `<a …>` tag opening at `i` has no href, or one whose scheme is allowed.
/// A closing `</a>` carries no href and is always allowed.
fn anchor_href_allowed(input: &str, bytes: &[u8], i: usize) -> bool {
    let mut j = i + 1;
    if bytes.get(j) == Some(&b'/') {
        return true;
    }
    let tag_end = {
        let mut k = j;
        while bytes.get(k).is_some_and(|&c| c != b'>' && c != b'\n') {
            k += 1;
        }
        k
    };
    while j < tag_end {
        if input[j..tag_end].starts_with("href") {
            let mut k = j + 4;
            while bytes.get(k).is_some_and(|c| c.is_ascii_whitespace()) {
                k += 1;
            }
            if bytes.get(k) != Some(&b'=') {
                j += 1;
                continue;
            }
            k += 1;
            while bytes.get(k).is_some_and(|c| c.is_ascii_whitespace()) {
                k += 1;
            }
            let quote = bytes.get(k).copied();
            let (start, end) = match quote {
                Some(q @ (b'"' | b'\'')) => {
                    let s = k + 1;
                    let mut e = s;
                    while e < tag_end && bytes[e] != q {
                        e += 1;
                    }
                    (s, e)
                }
                _ => {
                    let s = k;
                    let mut e = s;
                    while e < tag_end && !bytes[e].is_ascii_whitespace() {
                        e += 1;
                    }
                    (s, e)
                }
            };
            return scheme_is_allowed(&input[start..end.min(tag_end)]);
        }
        j += 1;
    }
    true // no href attribute at all
}

/// Copy a fenced code block verbatim starting at `i`. Returns the index just past it.
/// An unclosed fence copies to end-of-input, matching how Markdown renders it.
fn copy_fenced_block(
    input: &str,
    bytes: &[u8],
    i: usize,
    fence: (u8, usize, bool),
    out: &mut String,
) -> usize {
    let (marker, run, _) = fence;
    // Copy the opening fence line.
    let mut pos = line_end(bytes, i);
    out.push_str(&input[i..pos]);

    while pos < bytes.len() {
        let line_start = pos;
        let end = line_end(bytes, line_start);
        out.push_str(&input[line_start..end]);
        // A closing fence is the same marker, at least as long as the opener, and
        // carries no info string — ` ```js ` inside a block is content, not a close.
        if let Some((m, r, has_info)) = fence_at(bytes, line_start) {
            if m == marker && r >= run && !has_info {
                return end;
            }
        }
        pos = end;
    }
    pos
}

/// Index just past the end of the line starting at `i` (including its `\n`).
fn line_end(bytes: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < bytes.len() && bytes[j] != b'\n' {
        j += 1;
    }
    if j < bytes.len() {
        j + 1
    } else {
        j
    }
}

/// Copy an inline code span verbatim. A run of N backticks closes on the next run of
/// exactly N. If it never closes, the backticks are not a code span at all — copy just
/// the run and let the caller keep scanning (and escaping) the rest.
///
/// The scan stops at any line that starts a new block. Markdown parses block structure
/// *before* inline structure, so a backtick in one block can never pair with one in
/// another — and pairing them here would copy everything between the two verbatim,
/// handing an attacker a way to smuggle a live `<tg-button>` past this pass by planting
/// a stray backtick on either side of it. Stopping early is the safe direction: the
/// backticks are then treated as ordinary text and the content is escaped.
fn copy_code_span(input: &str, bytes: &[u8], i: usize, out: &mut String) -> usize {
    let mut run = 0;
    while bytes.get(i + run) == Some(&b'`') {
        run += 1;
    }
    let mut j = i + run;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let mut close = 0;
            while bytes.get(j + close) == Some(&b'`') {
                close += 1;
            }
            if close == run {
                let end = j + close;
                out.push_str(&input[i..end]);
                return end;
            }
            j += close;
            continue;
        }
        if bytes[j] == b'\n' && starts_new_block(bytes, j + 1) {
            break;
        }
        j += 1;
    }
    out.push_str(&input[i..i + run]);
    i + run
}

/// True when the line starting at `i` ends the current paragraph — either it is blank
/// (spaces and tabs only, tolerating CRLF) or its first non-space character opens a new
/// block. Used to bound an inline code-span scan to a single paragraph.
fn starts_new_block(bytes: &[u8], i: usize) -> bool {
    if i >= bytes.len() {
        return true;
    }
    let mut j = i;
    while bytes.get(j).is_some_and(|c| matches!(c, b' ' | b'\t')) {
        j += 1;
    }
    match bytes.get(j) {
        // Blank line — the paragraph ends here. `\r` covers CRLF input, which reaches us
        // verbatim from quoted email and web content.
        None | Some(b'\n') | Some(b'\r') => true,
        // Block quotation, ATX heading, table row, thematic break / list bullet, fence.
        Some(b'>') | Some(b'#') | Some(b'|') => true,
        Some(b'`') | Some(b'~') => fence_at(bytes, i).is_some(),
        Some(b'-') | Some(b'*') | Some(b'+') => {
            matches!(bytes.get(j + 1), Some(b' ') | Some(b'\t'))
        }
        Some(c) if c.is_ascii_digit() => {
            let mut k = j;
            while bytes.get(k).is_some_and(|c| c.is_ascii_digit()) {
                k += 1;
            }
            matches!(bytes.get(k), Some(b'.') | Some(b')'))
                && matches!(bytes.get(k + 1), Some(b' ') | Some(b'\t'))
        }
        _ => false,
    }
}

/// Lower-cased tag name of the HTML tag opening at `i` (which must be `<`), for both
/// `<foo ...>` and `</foo>`. `None` when this is not tag-shaped.
fn tag_name_at(bytes: &[u8], i: usize) -> Option<String> {
    let mut j = i + 1;
    if bytes.get(j) == Some(&b'/') {
        j += 1;
    }
    let start = j;
    while let Some(&c) = bytes.get(j) {
        if c.is_ascii_alphanumeric() || c == b'-' {
            j += 1;
        } else {
            break;
        }
    }
    if j == start {
        return None;
    }
    // Must actually terminate like a tag, not be prose such as `5 <3 apples`.
    match bytes.get(j) {
        Some(&c) if c == b'>' || c == b'/' || c.is_ascii_whitespace() => {}
        _ => return None,
    }
    Some(String::from_utf8_lossy(&bytes[start..j]).to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_injected_buttons() {
        let injected = r#"Report<tg-button type="callback_data" data="x">Tap</tg-button>"#;
        let out = sanitize_rich_markdown(injected);
        assert!(!out.contains("<tg-button"));
        assert!(out.contains("&lt;tg-button"));
        assert!(out.contains("&lt;/tg-button"));
    }

    #[test]
    fn strips_button_rows_maps_and_media_containers() {
        for tag in [
            "tg-button-row",
            "tg-map",
            "tg-collage",
            "tg-slideshow",
            "tg-thinking",
            "details",
            "summary",
            "img",
            "video",
        ] {
            let out = sanitize_rich_markdown(&format!("<{tag}>x"));
            assert!(out.starts_with("&lt;"), "{tag} was not escaped: {out}");
        }
    }

    #[test]
    fn keeps_passive_formatting_tags() {
        let s = "<b>bold</b> <u>under</u> <tg-spoiler>hidden</tg-spoiler> <sup>2</sup>";
        assert_eq!(sanitize_rich_markdown(s), s);
    }

    #[test]
    fn escapes_image_syntax_so_media_is_not_fetched() {
        let out = sanitize_rich_markdown("see ![alt](https://evil.example/x.jpg)");
        assert_eq!(out, "see \\![alt](https://evil.example/x.jpg)");
    }

    #[test]
    fn leaves_code_spans_and_fences_verbatim() {
        let span = "use `<tg-button>` carefully";
        assert_eq!(sanitize_rich_markdown(span), span);

        let fence = "```html\n<tg-button type=\"url\">x</tg-button>\n```\n";
        assert_eq!(sanitize_rich_markdown(fence), fence);

        let tilde = "~~~\n<details>x</details>\n~~~\n";
        assert_eq!(sanitize_rich_markdown(tilde), tilde);
    }

    #[test]
    fn escapes_after_a_fence_closes() {
        let s = "```\n<details>ok</details>\n```\n<details>bad</details>";
        let out = sanitize_rich_markdown(s);
        assert!(out.contains("```\n<details>ok</details>\n```"));
        assert!(out.contains("&lt;details>bad"));
    }

    #[test]
    fn unclosed_fence_swallows_the_rest_like_markdown_does() {
        let s = "```\n<tg-button>x</tg-button>";
        assert_eq!(sanitize_rich_markdown(s), s);
    }

    #[test]
    fn bare_less_than_in_prose_is_escaped_not_treated_as_a_tag() {
        assert_eq!(
            sanitize_rich_markdown("5 < 3 is false"),
            "5 &lt; 3 is false"
        );
        assert_eq!(sanitize_rich_markdown("a<3"), "a&lt;3");
    }

    #[test]
    fn tables_and_emphasis_pass_through_untouched() {
        let s = "| a | b |\n|:--|--:|\n| _x_ | **y _z_** |\n";
        assert_eq!(sanitize_rich_markdown(s), s);
    }

    /// A code span must not pair across a block boundary. Markdown resolves block
    /// structure first, so backticks in different blocks never form a span — and
    /// treating them as one copies everything between verbatim, which is exactly how a
    /// quoted web page or email could smuggle a live button past this pass.
    #[test]
    fn code_span_does_not_pair_across_a_block_boundary() {
        let button = r#"<tg-button type="callback_data" data="wipe">Confirm</tg-button>"#;
        for (label, blank) in [
            ("crlf blank line", "\r\n\r\n"),
            ("space-only blank line", "\n \n"),
            ("tab-only blank line", "\n\t\n"),
            ("plain blank line", "\n\n"),
        ] {
            let input = format!("He said `hello{blank}{button}{blank}`");
            let out = sanitize_rich_markdown(&input);
            assert!(
                !out.contains("<tg-button"),
                "{label}: raw button leaked: {out}"
            );
        }
        for (label, prefix) in [
            ("blockquote", "> "),
            ("heading", "# "),
            ("list item", "- "),
            ("ordered list item", "1. "),
            ("table row", "| "),
        ] {
            let input = format!("He said `hello\n{prefix}{button}\n{prefix}`");
            let out = sanitize_rich_markdown(&input);
            assert!(
                !out.contains("<tg-button"),
                "{label}: raw button leaked: {out}"
            );
        }
    }

    #[test]
    fn code_span_still_spans_lines_inside_one_paragraph() {
        let s = "a `code\nstill code` b";
        assert_eq!(sanitize_rich_markdown(s), s);
    }

    #[test]
    fn closing_fence_may_not_carry_an_info_string() {
        // ```js inside the block is content, so the block runs to the final fence and
        // the button stays verbatim inside the user's code sample.
        let s = "```\nline\n```js\n<tg-button>x</tg-button>\n```\n";
        assert_eq!(sanitize_rich_markdown(s), s);
    }

    #[test]
    fn anchor_with_a_disallowed_scheme_is_escaped() {
        for bad in [
            r#"<a href="javascript:alert(1)">x</a>"#,
            r#"<a href='data:text/html,y'>x</a>"#,
            r#"<a href=javascript:alert(1)>x</a>"#,
            r#"<a  href = "javascript:alert(1)" >x</a>"#,
        ] {
            let out = sanitize_rich_markdown(bad);
            assert!(out.starts_with("&lt;a"), "not escaped: {out}");
        }
        // Allowed schemes, a scheme-less target and a bare `</a>` all still pass.
        for good in [
            r#"<a href="https://example.com">x</a>"#,
            r#"<a href="mailto:a@b.c">x</a>"#,
            r#"<a href="tg://user?id=1">x</a>"#,
            r##"<a href="#anchor">x</a>"##,
            r#"<a name="chapter">x</a>"#,
        ] {
            assert_eq!(sanitize_rich_markdown(good), good);
        }
    }

    #[test]
    fn markdown_link_with_a_disallowed_scheme_is_escaped() {
        assert_eq!(
            sanitize_rich_markdown("[click](javascript:alert(1))"),
            "\\[click](javascript:alert(1))"
        );
        // Ordinary links are untouched, including scheme-less and anchor targets.
        for good in [
            "[x](https://example.com/a?b=1)",
            "[x](mailto:a@b.c)",
            "[x](#section)",
            "[x](relative/path)",
        ] {
            assert_eq!(sanitize_rich_markdown(good), good);
        }
    }

    #[test]
    fn multibyte_text_is_preserved() {
        let s = "таблица — да, <tg-button>нет</tg-button> 🎉";
        let out = sanitize_rich_markdown(s);
        assert!(out.contains("таблица — да"));
        assert!(out.contains('🎉'));
        assert!(out.contains("&lt;tg-button"));
    }
}
