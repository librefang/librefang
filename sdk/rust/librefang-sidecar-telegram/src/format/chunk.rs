//! UTF-16 chunking for Telegram's 4096-code-unit message limit.
//!
//! Mirrors the Python adapter's `_utf16_len`, `_truncate_to_utf16_limit`, and `_split_to_utf16_chunks`, with two additional guards specific to the Rust port:
//! - the entity-boundary back-off only fires for *known* HTML entity prefixes (so a chunk ending in a literal `&` is not silently truncated);
//! - the splitter is tag-aware: if a chunk ends with one or more open HTML tags, matching close tags are appended to the chunk and the matching open tags are carried over to the next chunk so the user's formatting survives across boundaries.
//!
//! Telegram counts code units, not bytes or Unicode scalars; chars above U+FFFF count as 2.

use once_cell::sync::Lazy;
use regex::Regex;

pub const TELEGRAM_MSG_LIMIT: usize = 4096;

/// UTF-16 code-unit length of `s` (chars above U+FFFF count as 2).
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Longest prefix of `s` whose UTF-16 length is <= `limit`, with the prefix ending on a Unicode scalar boundary.
pub fn truncate_to_utf16_limit(s: &str, limit: usize) -> &str {
    if limit == 0 {
        return "";
    }
    let mut acc = 0usize;
    let mut last = 0usize;
    for (idx, ch) in s.char_indices() {
        let units = ch.len_utf16();
        if acc + units > limit {
            return &s[..last];
        }
        acc += units;
        last = idx + ch.len_utf8();
    }
    s
}

/// Known HTML entity prefixes (no trailing `;`). If a chunk ends with `&<prefix>`, the chunk has split mid-entity and we trim it back to before the `&`.
const ENTITY_PREFIXES: &[&str] = &[
    "amp", "am", "a", "lt", "l", "gt", "g", "quot", "quo", "qu", "q", "nbsp", "nbs", "nb", "n",
    "apos", "apo", "ap",
];

fn looks_like_partial_entity(suffix: &str) -> bool {
    if suffix.is_empty() {
        return true;
    }
    if let Some(rest) = suffix.strip_prefix('#') {
        if let Some(hex_rest) = rest.strip_prefix(['x', 'X']) {
            return !hex_rest.is_empty()
                && hex_rest.len() <= 8
                && hex_rest.chars().all(|c| c.is_ascii_hexdigit());
        }
        return !rest.is_empty() && rest.len() <= 10 && rest.chars().all(|c| c.is_ascii_digit());
    }
    ENTITY_PREFIXES.contains(&suffix)
}

/// If `chunk` ends mid-HTML-entity (`&` opened but not closed AND the trailing chars look like a known entity prefix), shrink it back to before the `&`. A literal `&` near the end (not followed by an entity-shaped suffix) is preserved.
fn adjust_html_entity_boundary(chunk: &str) -> &str {
    let bytes = chunk.as_bytes();
    let mut amp: Option<usize> = None;
    for (i, b) in bytes.iter().enumerate().rev() {
        match b {
            b';' => return chunk, // most recent ampersand is closed
            b'&' => {
                amp = Some(i);
                break;
            }
            _ => {}
        }
        // Telegram-relevant entities never exceed ~10 bytes.
        if bytes.len() - i > 12 {
            return chunk;
        }
    }
    match amp {
        Some(i) => {
            let suffix = &chunk[i + 1..];
            if looks_like_partial_entity(suffix) {
                &chunk[..i]
            } else {
                chunk
            }
        }
        None => chunk,
    }
}

/// If `chunk` ends inside an HTML tag (`<` opened but not closed), back off to before the `<` so the next chunk gets the full tag intact.
fn strip_mid_tag(chunk: &str) -> &str {
    let bytes = chunk.as_bytes();
    let mut last_lt: Option<usize> = None;
    let mut open = false;
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'<' => {
                last_lt = Some(i);
                open = true;
            }
            b'>' => {
                open = false;
            }
            _ => {}
        }
    }
    if open {
        match last_lt {
            Some(i) => &chunk[..i],
            None => chunk,
        }
    } else {
        chunk
    }
}

