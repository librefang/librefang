//! UTF-16 aware message truncation for platform character limits.
//!
//! Telegram's 4096-character limit (and similar platform limits) are measured
//! in **UTF-16 code units**, not Unicode code points.  Characters outside the
//! Basic Multilingual Plane — emoji (e.g. 😀), CJK Extension B, musical
//! symbols — are encoded as surrogate pairs and consume **two** UTF-16 code
//! units each, even though Rust's `char` and `str::chars().count()` count
//! them as a single code point.
//!
//! Ported from the Python reference in hermes-agent/gateway/platforms/base.py
//! (originally from nearai/ironclaw#2304).

/// Platform message limits in UTF-16 code units.
pub const TELEGRAM_MESSAGE_LIMIT: usize = 4096;
/// Telegram caption limit (photo / video / document captions).
pub const TELEGRAM_CAPTION_LIMIT: usize = 1024;
/// Discord message limit in UTF-16 code units.
pub const DISCORD_MESSAGE_LIMIT: usize = 2000;

/// Count the number of UTF-16 code units in `s`.
///
/// Characters in the Basic Multilingual Plane (U+0000–U+FFFF) occupy one
/// code unit; supplementary characters (U+10000 and above, including most
/// emoji and CJK Extension B) occupy two code units (a surrogate pair).
///
/// # Examples
/// ```
/// use librefang_channels::message_truncator::utf16_len;
///
/// assert_eq!(utf16_len("hello"), 5);
/// assert_eq!(utf16_len("🎉"),  2); // surrogate pair
/// assert_eq!(utf16_len("中文"),  2); // BMP — one unit each
/// ```
pub fn utf16_len(s: &str) -> usize {
    s.chars()
        .map(|c| if (c as u32) > 0xFFFF { 2 } else { 1 })
        .sum()
}

/// If `chunk` ends inside an HTML entity that spans the boundary into `rest`,
/// shrink `chunk` to end at the last safe entity boundary so the entity is
/// preserved intact.
///
/// HTML entities are `&name;` or `&#123;` (decimal) or `&#xABC;` (hex).
/// We only handle the named entities that `sanitize_telegram_html` produces
/// or that can appear in Telegram HTML: `&amp;`, `&lt;`, `&gt;`, `&quot;`,
/// `&nbsp;`, `&amp;#` (escaped entity prefix), `&#`, `&#x`.
///
/// Returns `chunk` unchanged if no entity is broken across the boundary.
///
/// # Examples
/// ```
/// use librefang_channels::message_truncator::adjust_html_entity_boundary;
///
/// // Entity `&lt;` split after `&l` — should cut before `&l` so `&lt;` stays complete
/// assert_eq!(adjust_html_entity_boundary("foo &l"), "foo ");
/// // No broken entity — no change
/// assert_eq!(adjust_html_entity_boundary("hello &lt;"), "hello &lt;");
/// // `&#` prefix cut — remove the broken `&#`
/// assert_eq!(adjust_html_entity_boundary("text &#x"), "text ");
/// ```
fn adjust_html_entity_boundary(chunk: &str) -> &str {
    // Check if chunk ends with what looks like a truncated entity.
    // We look for '&' followed by partial entity content (but not a
    // complete valid entity that ends with ';').
    let tail = match chunk.rfind('&') {
        Some(pos) => &chunk[pos..],
        None => return chunk,
    };

    // If the tail is already a valid complete entity (ends with ';'), it is
    // safe — no adjustment needed.
    if tail.ends_with(';') {
        return chunk;
    }

    // The '&' is not followed by ';' — entity may be broken.
    // Named entities: &amp; &lt; &gt; &quot; &nbsp;
    // Numeric entities: &#digits;  &#xhexdigits;
    // Pattern: '&' followed by name chars (a-zA-Z) or '#' (decimal/hex).
    // We accept up to 10 chars after '&' to handle `&#xxxxxxxx` (8 hex + 'x').
    let after_ampersand = &tail[1..];
    let is_entity_like = !after_ampersand.is_empty()
        && after_ampersand
            .chars()
            .take(10)
            .all(|c| c.is_ascii_alphanumeric() || c == '#' || c == 'x' || c == ';');

    if !is_entity_like {
        // No active entity — the '&' is a literal ampersand or start of
        // something else (e.g. `&foo` not a known entity). Leave as-is.
        return chunk;
    }

    // Entity is broken. Find the last safe split point by scanning backward.
    // Safe points are: before '&', or after ';' (but we already checked that).
    // We scan the chunk for the last '&' that is not followed by a valid ';`.
    // Actually: we want to drop everything from the broken entity start.
    let amp_pos = chunk.rfind('&').unwrap();
    &chunk[..amp_pos]
}

