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

        // A backslash escape makes the next punctuation character literal, so `` \` ``
        // does not open a code span. Copying both bytes here keeps the escaped backtick
        // out of `copy_code_span`, which would otherwise pair it with a later one and
        // copy everything between them verbatim.
        if b == b'\\' && bytes.get(i + 1).is_some_and(u8::is_ascii_punctuation) {
            out.push_str(&input[i..i + 2]);
            at_line_start = false;
            i += 2;
            continue;
        }

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
            // Inline form `[label](destination)`, and — at the start of a line — the
            // reference definition `[label]: destination`, which resolves `[x][label]`
            // elsewhere in the message. Escaping the `[` breaks the definition, so the
            // reference no longer resolves to anything.
            let dest = link_destination_at(bytes, i).or_else(|| {
                at_line_start
                    .then(|| reference_definition_at(bytes, i))
                    .flatten()
            });
            if let Some(dest) = dest {
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
            if allowed {
                // Copy the whole tag, not just the `<`. Attribute values are not
                // Markdown: a backtick inside one (`<a title="`" …>`) would otherwise be
                // read as opening a code span and copy the rest of the line verbatim.
                let end = tag_end(bytes, i);
                out.push_str(&input[i..end]);
                at_line_start = false;
                i = end;
            } else {
                out.push_str("&lt;");
                at_line_start = false;
                i += 1;
            }
            continue;
        }

        let ch_len = utf8_char_len(b);
        out.push_str(&input[i..i + ch_len]);
        at_line_start = is_line_break(b);
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
///
/// The label may contain balanced brackets (`[a[b]c]`) and the destination may contain
/// balanced parentheses or be `<…>`-wrapped, so both are scanned with a depth counter
/// rather than to the first closer.
fn link_destination_at(bytes: &[u8], i: usize) -> Option<std::ops::Range<usize>> {
    let mut depth = 0_i32;
    let mut j = i;
    loop {
        match bytes.get(j)? {
            b'\\' => j += 1, // escaped bracket is literal
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if bytes.get(j + 1) != Some(&b'(') {
        return None;
    }
    let start = j + 2;
    // A `<…>` destination may hold anything up to the `>`.
    if bytes.get(start) == Some(&b'<') {
        let mut k = start + 1;
        while bytes.get(k).is_some_and(|&c| c != b'>' && c != b'\n') {
            k += 1;
        }
        return (bytes.get(k) == Some(&b'>')).then_some(start..k + 1);
    }
    // A bare destination ends at the first whitespace or at the closing `)`. Whitespace
    // (including one line break) may then separate it from an optional title and the
    // `)` — treating a break as "not a link" left `[x](javascript:…\n)` unescaped even
    // though Markdown resolves it.
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
    // Skip whitespace, an optional quoted or parenthesised title, then whitespace again.
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
/// starting at `i`. `None` when the line is not a definition.
///
/// Without this the rich path is weaker than the legacy one it replaces: a `[x][ref]`
/// reference plus a `[ref]: javascript:…` definition renders a live link here, while the
/// legacy pipeline leaves the whole thing as inert text.
fn reference_definition_at(bytes: &[u8], i: usize) -> Option<std::ops::Range<usize>> {
    let mut j = i + 1;
    while bytes
        .get(j)
        .is_some_and(|&c| c != b']' && !is_line_break(c))
    {
        j += 1;
    }
    if bytes.get(j) != Some(&b']') || bytes.get(j + 1) != Some(&b':') {
        return None;
    }
    let start = skip_ascii_whitespace(bytes, j + 2);
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

fn skip_ascii_whitespace(bytes: &[u8], mut k: usize) -> usize {
    while bytes.get(k).is_some_and(u8::is_ascii_whitespace) {
        k += 1;
    }
    k
}

/// True when the `<a …>` tag opening at `i` has no href, or one whose scheme is allowed.
/// A closing `</a>` carries no href and is always allowed.
///
/// Attributes are walked as `name [= value]` pairs rather than by searching for the
/// substring `href`: HTML attribute names are case-insensitive (`HREF`), and a substring
/// search matches inside *other* attributes, so `<a data-href="https://ok" href="…">`
/// would be judged on the decoy and the real destination never inspected.
fn anchor_href_allowed(input: &str, bytes: &[u8], i: usize) -> bool {
    if bytes.get(i + 1) == Some(&b'/') {
        return true;
    }
    // Exclusive of the `>`; a tag may span lines, so this must not stop at a break.
    let tag_end = tag_end(bytes, i).saturating_sub(1);
    // Skip the tag name.
    let mut j = i + 1;
    while j < tag_end && !bytes[j].is_ascii_whitespace() {
        j += 1;
    }

    while j < tag_end {
        while j < tag_end && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= tag_end {
            break;
        }
        let name_start = j;
        while j < tag_end && !bytes[j].is_ascii_whitespace() && bytes[j] != b'=' {
            j += 1;
        }
        let name = &input[name_start..j];
        let mut k = j;
        while k < tag_end && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        if bytes.get(k) != Some(&b'=') {
            // Valueless attribute. `j` advanced past a non-empty name, or must be nudged
            // to guarantee progress.
            if j == name_start {
                j += 1;
            }
            continue;
        }
        k += 1;
        while k < tag_end && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        let (value_start, value_end, next) = match bytes.get(k) {
            Some(&quote @ (b'"' | b'\'')) => {
                let start = k + 1;
                let mut end = start;
                while end < tag_end && bytes[end] != quote {
                    end += 1;
                }
                (start, end, (end + 1).min(tag_end))
            }
            _ => {
                let start = k;
                let mut end = start;
                while end < tag_end && !bytes[end].is_ascii_whitespace() {
                    end += 1;
                }
                (start, end, end)
            }
        };
        if name.eq_ignore_ascii_case("href") {
            return scheme_is_allowed(&input[value_start..value_end]);
        }
        j = if next > j { next } else { j + 1 };
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

/// True for either line-break byte. Markdown treats a lone `\r` as a line ending, and
/// lone-CR text reaches us verbatim from quoted mail and older transports — handling only
/// `\n` would mean the block-boundary checks never run on such input at all.
fn is_line_break(c: u8) -> bool {
    c == b'\n' || c == b'\r'
}

/// Index just past the end of the line starting at `i`, including its terminator
/// (`\n`, `\r`, or the `\r\n` pair).
fn line_end(bytes: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < bytes.len() && !is_line_break(bytes[j]) {
        j += 1;
    }
    next_line_start(bytes, j)
}

/// Index just past the line terminator at `j`, treating `\r\n` as one terminator.
/// Returns `j` unchanged when it is not a terminator.
fn next_line_start(bytes: &[u8], j: usize) -> usize {
    match bytes.get(j) {
        Some(b'\r') if bytes.get(j + 1) == Some(&b'\n') => j + 2,
        Some(c) if is_line_break(*c) => j + 1,
        _ => j,
    }
}

/// Index of the `>` closing the tag opening at `i`, exclusive of it, or end of input.
/// A tag may span lines, so this deliberately does not stop at a line break: doing so
/// left `<a\nhref="javascript:…">` with no attributes to inspect.
fn tag_end(bytes: &[u8], i: usize) -> usize {
    let mut k = i + 1;
    while bytes.get(k).is_some_and(|&c| c != b'>') {
        k += 1;
    }
    (k + 1).min(bytes.len())
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
        if is_line_break(bytes[j]) && starts_new_block(bytes, next_line_start(bytes, j)) {
            break;
        }
        j += 1;
    }
    out.push_str(&input[i..i + run]);
    i + run
}

/// True when the line starting at `i` ends the current paragraph.
///
/// Used to bound an inline code-span scan to a single paragraph. The set below is taken
/// from CommonMark's "can interrupt a paragraph" rules rather than assembled from
/// memory — a list built by recalling cases closes the inputs you thought of and leaves
/// the hole open on the rest, which is exactly how `***`, `---`, `___` and HTML blocks
/// survived the first attempt at this function.
///
/// Erring towards `true` is safe: the backticks are then treated as ordinary text and
/// their content is escaped. Erring towards `false` copies the region verbatim, which is
/// the injection this whole module exists to prevent. So `|` (a GFM table row, which
/// interrupts in GFM but not in plain CommonMark) and any `<` (HTML block) are included.
fn starts_new_block(bytes: &[u8], i: usize) -> bool {
    if i >= bytes.len() {
        return true;
    }
    let mut j = i;
    while bytes.get(j).is_some_and(|c| matches!(c, b' ' | b'\t')) {
        j += 1;
    }
    match bytes.get(j) {
        // Blank line ends the paragraph. `\r` covers CRLF input, which reaches us
        // verbatim from quoted email and web content.
        None | Some(b'\n') | Some(b'\r') => true,
        // Block quotation, ATX heading, GFM table row, HTML block.
        Some(b'>') | Some(b'#') | Some(b'|') | Some(b'<') => true,
        Some(b'`') => fence_at(bytes, i).is_some(),
        // `~` opens a fence but is not a list bullet.
        Some(b'~') => fence_at(bytes, i).is_some() || thematic_break_at(bytes, j),
        // `-` and `=` runs are also setext heading underlines; `-`, `*` and `_` runs are
        // thematic breaks, which may carry spaces between the markers (`- - -`).
        Some(b'-') | Some(b'*') | Some(b'_') | Some(b'=') => {
            thematic_break_at(bytes, j) || setext_underline_at(bytes, j) || is_bullet(bytes, j)
        }
        Some(b'+') => is_bullet(bytes, j),
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

/// A list bullet: `-`, `*` or `+` followed by a space or tab.
fn is_bullet(bytes: &[u8], j: usize) -> bool {
    matches!(bytes.get(j), Some(b'-') | Some(b'*') | Some(b'+'))
        && matches!(bytes.get(j + 1), Some(b' ') | Some(b'\t'))
}

/// CommonMark thematic break: three or more `-`, `*` or `_`, all the same, with only
/// spaces and tabs between them and nothing else on the line.
fn thematic_break_at(bytes: &[u8], j: usize) -> bool {
    let marker = match bytes.get(j) {
        Some(&c @ (b'-' | b'*' | b'_')) => c,
        _ => return false,
    };
    let mut count = 0;
    let mut k = j;
    while let Some(&c) = bytes.get(k) {
        match c {
            c if c == marker => count += 1,
            b' ' | b'\t' => {}
            b'\n' | b'\r' => break,
            _ => return false,
        }
        k += 1;
    }
    count >= 3
}

/// Setext heading underline: a run of `=` or a run of `-`, optionally followed by spaces.
/// Only `=` needs handling separately — a `-` run of three or more is already a thematic
/// break, but `-` or `--` alone is a setext underline and nothing else.
fn setext_underline_at(bytes: &[u8], j: usize) -> bool {
    let marker = match bytes.get(j) {
        Some(&c @ (b'=' | b'-')) => c,
        _ => return false,
    };
    let mut k = j;
    while bytes.get(k) == Some(&marker) {
        k += 1;
    }
    if k == j {
        return false;
    }
    while bytes.get(k).is_some_and(|c| matches!(c, b' ' | b'\t')) {
        k += 1;
    }
    matches!(bytes.get(k), None | Some(b'\n') | Some(b'\r'))
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

    /// The block-starter set is taken from CommonMark, not from recall. These are the
    /// cases a from-memory list misses — each one interrupts a paragraph, so a code span
    /// must not pair across it.
    #[test]
    fn code_span_stops_at_every_paragraph_interrupting_construct() {
        let button = r#"<tg-button type="callback_data" data="wipe">Confirm</tg-button>"#;
        // A ``` separator is deliberately absent: it puts the button inside a real
        // fenced code block, where CommonMark renders it as escaped text rather than
        // live HTML, so copying it verbatim is correct.
        for separator in [
            "***", "---", "___", "* * *", "- - -", "___ _", "===", "--", "<div>", "<table>",
            "> quote", "# head", "- item", "1. item", "+ item", "| a |",
        ] {
            let input = format!("He said `hello\n{separator}\n{button}\n{separator}\n`");
            let out = sanitize_rich_markdown(&input);
            assert!(
                !out.contains("<tg-button"),
                "separator {separator:?} let a raw button through: {out}"
            );
        }
    }

    /// A four-space indent is *not* a paragraph interrupter in CommonMark (it only opens
    /// an indented code block after a blank line), and `~ x` is not a list bullet. Erring
    /// towards "new block" here would needlessly break ordinary multi-line code spans.
    #[test]
    fn code_span_survives_lines_that_do_not_interrupt_a_paragraph() {
        for continuation in ["    indented", "~ not a bullet", "plain text"] {
            let input = format!("a `code\n{continuation}\nmore` b");
            assert_eq!(sanitize_rich_markdown(&input), input);
        }
    }

    /// A lone `\r` is a line ending in Markdown, and lone-CR text arrives verbatim from
    /// quoted mail. Matching only `\n` meant the block-boundary check never ran at all on
    /// such input, so every separator below let a raw button through.
    #[test]
    fn code_span_respects_block_boundaries_with_lone_cr_line_endings() {
        let button = r#"<tg-button type="callback_data" data="wipe">Tap</tg-button>"#;
        for separator in ["\r> q\r> ", "\r\r", "\r# h\r", "\r---\r", "\r<div>\r"] {
            let input = format!("He said `hello{separator}{button}{separator}`");
            let out = sanitize_rich_markdown(&input);
            assert!(
                !out.contains("<tg-button"),
                "lone-CR separator {separator:?} leaked: {out}"
            );
        }
    }

    /// `` \` `` is a literal backtick, not the start of a code span, and a backtick inside
    /// an attribute value is not Markdown at all. Both used to open a span that then
    /// copied everything up to the next backtick verbatim.
    #[test]
    fn escaped_and_attribute_backticks_do_not_open_a_code_span() {
        let button = r#"<tg-button type="callback_data" data="x">Tap</tg-button>"#;
        let escaped = format!("a \\` {button} \\` b");
        assert!(!sanitize_rich_markdown(&escaped).contains("<tg-button"));

        let in_attribute = format!(r#"<a title="`" href="https://ok">t</a> {button} `x`"#);
        assert!(!sanitize_rich_markdown(&in_attribute).contains("<tg-button"));
    }

    /// An entity decoding to whitespace, and an entity we cannot decode at all, both used
    /// to hide the scheme behind them — one by making the destination "start with a
    /// space", the other by truncating the scan.
    #[test]
    fn entities_cannot_hide_a_scheme() {
        for bad in [
            "&#32;javascript:alert(1)",
            "&#9;javascript:alert(1)",
            "java&#10;script:alert(1)",
            "&Tab;javascript:alert(1)",
            "java&NewLine;script:alert(1)",
        ] {
            assert!(!scheme_is_allowed(bad), "scheme slipped through: {bad}");
        }
        // Ordinary destinations still pass, including a query string full of `&`.
        assert!(scheme_is_allowed("https://example.com/s?a=1&b=2&c=3"));
    }

    /// HTML allows a line break inside a tag; stopping the attribute scan at one left the
    /// href uninspected and the tag emitted live.
    #[test]
    fn tag_spanning_lines_still_has_its_href_checked() {
        let out = sanitize_rich_markdown("<a\nhref=\"javascript:alert(1)\">t</a>");
        assert!(out.starts_with("&lt;a"), "not escaped: {out}");
        let ok = "<a\nhref=\"https://example.com\">t</a>";
        assert_eq!(sanitize_rich_markdown(ok), ok);
    }

    /// Whitespace — including one line break — may sit between a destination and its
    /// closing `)`, and a title may sit in between. Treating that as "not a link" left
    /// the destination unchecked.
    #[test]
    fn destination_is_checked_across_whitespace_and_titles() {
        for bad in [
            "[x](javascript:alert(1)\n)",
            "[x](javascript:alert(1) )",
            "[x](javascript:alert(1) \"title\")",
        ] {
            let out = sanitize_rich_markdown(bad);
            assert!(out.starts_with("\\["), "not escaped: {out}");
        }
        for good in [
            "[x](https://example.com \"title\")",
            "[x](https://example.com)",
        ] {
            assert_eq!(sanitize_rich_markdown(good), good);
        }
    }

    /// A reference definition carries the destination for `[x][ref]` elsewhere in the
    /// message. It was not inspected at all, making the rich path weaker than the legacy
    /// pipeline it replaces — which leaves the same input inert.
    #[test]
    fn reference_definitions_are_scheme_checked() {
        let out = sanitize_rich_markdown("[x][ref]\n\n[ref]: javascript:alert(1)");
        assert!(out.contains("\\[ref]:"), "definition not escaped: {out}");
        let ok = "[x][ref]\n\n[ref]: https://example.com";
        assert_eq!(sanitize_rich_markdown(ok), ok);
    }

    /// Boundaries of the block-starter helpers, which no test pinned before: mutating any
    /// of these left all 130 tests green.
    #[test]
    fn block_starter_boundaries_are_pinned() {
        // An ordered-list marker needs a following space.
        assert!(starts_new_block(b"1. item\n", 0));
        assert!(!starts_new_block(b"1.item\n", 0));
        // A thematic break needs three markers, not two.
        assert!(thematic_break_at(b"***\n", 0));
        assert!(!thematic_break_at(b"**\n", 0));
        // A control character inside a destination is dropped, not kept.
        assert!(!scheme_is_allowed("java\u{1}script:alert(1)"));
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

    /// HTML attribute names are case-insensitive, and a substring search for `href`
    /// matches inside other attribute names and values — both let a disallowed scheme
    /// through on a tag we then emit live.
    #[test]
    fn anchor_href_is_found_regardless_of_case_or_decoy_attributes() {
        for bad in [
            r#"<a HREF="javascript:alert(1)">x</a>"#,
            r#"<A Href="javascript:alert(1)">x</A>"#,
            r#"<a data-href="https://ok.example" href="javascript:alert(1)">x</a>"#,
            r#"<a title="href=x" href="javascript:alert(1)">x</a>"#,
            r#"<a class="a" data-href="https://ok" HREF='javascript:alert(1)'>x</a>"#,
        ] {
            let out = sanitize_rich_markdown(bad);
            assert!(
                out.starts_with("&lt;a") || out.starts_with("&lt;A"),
                "not escaped: {out}"
            );
        }
        // A decoy alone, with no real href, is still a plain anchor.
        let benign = r#"<a data-href="https://ok.example">x</a>"#;
        assert_eq!(sanitize_rich_markdown(benign), benign);
    }

    /// The scheme check must hold on what the parser resolves, not on the literal bytes:
    /// `<…>` wrappers, entity-encoded colons and embedded whitespace all hide a scheme.
    #[test]
    fn scheme_check_sees_through_wrappers_entities_and_whitespace() {
        for bad in [
            "[x](<javascript:alert(1)>)",
            "[x](javascript&#58;alert(1))",
            "[x](javascript&#x3a;alert(1))",
            "[x](javascript&colon;alert(1))",
            "[a[b]c](javascript:alert(1))",
        ] {
            let out = sanitize_rich_markdown(bad);
            assert!(out.starts_with("\\["), "not escaped: {out}");
        }
        for bad in [
            "<a href=\"java\tscript:alert(1)\">x</a>",
            "<a href=\"<javascript:alert(1)>\">x</a>",
        ] {
            let out = sanitize_rich_markdown(bad);
            assert!(out.starts_with("&lt;a"), "not escaped: {out}");
        }
        // Balanced parentheses in an ordinary destination still parse, and stay allowed.
        let ok = "[x](https://example.com/a_(b)_c)";
        assert_eq!(sanitize_rich_markdown(ok), ok);
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
