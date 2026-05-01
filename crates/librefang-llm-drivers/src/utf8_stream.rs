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

        let mut out = String::with_capacity(buf.len());
        let mut cursor = 0usize;

        // Drain `buf` left-to-right one segment at a time. A segment is
        // either a maximal valid UTF-8 run, a single replacement for a
        // genuinely invalid sequence, or a trailing partial codepoint
        // that we buffer for the next call. The loop avoids the
        // recursion in the original implementation: each iteration
        // advances `cursor` by ≥1 byte (or breaks), so adversarial
        // input with many invalid bytes stays O(n) without growing the
        // call stack.
        while cursor < buf.len() {
            match std::str::from_utf8(&buf[cursor..]) {
                Ok(s) => {
                    out.push_str(s);
                    cursor = buf.len();
                }
                Err(e) => {
                    let valid_up_to = e.valid_up_to();
                    if valid_up_to > 0 {
                        // Validated by from_utf8 above; expect is preferred
                        // over `unsafe { from_utf8_unchecked }` — the optimizer
                        // elides the bounds check, and we keep the whole
                        // module #![forbid(unsafe_code)]-clean.
                        out.push_str(
                            std::str::from_utf8(&buf[cursor..cursor + valid_up_to])
                                .expect("valid_up_to range was just validated by from_utf8"),
                        );
                    }
                    let segment_start = cursor + valid_up_to;
                    match e.error_len() {
                        // None = trailing partial codepoint — buffer it
                        // for the next decode() call.
                        None => {
                            let tail_len = buf.len() - segment_start;
                            if tail_len <= MAX_PARTIAL_LEN {
                                self.pending = buf[segment_start..].to_vec();
                            } else {
                                // Cannot happen for well-formed UTF-8 (no
                                // codepoint exceeds 4 bytes), but guard
                                // against unbounded buffer growth on hostile
                                // input by flushing as replacement chars.
                                out.push_str(&String::from_utf8_lossy(&buf[segment_start..]));
                            }
                            return out;
                        }
                        // Some(n) = genuinely invalid n-byte sequence;
                        // emit one replacement char and continue past it.
                        Some(n) => {
                            out.push('\u{FFFD}');
                            cursor = segment_start + n;
                        }
                    }
                }
            }
        }

        out
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

    /// Hostile input with many invalid bytes must stay O(n) and not
    /// blow the stack. The pre-loop implementation recursed once per
    /// invalid sequence; this stress-test would have been a depth bomb.
    #[test]
    fn many_invalid_bytes_do_not_overflow_stack() {
        let mut d = Utf8StreamDecoder::new();
        // 100k stray 0xFF bytes interleaved with ASCII.
        let mut buf = Vec::with_capacity(200_000);
        for _ in 0..100_000 {
            buf.push(b'a');
            buf.push(0xFF);
        }
        let out = d.decode(&buf);
        // Every 0xFF maps to one U+FFFD; every 'a' passes through.
        assert_eq!(out.chars().filter(|c| *c == '\u{FFFD}').count(), 100_000);
        assert_eq!(out.chars().filter(|c| *c == 'a').count(), 100_000);
    }

    /// Multiple invalid sequences in one chunk must each get exactly one
    /// replacement char and the tail must keep flowing.
    #[test]
    fn multiple_invalid_sequences_emit_separate_replacements() {
        let mut d = Utf8StreamDecoder::new();
        let out = d.decode(&[b'x', 0xFF, b'y', 0xFE, b'z']);
        assert_eq!(out.chars().filter(|c| *c == '\u{FFFD}').count(), 2);
        assert!(out.starts_with('x'));
        assert!(out.ends_with('z'));
    }
}