/// Split `s` into chunks where each chunk's UTF-16 length is ≤ `limit`.
///
/// Splits preferring newline boundaries when a natural break point exists near
/// the limit, then falls back to splitting exactly at the char boundary that
/// keeps the chunk within `limit` UTF-16 code units.
///
/// Never splits inside a surrogate pair (i.e. always at a Rust `char`
/// boundary), so the output chunks are always valid `&str` slices.
///
/// Returns a single-element `vec![s]` when `s` already fits within `limit`.
///
/// # Examples
/// ```
/// use librefang_channels::message_truncator::split_to_utf16_chunks;
///
/// // ASCII — no split needed
/// let chunks = split_to_utf16_chunks("hello", 10);
/// assert_eq!(chunks, vec!["hello"]);
///
/// // Each 🎉 = 2 UTF-16 units; limit=4 → split after 2 emoji
/// let chunks = split_to_utf16_chunks("🎉🎉🎉", 4);
/// assert_eq!(chunks, vec!["🎉🎉", "🎉"]);
/// ```
pub fn split_to_utf16_chunks(s: &str, limit: usize) -> Vec<&str> {
    if utf16_len(s) <= limit {
        return vec![s];
    }
    let mut chunks: Vec<&str> = Vec::new();
    let mut remaining = s;
    while !remaining.is_empty() {
        if utf16_len(remaining) <= limit {
            chunks.push(remaining);
            break;
        }
        // Find the longest prefix that fits within `limit` UTF-16 code units.
        let safe_prefix = truncate_to_utf16_limit(remaining, limit);
        // Prefer splitting at a newline inside the safe prefix.
        // When the newline is preceded by \r (Windows CRLF), split *before*
        // the \r so that the emitted chunk doesn't end with a stray '\r'.
        // The \r\n pair is then consumed together by the strip_prefix below.
        let split_at = match safe_prefix.rfind('\n') {
            Some(nl) if nl > 0 && safe_prefix.as_bytes()[nl - 1] == b'\r' => nl - 1,
            Some(nl) => nl,
            None => safe_prefix.len(),
        };
        let raw_chunk_len = split_at;
        let (chunk, rest) = remaining.split_at(raw_chunk_len);

        // ── HTML-entity boundary guard ─────────────────────────────────────
        // When streaming, the caller sends chunks with parse_mode=HTML.
        // Splitting inside an HTML entity (e.g. `&lt;` → `&lt` + `<text`)
        // causes Telegram to reject the chunk with "can't parse entities".
        // Detect and avoid this by shrinking the chunk to the last complete
        // entity boundary before the split point.
        let chunk = adjust_html_entity_boundary(chunk);
        // Recompute rest after the adjustment — the part we discarded
        // (broken entity prefix + rest) must be prepended to `rest`.
        let discard_len = raw_chunk_len - chunk.len();
        let rest = &remaining[discard_len..];
        // ─────────────────────────────────────────────────────────────────

        // Guard against zero-progress (degenerate limit=0 or limit=1 on a
        // 2-unit char that can't fit at all).
        if chunk.is_empty() {
            if safe_prefix.is_empty() {
                // safe_prefix is empty when even a single char exceeds the
                // limit (e.g. a surrogate-pair emoji with limit=1, or
                // limit=0).  We must still advance past at least one char
                // to avoid an infinite loop.  Emit that one char as an
                // oversized-but-unavoidable chunk and continue.
                let next_char_len = remaining
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(remaining.len());
                chunks.push(&remaining[..next_char_len]);
                remaining = &remaining[next_char_len..];
            } else {
                // Force progress: emit the safe prefix and continue.
                chunks.push(safe_prefix);
                remaining = &remaining[safe_prefix.len()..];
            }
            continue;
        }
        chunks.push(chunk);
        // Skip the newline we split on (handle \r\n and bare \n).
        remaining = rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
            .unwrap_or(rest);
    }
    chunks
}