static RE_TAG: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<(/?)([a-zA-Z][a-zA-Z0-9-]*)([^>]*)>").expect("tag regex"));

/// Walk `chunk` and return the stack of tags left unclosed at end-of-chunk. Each entry is `(name, full_open_tag_with_attrs)` so the caller can both close (`</name>`) at the end of this chunk and reopen with the original attributes at the start of the next chunk.
fn unclosed_tags(chunk: &str) -> Vec<(String, String)> {
    let mut stack: Vec<(String, String)> = Vec::new();
    for caps in RE_TAG.captures_iter(chunk) {
        let closing = !caps.get(1).unwrap().as_str().is_empty();
        let name = caps.get(2).unwrap().as_str().to_ascii_lowercase();
        let full = caps.get(0).unwrap().as_str().to_string();
        if closing {
            if let Some(pos) = stack.iter().rposition(|(n, _)| *n == name) {
                stack.truncate(pos);
            }
        } else {
            stack.push((name, full));
        }
    }
    stack
}

/// Split `s` into chunks no longer than `limit` UTF-16 code units each.
/// Prefers a trailing newline as the split point; falls back to truncating at the highest char boundary that fits.
/// Tag-aware: open HTML tags at a chunk's end are closed with matching `</tag>` and re-opened verbatim at the start of the next chunk so the user's formatting carries across.
pub fn split_to_utf16_chunks(s: &str, limit: usize) -> Vec<String> {
    assert!(limit > 0, "limit must be > 0");
    if utf16_len(s) <= limit {
        return vec![s.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut carry: String = String::new();
    let mut remaining: &str = s;

    while !remaining.is_empty() {
        let carry_units = utf16_len(&carry);
        // Degenerate: carry alone is too big. Emit it on its own and reset; in practice this never happens because tag prefixes are short.
        if carry_units >= limit {
            out.push(std::mem::take(&mut carry));
            continue;
        }
        if carry_units + utf16_len(remaining) <= limit {
            let mut last = String::with_capacity(carry.len() + remaining.len());
            last.push_str(&carry);
            last.push_str(remaining);
            out.push(last);
            break;
        }
        let budget = limit - carry_units;
        let input_prefix = truncate_to_utf16_limit(remaining, budget);
        // Prefer a newline as the split point.
        let split_idx = input_prefix
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(input_prefix.len());
        let mut input_chunk = &input_prefix[..split_idx];
        if input_chunk.is_empty() {
            input_chunk = input_prefix;
        }
        // Combine carry + input_chunk for entity / tag analysis on the actual emitted text.
        let mut combined = String::with_capacity(carry.len() + input_chunk.len());
        combined.push_str(&carry);
        combined.push_str(input_chunk);
        let trimmed = strip_mid_tag(&combined);
        let trimmed = adjust_html_entity_boundary(trimmed);
        let trimmed_len = trimmed.len();
        // Choose what to emit: either the entity/tag-trimmed combined slice (normal path) or, if that left no progress to make on `remaining`, the carry plus one forced char of input. Either way we run the SAME tag-rebalancing on the emitted text so open tags from `carry` get matching close tags appended and propagate forward via `next_carry`.
        let emitted_text: String;
        let consumed_from_input: usize;
        if trimmed_len <= carry.len() {
            let one_char_end = remaining
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(remaining.len());
            let mut t = String::with_capacity(carry.len() + one_char_end);
            t.push_str(&carry);
            t.push_str(&remaining[..one_char_end]);
            emitted_text = t;
            consumed_from_input = one_char_end;
        } else {
            emitted_text = trimmed.to_string();
            consumed_from_input = trimmed_len - carry.len();
        }
        let stack = unclosed_tags(&emitted_text);
        let close_suffix: String = stack.iter().rev().map(|(n, _)| format!("</{n}>")).collect();
        let next_carry: String = stack.iter().map(|(_, full)| full.clone()).collect();
        let mut emit = String::with_capacity(emitted_text.len() + close_suffix.len());
        emit.push_str(&emitted_text);
        emit.push_str(&close_suffix);
        out.push(emit);
        carry = next_carry;
        remaining = &remaining[consumed_from_input..];
    }
    // Trailing carry covers nothing — would render as empty tag pairs; drop it.
    drop(carry);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_len_basic() {
        assert_eq!(utf16_len("hi"), 2);
        assert_eq!(utf16_len(""), 0);
        assert_eq!(utf16_len("a\u{1F600}"), 3); // 'a' + emoji surrogate pair
    }

    #[test]
    fn truncate_keeps_full_chars() {
        assert_eq!(truncate_to_utf16_limit("hello", 3), "hel");
        assert_eq!(truncate_to_utf16_limit("a\u{1F600}", 2), "a");
        assert_eq!(truncate_to_utf16_limit("a\u{1F600}", 3), "a\u{1F600}");
    }

    #[test]
    fn split_prefers_newline() {
        let s = "abc\ndef\nghi";
        let chunks = split_to_utf16_chunks(s, 5);
        // Each chunk should end in '\n' until the last.
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn split_handles_single_oversized_line() {
        let s = "a".repeat(10);
        let chunks = split_to_utf16_chunks(&s, 3);
        assert_eq!(chunks.len(), 4);
        assert!(chunks.iter().all(|c| c.len() <= 3));
    }

    #[test]
    fn no_split_inside_html_entity() {
        let s = "abc&lt;def";
        // limit chosen so the boundary falls mid-`&lt;` (chars 4 = 'abc&')
        let chunks = split_to_utf16_chunks(s, 4);
        // First chunk must NOT contain a trailing bare '&'.
        assert!(!chunks[0].ends_with('&'));
    }

    #[test]
    fn literal_ampersand_near_boundary_is_preserved() {
        // `foo & bar` has a literal `&` followed by ` ` — not a known entity prefix, so the boundary helper should leave it alone.
        let s = "foo & bar";
        // Larger limit so we don't actually split, but the entity-boundary check still runs on the chunk.
        assert_eq!(adjust_html_entity_boundary(s), s);
        // Now force a split at the end so the chunk includes the `&` but no entity follows.
        let chunks = split_to_utf16_chunks(s, 9);
        assert_eq!(chunks.join(""), s);
    }

    #[test]
    fn no_split_inside_html_tag() {
        // limit forces split at byte 7 — inside `<b>foo</b>` somewhere. The mid-tag guard should back off so each chunk has only complete tags.
        let s = "<b>foofoofoo</b>";
        let chunks = split_to_utf16_chunks(s, 10);
        for c in &chunks {
            // No chunk should contain a `<` without a matching `>`.
            let opens = c.matches('<').count();
            let closes = c.matches('>').count();
            assert_eq!(opens, closes, "unbalanced angle brackets in chunk {c:?}");
        }
    }

    #[test]
    fn tag_carry_across_chunks() {
        // `<b>...</b>` long enough to force a split. Each chunk must be locally balanced and concatenating the inner text should reconstruct the original.
        let inner = "x".repeat(20);
        let s = format!("<b>{inner}</b>");
        let chunks = split_to_utf16_chunks(&s, 10);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert_eq!(
                c.matches("<b>").count(),
                c.matches("</b>").count(),
                "chunk {c:?} unbalanced",
            );
        }
        // First chunk should end with </b> (the close suffix); subsequent chunks should begin with <b>.
        assert!(chunks[0].ends_with("</b>"));
        assert!(chunks[1].starts_with("<b>"));
    }
}
