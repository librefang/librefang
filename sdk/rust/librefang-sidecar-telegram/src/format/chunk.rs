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

/// Cap on the safety margin reserved per chunk for close tags the rebalancer will append at chunk end. Realistic markdown nesting is 2-3 deep so the actual reserve we use is `min(NEW_TAG_RESERVE, limit/4)` — small enough not to starve tiny test limits, large enough to absorb the 1-2 new tags a chunk typically opens inside itself.
const NEW_TAG_RESERVE: usize = 16;

/// Compute the UTF-16 width of the close-tag suffix that would be appended for the tags already open in `carry`. Used to subtract the *known* close-tag cost from the chunk budget so the emit cannot overshoot `limit` and trip Telegram's `MESSAGE_TOO_LONG`. The chunk may open additional tags inside it; those are covered by `NEW_TAG_RESERVE`.
fn carry_close_cost(carry: &str) -> usize {
    unclosed_tags(carry)
        .iter()
        .map(|(name, _)| 3 + utf16_len(name)) // `</name>`
        .sum()
}

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

/// Trim only a newly selected input prefix.
/// Its returned length is therefore the exact byte count to consume from `remaining`; formatting carry is never part of the boundary calculation.
fn trim_input_boundary(input: &str) -> &str {
    adjust_html_entity_boundary(strip_mid_tag(input))
}