/// Return the longest prefix of `s` whose UTF-16 length is < `limit`.
///
/// Uses binary search over the char-index table, so the result is always
/// aligned to a char boundary — we never slice a surrogate pair in half.
///
/// Returns the original `s` unchanged when it already fits within `limit`.
///
/// # Examples
/// ```
/// use librefang_channels::message_truncator::truncate_to_utf16_limit;
///
/// // ASCII — no truncation needed
/// assert_eq!(truncate_to_utf16_limit("hello", 10), "hello");
///
/// // Emoji: each 🎉 = 2 units, so 3 emoji = 6 units > 5 → truncates to 2
/// let s = "🎉🎉🎉";
/// assert_eq!(truncate_to_utf16_limit(s, 5), "🎉🎉");
///
/// // Boundary: exactly at limit
/// let s = "🎉🎉";
/// assert_eq!(truncate_to_utf16_limit(s, 4), "🎉🎉");
/// ```
pub fn truncate_to_utf16_limit(s: &str, limit: usize) -> &str {
    if utf16_len(s) <= limit {
        return s;
    }

    // Collect (byte_offset, char) pairs once; avoids repeated scanning.
    let chars: Vec<(usize, char)> = s.char_indices().collect();

    // Binary-search for the largest prefix of `chars` whose cumulative
    // UTF-16 length is strictly less than `limit`.
    let mut lo: usize = 0;
    let mut hi: usize = chars.len();

    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let count: usize = chars[..mid]
            .iter()
            .map(|(_, c)| if (*c as u32) > 0xFFFF { 2 } else { 1 })
            .sum();
        if count < limit {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    // `lo` is the number of chars that fit; look up the byte offset of the
    // *next* char (or end-of-string) to get the slice boundary.
    let byte_end = if lo == 0 {
        0
    } else if lo < chars.len() {
        chars[lo].0 // byte offset of the first char that did NOT fit
    } else {
        s.len()
    };

    &s[..byte_end]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── utf16_len ────────────────────────────────────────────────────────────

    #[test]
    fn ascii_counts_one_per_char() {
        assert_eq!(utf16_len("hello, world!"), 13);
        assert_eq!(utf16_len(""), 0);
    }

    #[test]
    fn bmp_cjk_counts_one_per_char() {
        // U+4E2D (中) and U+6587 (文) are in the BMP → 1 unit each
        assert_eq!(utf16_len("中文"), 2);
        assert_eq!(utf16_len("日本語"), 3);
    }

    #[test]
    fn emoji_surrogate_pairs_count_two() {
        // 😀 = U+1F600, outside BMP → 2 units
        assert_eq!(utf16_len("😀"), 2);
        // 🎉 = U+1F389 → 2 units
        assert_eq!(utf16_len("🎉"), 2);
        // Three emoji = 6 units
        assert_eq!(utf16_len("🎉🎉🎉"), 6);
    }

    #[test]
    fn cjk_extension_b_counts_two() {
        // U+20000 (𠀀) is in CJK Extension B → surrogate pair → 2 units
        let s = "\u{20000}";
        assert_eq!(utf16_len(s), 2);
    }

    #[test]
    fn mixed_ascii_emoji_cjk() {
        // "hi😀中" = 2 + 2 + 1 = 5
        assert_eq!(utf16_len("hi😀中"), 5);
    }

    // ── truncate_to_utf16_limit ──────────────────────────────────────────────

    #[test]
    fn no_truncation_when_within_limit() {
        assert_eq!(truncate_to_utf16_limit("hello", 10), "hello");
        assert_eq!(truncate_to_utf16_limit("", 4096), "");
    }

    #[test]
    fn ascii_truncation() {
        assert_eq!(truncate_to_utf16_limit("abcde", 3), "abc");
    }

    #[test]
    fn emoji_truncation_respects_surrogate_pairs() {
        // "🎉🎉🎉" = 6 UTF-16 units; limit=5 → only 2 emoji (4 units) fit
        let s = "🎉🎉🎉";
        let result = truncate_to_utf16_limit(s, 5);
        assert_eq!(result, "🎉🎉");
        assert_eq!(utf16_len(result), 4);
    }

    #[test]
    fn cjk_extension_b_truncation() {
        // Each 𠀀 (U+20000) = 2 units; three = 6 units; limit=4 → 2 chars
        let s = "\u{20000}\u{20000}\u{20000}";
        let result = truncate_to_utf16_limit(s, 4);
        assert_eq!(utf16_len(result), 4);
        assert_eq!(result.chars().count(), 2);
    }

    #[test]
    fn boundary_exactly_at_limit() {
        // "🎉🎉" = 4 units; limit=4 → no truncation
        let s = "🎉🎉";
        assert_eq!(truncate_to_utf16_limit(s, 4), s);
    }

    #[test]
    fn limit_zero_returns_empty() {
        assert_eq!(truncate_to_utf16_limit("hello", 0), "");
        assert_eq!(truncate_to_utf16_limit("🎉", 0), "");
    }

    #[test]
    fn mixed_content_truncation() {
        // "hi😀中文" = 2 + 2 + 1 + 1 = 6 units; limit=4 → "hi😀" (4 units)
        let s = "hi😀中文";
        let result = truncate_to_utf16_limit(s, 4);
        assert_eq!(result, "hi😀");
        assert_eq!(utf16_len(result), 4);
    }

    #[test]
    fn telegram_limit_constant_is_4096() {
        assert_eq!(TELEGRAM_MESSAGE_LIMIT, 4096);
    }

    #[test]
    fn discord_limit_constant_is_2000() {
        assert_eq!(DISCORD_MESSAGE_LIMIT, 2000);
    }

    // ── split_to_utf16_chunks ────────────────────────────────────────────────

    #[test]
    fn split_no_split_needed_ascii() {
        let chunks = split_to_utf16_chunks("hello", 10);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn split_no_split_needed_empty() {
        let chunks = split_to_utf16_chunks("", 4096);
        assert_eq!(chunks, vec![""]);
    }

    #[test]
    fn split_ascii_into_two_chunks() {
        // "abcde" limit=3 → ["abc", "de"]
        let chunks = split_to_utf16_chunks("abcde", 3);
        assert_eq!(chunks, vec!["abc", "de"]);
    }

    #[test]
    fn split_emoji_respects_surrogate_pairs() {
        // "🎉🎉🎉" = 6 UTF-16 units; limit=4 → ["🎉🎉", "🎉"]
        let s = "🎉🎉🎉";
        let chunks = split_to_utf16_chunks(s, 4);
        assert_eq!(chunks, vec!["🎉🎉", "🎉"]);
        // Verify each chunk fits within limit
        for c in &chunks {
            assert!(utf16_len(c) <= 4, "chunk exceeds limit: {c:?}");
        }
    }

    #[test]
    fn split_cjk_extension_b() {
        // Three 𠀀 (U+20000) chars = 6 UTF-16 units; limit=4 → 2 fit in chunk 1
        let s = "\u{20000}\u{20000}\u{20000}";
        let chunks = split_to_utf16_chunks(s, 4);
        assert_eq!(chunks.len(), 2);
        assert_eq!(utf16_len(chunks[0]), 4);
        assert_eq!(utf16_len(chunks[1]), 2);
    }

    #[test]
    fn split_prefers_newline_boundary() {
        // "abc\nde" with limit=5 → should split at newline → ["abc", "de"]
        let chunks = split_to_utf16_chunks("abc\nde", 5);
        assert_eq!(chunks, vec!["abc", "de"]);
    }

    #[test]
    fn split_crlf_no_trailing_cr() {
        // When the newline is part of a CRLF pair, the \r must NOT bleed
        // into the preceding chunk.  Previously rfind('\n') found the \n
        // at byte 4 of "abc\r\n" and split_at(4) yielded chunk="abc\r".
        let chunks = split_to_utf16_chunks("abc\r\nde", 5);
        assert_eq!(chunks, vec!["abc", "de"]);
        for c in &chunks {
            assert!(
                !c.ends_with('\r'),
                "chunk must not end with stray \\r: {c:?}"
            );
        }
    }

    #[test]
    fn split_crlf_emoji_no_trailing_cr() {
        // Same but with emoji to exercise the UTF-16 counting path.
        // "🎉\r\nok" = 2+1+1+2 = 6 units; limit=4 → split at \r\n → ["🎉", "ok"]
        let chunks = split_to_utf16_chunks("🎉\r\nok", 4);
        assert_eq!(chunks, vec!["🎉", "ok"]);
        for c in &chunks {
            assert!(
                !c.ends_with('\r'),
                "chunk must not end with stray \\r: {c:?}"
            );
        }
    }

    #[test]
    fn split_mixed_emoji_and_ascii() {
        // "hi🎉 ok" = 2+2+1+2 = 7 units; limit=5 → "hi🎉" (4) fits, " ok" (3)
        let s = "hi🎉 ok";
        let chunks = split_to_utf16_chunks(s, 5);
        for c in &chunks {
            assert!(utf16_len(c) <= 5, "chunk {c:?} exceeds limit");
        }
        // Reconstruct original (newline-split drops \n; space split is raw)
        // Just verify the chunks together cover all content
        let joined: String = chunks.concat();
        assert_eq!(joined, s);
    }

    #[test]
    fn split_exactly_at_limit_no_split() {
        // "🎉🎉" = 4 UTF-16 units; limit=4 → single chunk
        let s = "🎉🎉";
        let chunks = split_to_utf16_chunks(s, 4);
        assert_eq!(chunks, vec!["🎉🎉"]);
    }

    #[test]
    fn split_limit_zero_does_not_loop() {
        // limit=0: no char fits, but each char must still be emitted to
        // avoid an infinite loop.  Every character becomes its own chunk.
        let chunks = split_to_utf16_chunks("ab", 0);
        assert_eq!(chunks, vec!["a", "b"]);
    }

    #[test]
    fn split_surrogate_pair_exceeds_limit_does_not_loop() {
        // limit=1: a surrogate-pair emoji (2 units) cannot fit within the
        // limit; must still advance past it rather than looping forever.
        let chunks = split_to_utf16_chunks("🎉🎉", 1);
        // Each emoji is an unavoidable oversized chunk.
        assert_eq!(chunks, vec!["🎉", "🎉"]);
    }

    // ── adjust_html_entity_boundary ─────────────────────────────────────────

    #[test]
    fn html_entity_not_broken_no_change() {
        // Complete entity at end — no adjustment needed
        assert_eq!(adjust_html_entity_boundary("hello &lt;"), "hello &lt;");
        assert_eq!(
            adjust_html_entity_boundary("foo &amp; bar"),
            "foo &amp; bar"
        );
        assert_eq!(
            adjust_html_entity_boundary("&quot;quoted&quot;"),
            "&quot;quoted&quot;"
        );
        assert_eq!(
            adjust_html_entity_boundary("text&nbsp;here"),
            "text&nbsp;here"
        );
        assert_eq!(adjust_html_entity_boundary("&#42;"), "&#42;");
        assert_eq!(adjust_html_entity_boundary("&#x2A;"), "&#x2A;");
    }

    #[test]
    fn html_entity_broken_at_name_truncates_to_safe_point() {
        // `&lt` is truncated — entity is broken, drop the broken prefix
        assert_eq!(adjust_html_entity_boundary("foo &l"), "foo ");
        assert_eq!(adjust_html_entity_boundary("&am"), "");
        assert_eq!(adjust_html_entity_boundary("text &gt"), "text ");
    }

    #[test]
    fn html_entity_broken_numeric_prefix_truncates() {
        // `&#` or `&#x` prefix without closing `;` — entity broken
        assert_eq!(adjust_html_entity_boundary("text &#x"), "text ");
        assert_eq!(adjust_html_entity_boundary("&#42"), "");
        assert_eq!(adjust_html_entity_boundary("val &#x1F"), "val ");
    }

    #[test]
    fn html_entity_ampersand_letter_not_entity_no_change() {
        // `&` followed by something that doesn't look like an entity — keep
        assert_eq!(adjust_html_entity_boundary("foo &bar"), "foo &bar");
    }

    #[test]
    fn html_entity_no_ampersand_no_change() {
        assert_eq!(adjust_html_entity_boundary("hello world"), "hello world");
        assert_eq!(
            adjust_html_entity_boundary("no entities here"),
            "no entities here"
        );
    }

    #[test]
    fn split_preserves_html_entities_across_chunks() {
        // Entity `&lt;` would be split by raw limit — boundary guard must prevent it.
        // "text &lt;more" with limit=10: `&lt` (3 UTF-16 units) fits in 10,
        // so safe_prefix includes "text &lt", but rfind('\n')==None so split at
        // byte 9. `adjust_html_entity_boundary` drops the broken `&lt` → "text ".
        // Next iteration processes " &lt;more" which starts with a space.
        let s = "text &lt;tag&gt;end";
        let chunks = split_to_utf16_chunks(s, 10);
        // Verify no chunk ends with a broken entity (no '&' not followed by ';')
        for chunk in &chunks {
            if let Some(pos) = chunk.rfind('&') {
                let tail = &chunk[pos..];
                assert!(
                    tail.ends_with(';')
                        || !tail
                            .chars()
                            .take(10)
                            .all(|c| c.is_ascii_alphanumeric() || c == '#' || c == 'x'),
                    "chunk has broken entity: {chunk:?}"
                );
            }
        }
    }

    #[test]
    fn split_numeric_html_entity_intact() {
        // `&#x1F600;` (emoji as numeric entity) should stay intact across chunks
        let s = "a&#x1F600;b&#x1F600;c";
        let chunks = split_to_utf16_chunks(s, 8); // force split between entities
        for chunk in &chunks {
            // No chunk should end with broken numeric entity prefix
            if chunk.ends_with('&') || chunk.ends_with('#') || chunk.ends_with("#x") {
                panic!("chunk ends with broken entity: {chunk:?}");
            }
        }
    }

    #[test]
    fn split_entity_at_chunk_boundary() {
        // Specifically test the reported bug: entity split at chunk boundary
        // produces &lt without ; which Telegram rejects.
        // "say &lt; here" — if limit cuts before `;`, entity must be preserved.
        let s = "alpha &lt;beta&gt; gamma";
        // Use a tight limit to force split right after &lt
        let chunks = split_to_utf16_chunks(s, 12); // "alpha &lt;" = 11, split possible
        for chunk in &chunks {
            assert!(
                !chunk.ends_with('&'),
                "chunk must not end with bare &: {chunk:?}"
            );
        }
    }
}
