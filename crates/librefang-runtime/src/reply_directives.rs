//! Reply directive parsing and streaming accumulation.
//!
//! Supports inline directives in agent output:
//! - `[[reply:id]]` — reply to a specific message ID
//! - `[[@current]]` — reply in the current thread
//! - `[[silent]]` — suppress the response from being sent to the user
//!
//! Directives are stripped from the visible text and collected into a
//! `DirectiveSet`. The `StreamingDirectiveAccumulator` handles partial
//! directive splits at chunk boundaries during streaming.

use serde::{Deserialize, Serialize};

/// Collected directives parsed from agent output.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DirectiveSet {
    /// Reply to a specific message ID.
    pub reply_to: Option<String>,
    /// Reply in the current thread.
    pub current_thread: bool,
    /// Suppress the response.
    pub silent: bool,
}

/// Accumulator that handles directive parsing across streaming chunk boundaries.
///
/// Holds a small partial buffer for cases where a directive tag is split
/// across two chunks (e.g., `[[re` then `ply:123]]`).
pub struct StreamingDirectiveAccumulator {
    /// Partial buffer for incomplete directive tags.
    partial: String,
    /// Accumulated directives (sticky — once set, stays set).
    pub directives: DirectiveSet,
}

/// Maximum size of the partial buffer before we give up and flush it as text.
const MAX_PARTIAL_LEN: usize = 30;

impl StreamingDirectiveAccumulator {
    /// Create a new accumulator.
    pub fn new() -> Self {
        Self {
            partial: String::new(),
            directives: DirectiveSet::default(),
        }
    }

    /// Process a streaming chunk, extracting any directives.
    ///
    /// Returns the cleaned text to display. Handles partial directive tags
    /// that span chunk boundaries. On `is_final`, flushes any remaining
    /// partial buffer as literal text.
    pub fn consume(&mut self, chunk: &str, is_final: bool) -> String {
        // Prepend any partial from previous chunk
        let input = if self.partial.is_empty() {
            chunk.to_string()
        } else {
            let mut combined = std::mem::take(&mut self.partial);
            combined.push_str(chunk);
            combined
        };

        let mut output = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();

        while let Some(&ch) = chars.peek() {
            if ch == '[' {
                // Collect potential directive tag
                let remaining: String = chars.clone().collect();

                // Check if we might be at the start of a directive
                if let Some(after_open) = remaining.strip_prefix("[[") {
                    // Look for closing ]]
                    if let Some(end) = after_open.find("]]") {
                        let tag_content = &after_open[..end];
                        let tag_len = 2 + end + 2; // [[ + content + ]]

                        // Parse the directive
                        self.parse_tag(tag_content);

                        // Advance past the full tag
                        for _ in 0..tag_len {
                            chars.next();
                        }
                        continue;
                    } else if !is_final && remaining.len() < MAX_PARTIAL_LEN {
                        // Might be split across chunks — buffer it
                        self.partial = remaining;
                        return output;
                    }
                    // Else: too long or final — treat as literal
                }
            }

            output.push(chars.next().unwrap());
        }

        // On final chunk, flush any remaining partial as literal text
        if is_final && !self.partial.is_empty() {
            output.push_str(&std::mem::take(&mut self.partial));
        }

        output
    }

    /// Parse a directive tag's inner content.
    fn parse_tag(&mut self, content: &str) {
        let trimmed = content.trim();
        if let Some(id) = trimmed.strip_prefix("reply:") {
            let id = id.trim();
            if !id.is_empty() {
                self.directives.reply_to = Some(id.to_string());
            }
        } else if trimmed == "@current" {
            self.directives.current_thread = true;
        } else if trimmed.eq_ignore_ascii_case("silent") {
            self.directives.silent = true;
        }
        // Unknown directives are silently dropped (stripped from output)
    }
}