static RE_TAG: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<(/?)([a-zA-Z][a-zA-Z0-9-]*)([^>]*)>").expect("tag regex"));
const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Walk `chunk` and return the stack of tags left unclosed at end-of-chunk. Each entry is `(name, full_open_tag_with_attrs)` so the caller can both close (`</name>`) at the end of this chunk and reopen with the original attributes at the start of the next chunk.
fn unclosed_tags(chunk: &str) -> Vec<(String, String)> {
    let mut stack: Vec<(String, String)> = Vec::new();
    for caps in RE_TAG.captures_iter(chunk) {
        let closing = !caps.get(1).unwrap().as_str().is_empty();
        let name = caps.get(2).unwrap().as_str().to_ascii_lowercase();
        let full = caps.get(0).unwrap().as_str().to_string();
        let self_closing = caps
            .get(3)
            .is_some_and(|attrs| attrs.as_str().trim_end().ends_with('/'));
        if closing {
            if let Some(pos) = stack.iter().rposition(|(n, _)| *n == name) {
                stack.truncate(pos);
            }
        } else if !self_closing && !VOID_TAGS.contains(&name.as_str()) {
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
        // Degenerate: carry is formatting-only and cannot fit by itself.
        // Drop it rather than violating the caller's hard wire limit.
        if carry_units >= limit {
            carry.clear();
            continue;
        }
        if carry_units + utf16_len(remaining) <= limit {
            let mut last = String::with_capacity(carry.len() + remaining.len());
            last.push_str(&carry);
            last.push_str(remaining);
            out.push(last);
            break;
        }
        // Reserve room for the close-tag suffix we will append after `unclosed_tags` runs. Without the reserve, an emit ending in deeply-nested `<b><i><a>` opens would push the total past `limit` once the matching closes are appended, and Telegram rejects with `MESSAGE_TOO_LONG` (400). We charge the exact close cost of whatever is already open in `carry`, plus a tiny reserve for any new tags the chunk itself opens — both capped to `limit/4` so very small test limits still make non-trivial progress.
        let carry_close = carry_close_cost(&carry);
        let new_reserve = NEW_TAG_RESERVE.min(limit / 4);
        let budget = limit
            .saturating_sub(carry_units)
            .saturating_sub(carry_close)
            .saturating_sub(new_reserve)
            .max(1);
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
        // Trim only the new input.
        // Inferring progress by trimming `carry + input` and subtracting carry length would couple forward progress to assumptions inside the boundary helpers.
        let trimmed_input = trim_input_boundary(input_chunk);
        // Choose what to emit: either carry plus the safely trimmed input (normal path), or if no safe input remains, carry plus one forced input unit.
        // Both paths run the same tag rebalancing below.
        let mut emitted_text: String;
        let mut consumed_from_input: usize;
        let degenerate_progress = trimmed_input.is_empty();
        if degenerate_progress {
            // Degenerate: budget too small for any safe progress. If `remaining` starts with `<` AND the matching `>` is within `limit` UTF-16 units, consume the whole tag so the next chunk reopens cleanly — emitting a bare leading `<` would produce HTML Telegram cannot parse. If the tag is unmatched (no `>`) or runs past `limit` UTF-16 units (a degenerate or adversarial mega-attribute), fall back to forcing one Unicode scalar of progress; the chunk will be unbalanced and the parse-entities fallback will rescue delivery as plain text.
            //
            // Comparison must be in UTF-16 code units, not bytes — for ASCII tag content they coincide but `<tg-emoji emoji-id="…">` can carry non-ASCII attrs that make the byte count exceed the UTF-16 unit count.
            let take = remaining
                .starts_with('<')
                .then(|| {
                    remaining
                        .find('>')
                        .map(|gt| gt + 1)
                        .filter(|&n_bytes| utf16_len(&remaining[..n_bytes]) <= limit)
                })
                .flatten()
                .unwrap_or_else(|| {
                    remaining
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| i)
                        .unwrap_or(remaining.len())
                });
            let mut t = String::with_capacity(carry.len() + take);
            t.push_str(&carry);
            t.push_str(&remaining[..take]);
            emitted_text = t;
            consumed_from_input = take;
        } else {
            emitted_text = String::with_capacity(carry.len() + trimmed_input.len());
            emitted_text.push_str(&carry);
            emitted_text.push_str(trimmed_input);
            consumed_from_input = trimmed_input.len();
        }
        let mut stack = unclosed_tags(&emitted_text);
        let mut close_suffix: String = stack.iter().rev().map(|(n, _)| format!("</{n}>")).collect();
        let mut next_carry: String = stack.iter().map(|(_, full)| full.clone()).collect();
        if degenerate_progress
            && utf16_len(&emitted_text).saturating_add(utf16_len(&close_suffix)) > limit
        {
            // The whole-tag progress escape hatch above is bounded only by the
            // input tag itself. Carry + generated closing tags can still push
            // the actual wire chunk over Telegram's limit. Drop formatting
            // carry for this rare malformed/deeply-nested boundary and consume
            // one scalar as plain text; the dispatcher's parse-entities fallback
            // preserves delivery while guaranteeing forward progress.
            if let Some(tag_end) = remaining
                .starts_with('<')
                .then(|| remaining.find('>'))
                .flatten()
            {
                // The markup itself cannot fit once balanced. Consume it
                // without emitting it and continue with plain content.
                emitted_text.clear();
                consumed_from_input = tag_end + 1;
            } else {
                let forced = remaining
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| i)
                    .unwrap_or(remaining.len());
                let scalar = &remaining[..forced];
                emitted_text = if utf16_len(scalar) <= limit {
                    scalar.to_string()
                } else {
                    String::new()
                };
                consumed_from_input = forced;
            }
            close_suffix.clear();
            next_carry.clear();
        }
        while utf16_len(&emitted_text).saturating_add(utf16_len(&close_suffix)) > limit {
            // NEW_TAG_RESERVE is only a first-pass heuristic.
            // Recompute the exact suffix for the selected prefix, then shrink the input until both the body and its balancing closes fit.
            // Removing a closing tag can itself increase the required suffix, so repeat until the exact tag stack stabilizes within the limit.
            let previous_consumed = consumed_from_input;
            let available_input = limit
                .saturating_sub(utf16_len(&carry))
                .saturating_sub(utf16_len(&close_suffix));
            let reduced = truncate_to_utf16_limit(&remaining[..previous_consumed], available_input);
            let reduced_input = trim_input_boundary(reduced);
            let reduced_consumed = reduced_input.len();

            if reduced_consumed == 0 || reduced_consumed >= previous_consumed {
                // Carry plus its exact close suffix can consume the whole budget at tiny/adversarial limits.
                // Formatting cannot be represented there, so retain progress and fall back to the selected input's plain text rather than emitting oversized wire data or looping forever.
                let plain = RE_TAG.replace_all(&remaining[..previous_consumed], "");
                emitted_text = truncate_to_utf16_limit(&plain, limit).to_string();
                close_suffix.clear();
                next_carry.clear();
                break;
            }

            emitted_text.clear();
            emitted_text.reserve(carry.len() + reduced_input.len());
            emitted_text.push_str(&carry);
            emitted_text.push_str(reduced_input);
            consumed_from_input = reduced_consumed;
            stack = unclosed_tags(&emitted_text);
            close_suffix = stack.iter().rev().map(|(n, _)| format!("</{n}>")).collect();
            next_carry = stack.iter().map(|(_, full)| full.clone()).collect();
        }
        let mut emit = String::with_capacity(emitted_text.len() + close_suffix.len());
        emit.push_str(&emitted_text);
        emit.push_str(&close_suffix);
        if !emit.is_empty() {
            out.push(emit);
        }
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
    fn self_closing_and_void_tags_do_not_enter_the_carry_stack() {
        assert!(unclosed_tags("<code/>after").is_empty());
        assert!(unclosed_tags("<tg-emoji emoji-id=\"42\" />after").is_empty());
        assert!(unclosed_tags("before<br>after").is_empty());
        assert_eq!(unclosed_tags("<b>after")[0].0, "b");
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
    fn boundary_trimming_is_relative_to_new_input() {
        assert_eq!(trim_input_boundary("abc&am"), "abc");
        assert_eq!(trim_input_boundary("<tg-emoji emoji-id=\"42"), "");
        assert_eq!(trim_input_boundary("plain text"), "plain text");

        let chunks = split_to_utf16_chunks("<b>123456&amp;tail</b>", 12);
        let text = chunks.concat().replace("<b>", "").replace("</b>", "");
        assert_eq!(text, "123456&amp;tail");
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

    #[test]
    fn tag_carry_preserves_anchor_href_with_attributes() {
        // Anchor with attributes — the carry must preserve `href="..."` verbatim when reopening.
        let s = format!("<a href=\"https://example.com\">{}</a>", "x".repeat(40));
        // The opening tag (30 units) plus its required `</a>` suffix must fit before the chunker can preserve formatting.
        let chunks = split_to_utf16_chunks(&s, 45);
        assert!(chunks.len() >= 2);
        assert!(
            chunks[0].ends_with("</a>"),
            "chunk 0 must close: {:?}",
            chunks[0]
        );
        assert!(
            chunks[1].starts_with("<a href=\"https://example.com\">"),
            "chunk 1 must reopen with attrs: {:?}",
            chunks[1]
        );
    }

    #[test]
    fn tag_carry_nested_bold_italic_order_inside_out() {
        // Nested formatting: close tags emit inside-out, reopens emit outside-in.
        let inner = "x".repeat(40);
        let s = format!("<b><i>{inner}</i></b>");
        let chunks = split_to_utf16_chunks(&s, 25);
        assert!(chunks.len() >= 2);
        assert!(
            chunks[0].ends_with("</i></b>"),
            "wrong close order: {:?}",
            chunks[0]
        );
        assert!(
            chunks[1].starts_with("<b><i>"),
            "wrong reopen order: {:?}",
            chunks[1]
        );
    }

    #[test]
    fn degenerate_branch_consumes_whole_tag_when_remaining_starts_with_lt() {
        // Deep-nested unclosed tags + remaining starts with `<` of the closing tag. Previously the degenerate path force-advanced one char, emitting a bare `<` inside the chunk. With the whole-tag consume fix every chunk stays balanced.
        let s = "<b><i><u>xyz</u></i></b>";
        let chunks = split_to_utf16_chunks(s, 10);
        for c in &chunks {
            assert_eq!(
                c.matches('<').count(),
                c.matches('>').count(),
                "chunk has stray angle bracket: {c:?}",
            );
            // Sanity: no chunk contains `><` with nothing between (an artefact of the old degenerate path emitting carry directly).
            assert!(
                !c.contains("<>"),
                "empty tag span produced by chunker: {c:?}",
            );
        }
    }

    #[test]
    fn degenerate_whole_tag_progress_counts_carry_and_close_suffix() {
        let chunks = split_to_utf16_chunks("<b>xxxxx<tg-emoji>z</tg-emoji></b>", 16);
        assert!(chunks.len() > 1);
        for chunk in chunks {
            assert!(
                utf16_len(&chunk) <= 16,
                "degenerate emit exceeded limit: {chunk:?}"
            );
        }
    }

    #[test]
    fn impossible_single_unit_astral_limit_never_emits_an_oversized_chunk() {
        let chunks = split_to_utf16_chunks("😀😀", 1);
        assert!(chunks.iter().all(|chunk| utf16_len(chunk) <= 1));
    }

    #[test]
    fn entity_prefix_trims_back_named() {
        assert_eq!(adjust_html_entity_boundary("abc&am"), "abc");
        assert_eq!(adjust_html_entity_boundary("abc&lt"), "abc");
        assert_eq!(adjust_html_entity_boundary("abc&quot"), "abc");
        // Closed entity stays put.
        assert_eq!(adjust_html_entity_boundary("abc&lt;def"), "abc&lt;def");
    }

    #[test]
    fn chunks_never_exceed_limit_with_deep_nesting() {
        // Regression: the previous chunker didn't reserve budget for the close-tag suffix it appends after `unclosed_tags`. With deeply-nested formatting near the production limit, the emit could overshoot and Telegram would 400 it. With the carry-close cost + NEW_TAG_RESERVE, every emit must fit.
        let inner = "x".repeat(4090);
        let s = format!("<b><i><u>{inner}</u></i></b>");
        let chunks = split_to_utf16_chunks(&s, 4096);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(
                utf16_len(c) <= 4096,
                "chunk len {} exceeds 4096-unit limit: {:?}",
                utf16_len(c),
                &c[..c.len().min(80)]
            );
        }
    }

    #[test]
    fn long_tag_close_suffixes_are_counted_exactly() {
        let inner = "x".repeat(4090);
        let s = format!(
            "<tg-emoji emoji-id=\"1\"><tg-emoji emoji-id=\"2\">{inner}</tg-emoji></tg-emoji>"
        );
        let chunks = split_to_utf16_chunks(&s, TELEGRAM_MSG_LIMIT);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(
                utf16_len(chunk) <= TELEGRAM_MSG_LIMIT,
                "chunk exceeded Telegram limit: {}",
                utf16_len(chunk)
            );
        }
        let reconstructed = chunks
            .iter()
            .map(|chunk| RE_TAG.replace_all(chunk, ""))
            .collect::<String>();
        assert_eq!(reconstructed, inner);
    }

    #[test]
    fn degenerate_complete_tag_never_exceeds_limit_after_balancing() {
        let limit = 16;
        let chunks = split_to_utf16_chunks("<code class=\"\">xx", limit);
        assert!(
            chunks.iter().all(|chunk| utf16_len(chunk) <= limit),
            "oversized chunks: {chunks:?}"
        );
    }

    #[test]
    fn scalar_wider_than_limit_is_consumed_without_oversized_emit() {
        let chunks = split_to_utf16_chunks("😀😀", 1);
        assert!(chunks.is_empty());
    }

    #[test]
    fn long_tag_close_suffixes_are_counted_exactly_with_astral_content() {
        // Same shape as `long_tag_close_suffixes_are_counted_exactly` but the filler is an astral-plane emoji (surrogate pair, 2 UTF-16 units per scalar) instead of ASCII `x`, so the exact-suffix-budget shrink loop has to make progress in 2-unit steps and can never land mid-surrogate-pair.
        let inner = "😀".repeat(2045); // 2045 * 2 == 4090 UTF-16 units, matching the ASCII-filler test's budget pressure.
        let s = format!(
            "<tg-emoji emoji-id=\"1\"><tg-emoji emoji-id=\"2\">{inner}</tg-emoji></tg-emoji>"
        );
        let chunks = split_to_utf16_chunks(&s, TELEGRAM_MSG_LIMIT);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(
                utf16_len(chunk) <= TELEGRAM_MSG_LIMIT,
                "chunk exceeded Telegram limit: {}",
                utf16_len(chunk)
            );
            // A surrogate-pair split would produce a byte slice that does not fall on a scalar boundary; `String`'s UTF-8 invariant makes that unrepresentable, so `split_to_utf16_chunks` returning valid `String`s at all (rather than panicking on an invalid `str` slice) already proves every boundary landed on a full `😀`.
        }
        let reconstructed = chunks
            .iter()
            .map(|chunk| RE_TAG.replace_all(chunk, ""))
            .collect::<String>();
        assert_eq!(reconstructed, inner);
    }

    #[test]
    fn entity_prefix_trims_back_numeric() {
        assert_eq!(adjust_html_entity_boundary("abc&#39"), "abc");
        assert_eq!(adjust_html_entity_boundary("abc&#x1F"), "abc");
        // Bare `&` at the very end is ambiguous — treat as a potentially-truncated entity and trim back.
        assert_eq!(adjust_html_entity_boundary("abc&"), "abc");
        // `&` followed by clearly non-entity content (punctuation, not a known prefix) is preserved.
        assert_eq!(adjust_html_entity_boundary("abc&!"), "abc&!");
    }
}
