//! UTF-16 chunking for Telegram's 4096-code-unit message limit.
//!
//! Mirrors the Python adapter's `_utf16_len`, `_truncate_to_utf16_limit`, and `_split_to_utf16_chunks`.
//! Telegram counts code units, not bytes or Unicode scalars; chars above U+FFFF count as 2.
//! The chunker prefers newlines, falls back to char boundaries, and shrinks chunks ending mid-HTML-entity so the supervisor doesn't ship `&lt` and break formatting.

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

/// If `chunk` ends in a partial HTML entity (`&` opened but not closed, or `&#` digits then truncated), shrink it back to before the `&` so the receiving HTML parser never sees a broken entity.
fn adjust_html_entity_boundary(chunk: &str) -> &str {
    // Find the last `&` in the chunk; if there's no matching `;` after it, the entity is open.
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
        // Stop scanning after a few characters — Telegram entities never exceed ~10 bytes.
        if bytes.len() - i > 12 {
            return chunk;
        }
    }
    match amp {
        Some(i) => &chunk[..i],
        None => chunk,
    }
}

/// Split `s` into chunks no longer than `limit` UTF-16 code units each.
/// Prefers a trailing newline as the split point; falls back to truncating at the highest char boundary that fits.
/// A single character wider than `limit` (rare; Telegram limit is 4096) is emitted alone so the loop makes progress.
pub fn split_to_utf16_chunks(s: &str, limit: usize) -> Vec<String> {
    assert!(limit > 0, "limit must be > 0");
    if utf16_len(s) <= limit {
        return vec![s.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut remaining = s;
    while !remaining.is_empty() {
        if utf16_len(remaining) <= limit {
            out.push(remaining.to_string());
            break;
        }
        // Find the largest prefix <= limit.
        let prefix = truncate_to_utf16_limit(remaining, limit);
        // Prefer to split at the last newline inside that prefix; if not found, accept the prefix as-is.
        let split_idx = prefix.rfind('\n').map(|i| i + 1).unwrap_or(prefix.len());
        let mut chunk = &prefix[..split_idx];
        if chunk.is_empty() {
            // No newline in the prefix — single oversized line. Use the whole prefix.
            chunk = prefix;
        }
        // Avoid mid-entity truncation.
        let chunk = adjust_html_entity_boundary(chunk);
        if chunk.is_empty() {
            // The prefix consisted entirely of an open entity; force at least one char of progress so the loop terminates.
            let one_char_end = remaining
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(remaining.len());
            out.push(remaining[..one_char_end].to_string());
            remaining = &remaining[one_char_end..];
            continue;
        }
        out.push(chunk.to_string());
        remaining = &remaining[chunk.len()..];
    }
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
}