impl Default for StreamingDirectiveAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse directives from a complete text string.
///
/// Returns `(cleaned_text, directives)` where cleaned_text has all
/// directive tags removed.
pub fn parse_directives(text: &str) -> (String, DirectiveSet) {
    let mut acc = StreamingDirectiveAccumulator::new();
    let cleaned = acc.consume(text, true);
    (cleaned.trim().to_string(), acc.directives)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_reply_directive() {
        let (text, dirs) = parse_directives("[[reply:msg_123]] Hello!");
        assert_eq!(text, "Hello!");
        assert_eq!(dirs.reply_to.as_deref(), Some("msg_123"));
    }

    #[test]
    fn test_parse_current_thread() {
        let (text, dirs) = parse_directives("[[@current]] Replying in thread");
        assert_eq!(text, "Replying in thread");
        assert!(dirs.current_thread);
    }

    #[test]
    fn test_parse_silent() {
        let (text, dirs) = parse_directives("[[silent]] Internal note");
        assert_eq!(text, "Internal note");
        assert!(dirs.silent);
    }

    #[test]
    fn test_parse_multiple_directives() {
        let (text, dirs) = parse_directives("[[reply:456]] [[@current]] [[silent]] Done");
        assert_eq!(text, "Done");
        assert_eq!(dirs.reply_to.as_deref(), Some("456"));
        assert!(dirs.current_thread);
        assert!(dirs.silent);
    }

    #[test]
    fn test_no_directives() {
        let (text, dirs) = parse_directives("Just regular text");
        assert_eq!(text, "Just regular text");
        assert_eq!(dirs, DirectiveSet::default());
    }

    #[test]
    fn test_directive_in_middle() {
        let (text, dirs) = parse_directives("Hello [[silent]] world");
        assert_eq!(text, "Hello  world");
        assert!(dirs.silent);
    }

    #[test]
    fn test_streaming_split_directive() {
        let mut acc = StreamingDirectiveAccumulator::new();

        // First chunk ends mid-directive
        let out1 = acc.consume("Hello [[re", false);
        assert_eq!(out1, "Hello ");

        // Second chunk completes it
        let out2 = acc.consume("ply:xyz]] world", true);
        assert_eq!(out2, " world");
        assert_eq!(acc.directives.reply_to.as_deref(), Some("xyz"));
    }

    #[test]
    fn test_streaming_no_split() {
        let mut acc = StreamingDirectiveAccumulator::new();
        let out1 = acc.consume("[[silent]] chunk1", false);
        assert_eq!(out1, " chunk1");
        assert!(acc.directives.silent);

        let out2 = acc.consume(" chunk2", true);
        assert_eq!(out2, " chunk2");
    }

    #[test]
    fn test_streaming_sticky_directives() {
        let mut acc = StreamingDirectiveAccumulator::new();
        let _ = acc.consume("[[silent]]", false);
        assert!(acc.directives.silent);

        // Directive persists across chunks
        let _ = acc.consume("more text", true);
        assert!(acc.directives.silent);
    }

    #[test]
    fn test_partial_buffer_flush_on_final() {
        let mut acc = StreamingDirectiveAccumulator::new();
        // Looks like it could be a directive but never completes
        let out1 = acc.consume("text [[not_closed", false);
        assert_eq!(out1, "text ");

        // On final, partial is flushed as literal
        let out2 = acc.consume("", true);
        assert_eq!(out2, "[[not_closed");
    }

    #[test]
    fn test_backward_compat_no_reply() {
        // NO_REPLY token still works independently of directives
        let (text, dirs) = parse_directives("NO_REPLY");
        assert_eq!(text, "NO_REPLY");
        assert_eq!(dirs, DirectiveSet::default());
    }

    #[test]
    fn test_unknown_directive_stripped() {
        let (text, dirs) = parse_directives("[[unknown_thing]] visible");
        // Unknown directives are stripped from output but don't set any field
        assert_eq!(text, "visible");
        assert_eq!(dirs, DirectiveSet::default());
    }
}
