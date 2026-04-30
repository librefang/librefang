//! Incremental UTF-8 decoder for byte-stream SSE chunks.
//!
//! `String::from_utf8_lossy(&chunk)` is unsafe to call per-chunk on a
//! streaming HTTP body: when a multi-byte codepoint straddles a chunk
//! boundary the leading bytes appear in chunk N as an incomplete
//! sequence, get rewritten to U+FFFD, and the trailing bytes in chunk
//! N+1 also become U+FFFD. Two replacement characters in place of one
//! valid codepoint — for CJK text this corrupts every character whose
//! 3-byte UTF-8 encoding happens to land on a TCP segment boundary
//! (#3448).
//!
//! `Utf8StreamDecoder` buffers the trailing partial bytes between
//! `decode()` calls and only emits validated UTF-8 to the caller. On
//! genuinely invalid input it still falls back to lossy replacement so
//! the stream cannot stall indefinitely.

/// Maximum bytes that can sit at the end of one chunk waiting for a
/// continuation. UTF-8 codepoints are at most 4 bytes — anything longer
/// is invalid and gets flushed as replacement characters.
const MAX_PARTIAL_LEN: usize = 4;

/// Incrementally decodes a stream of byte chunks into UTF-8 text without
/// corrupting codepoints that span chunk boundaries.
#[derive(Default, Debug)]
pub struct Utf8StreamDecoder {
    /// Tail bytes from the previous chunk that did not form a complete
    /// UTF-8 sequence on their own. Always ≤ `MAX_PARTIAL_LEN` bytes.
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    /// Create a fresh decoder with no buffered bytes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode the next byte chunk, returning the longest valid UTF-8
    /// prefix that ends on a codepoint boundary. Any trailing bytes that
    /// form an incomplete sequence are buffered and prepended to the
    /// next call.
    pub fn decode(&mut self, chunk: &[u8]) -> String {
        // Concatenate any leftover bytes from the previous call.
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(chunk);

        match std::str::from_utf8(&buf) {
            Ok(s) => s.to_string(),
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                // Safe: validated by from_utf8 above.
                let head =
                    unsafe { std::str::from_utf8_unchecked(&buf[..valid_up_to]) }.to_string();

                match e.error_len() {
                    // None = trailing partial codepoint — buffer it.
                    None => {
                        let tail_len = buf.len() - valid_up_to;
                        if tail_len <= MAX_PARTIAL_LEN {
                            self.pending = buf[valid_up_to..].to_vec();
                            head
                        } else {
                            // Should never happen for well-formed UTF-8;
                            // flush as replacement to avoid unbounded growth.
                            head + &String::from_utf8_lossy(&buf[valid_up_to..])
                        }
                    }
                    // Some(n) = genuinely invalid n-byte sequence in the
                    // middle. Replace it and recurse over the remainder.
                    Some(n) => {
                        let bad_end = valid_up_to + n;
                        let mut out = head;
                        out.push('\u{FFFD}');
                        // Recurse to handle the rest (which may itself end
                        // in another partial codepoint).
                        out.push_str(&self.decode(&buf[bad_end..]));
                        out
                    }
                }
            }
        }
    }

    /// Flush any buffered partial bytes as replacement characters.
    /// Call at end-of-stream to avoid silently swallowing a final
    /// truncated codepoint.
    pub fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            String::new()
        } else {
            let s = String::from_utf8_lossy(&self.pending).into_owned();
            self.pending.clear();
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3-byte CJK codepoint split across two chunks must reassemble
    /// correctly — the original bug from #3448.
    #[test]
    fn cjk_split_across_two_chunks_round_trips() {
        // "你好" = E4 BD A0 E5 A5 BD (6 bytes total).
        let bytes = "你好".as_bytes();
        // Split mid-codepoint: 4 bytes then 2 bytes — the first chunk
        // ends with E5 alone, which is not valid UTF-8 on its own.
        let (a, b) = bytes.split_at(4);

        let mut d = Utf8StreamDecoder::new();
        let part1 = d.decode(a);
        let part2 = d.decode(b);
        assert_eq!(part1, "你");
        assert_eq!(part2, "好");
        assert_eq!(part1 + &part2, "你好");
        assert!(d.finish().is_empty());
    }

    /// Codepoint split across THREE chunks (1 + 1 + 1 byte) must also
    /// reassemble — provider buffers can be very small.
    #[test]
    fn three_byte_codepoint_split_three_ways() {
        let bytes = "好".as_bytes();
        assert_eq!(bytes.len(), 3);

        let mut d = Utf8StreamDecoder::new();
        let p1 = d.decode(&bytes[0..1]);
        let p2 = d.decode(&bytes[1..2]);
        let p3 = d.decode(&bytes[2..3]);
        assert_eq!(p1, "");
        assert_eq!(p2, "");
        assert_eq!(p3, "好");
    }

    /// 4-byte emoji split across chunks must also work.
    #[test]
    fn four_byte_emoji_split_in_half() {
        // "🦀" = F0 9F A6 80 (4 bytes).
        let bytes = "🦀".as_bytes();
        let (a, b) = bytes.split_at(2);

        let mut d = Utf8StreamDecoder::new();
        assert_eq!(d.decode(a), "");
        assert_eq!(d.decode(b), "🦀");
    }

    /// ASCII passes through unchanged with no buffering.
    #[test]
    fn ascii_passes_through_directly() {
        let mut d = Utf8StreamDecoder::new();
        assert_eq!(d.decode(b"hello"), "hello");
        assert_eq!(d.decode(b" world"), " world");
        assert!(d.finish().is_empty());
    }

    /// Mixed ASCII + CJK across boundaries.
    #[test]
    fn mixed_ascii_and_cjk_split() {
        let s = "ab你好cd";
        let bytes = s.as_bytes();
        // 'a' 'b' E4 BD | A0 E5 A5 BD 'c' 'd'
        let (a, b) = bytes.split_at(4);

        let mut d = Utf8StreamDecoder::new();
        let p1 = d.decode(a);
        let p2 = d.decode(b);
        assert_eq!(p1 + &p2, s);
    }

    /// Genuinely invalid bytes get replaced rather than swallowed.
    #[test]
    fn invalid_byte_becomes_replacement_character() {
        let mut d = Utf8StreamDecoder::new();
        let out = d.decode(&[0x68, 0xFF, 0x69]); // 'h' 0xFF 'i'
        assert!(out.contains('\u{FFFD}'));
        assert!(out.contains('h'));
        assert!(out.contains('i'));
    }

    /// `finish()` flushes a truncated codepoint at end-of-stream so
    /// callers see something rather than silently losing data.
    #[test]
    fn finish_flushes_dangling_partial_codepoint() {
        let mut d = Utf8StreamDecoder::new();
        // Just the first 2 bytes of a 3-byte codepoint.
        let _ = d.decode(&"好".as_bytes()[..2]);
        let tail = d.finish();
        assert!(!tail.is_empty());
        assert!(tail.contains('\u{FFFD}'));
    }
}
