//! Streaming duplicate detection.
//!
//! Detects when the LLM repeats text that was already sent (e.g., repeating
//! tool output verbatim). Uses exact + normalized matching with a sliding window.

/// Minimum length, in characters after `normalize`, for text to be a dedup candidate.
///
/// The detector runs on every streamed `TextDelta`, so the threshold is block-sized: ordinary token deltas (indentation runs, blank-line runs, a repeated `        }\n`, a repeated `| --- | --- |` table row) legitimately recur and must never be candidates.
/// The gate is applied to the normalised form so whitespace-only or whitespace-padded deltas collapse to an empty or near-empty string and are skipped rather than matching each other.
const MIN_DEDUP_LENGTH: usize = 64;

/// Number of recent chunks to keep in the dedup window.
const DEDUP_WINDOW: usize = 50;

/// Streaming duplicate detector.
pub struct StreamDedup {
    /// Recent chunks (exact text).
    recent_chunks: Vec<String>,
    /// Recent chunks (normalized: whitespace-collapsed and trimmed).
    recent_normalized: Vec<String>,
}

impl StreamDedup {
    /// Create a new dedup detector.
    pub fn new() -> Self {
        Self {
            recent_chunks: Vec::with_capacity(DEDUP_WINDOW),
            recent_normalized: Vec::with_capacity(DEDUP_WINDOW),
        }
    }

    /// Check if text is a duplicate of recently sent content.
    ///
    /// Returns `true` if the text matches (exact or normalized) any
    /// recent chunk. Skips texts whose normalised form is shorter than `MIN_DEDUP_LENGTH` characters.
    pub fn is_duplicate(&self, text: &str) -> bool {
        let normalized = normalize(text);
        if normalized.chars().count() < MIN_DEDUP_LENGTH {
            return false;
        }

        // Exact match
        if self.recent_chunks.iter().any(|c| c == text) {
            return true;
        }

        // Normalized match
        self.recent_normalized.iter().any(|c| c == &normalized)
    }

    /// Record text that was successfully sent to the client.
    pub fn record_sent(&mut self, text: &str) {
        let normalized = normalize(text);
        if normalized.chars().count() < MIN_DEDUP_LENGTH {
            return;
        }

        // Evict oldest if at capacity
        if self.recent_chunks.len() >= DEDUP_WINDOW {
            self.recent_chunks.remove(0);
            self.recent_normalized.remove(0);
        }

        self.recent_chunks.push(text.to_string());
        self.recent_normalized.push(normalized);
    }

    /// Clear the dedup window.
    pub fn clear(&mut self) {
        self.recent_chunks.clear();
        self.recent_normalized.clear();
    }
}

impl Default for StreamDedup {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize text for fuzzy matching: collapse whitespace runs to one space and trim.
///
/// Case is preserved: phrases that differ only in case are different content, not a repeated block.
fn normalize(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_was_space = false;

    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block-sized chunk, comfortably above `MIN_DEDUP_LENGTH`.
    const BLOCK: &str =
        "The tool returned the following output, which I will now summarise for you in detail.";

    fn block(i: usize) -> String {
        format!("{BLOCK} This is chunk number {i} of the repeated tool output block.")
    }

    #[test]
    fn test_exact_match_detected() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent(BLOCK);
        assert!(dedup.is_duplicate(BLOCK));
    }

    #[test]
    fn test_normalized_match_detected() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent(BLOCK);
        // Same text but different whitespace
        assert!(dedup.is_duplicate(
            "The tool returned the following  output,\n which I will now summarise  for you in detail."
        ));
    }

    #[test]
    fn test_short_text_skipped() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent("short");
        assert!(!dedup.is_duplicate("short"));
    }

    #[test]
    fn test_window_rollover() {
        let mut dedup = StreamDedup::new();
        // Fill the window
        for i in 0..DEDUP_WINDOW {
            dedup.record_sent(&block(i));
        }
        // Add one more — should evict the oldest
        dedup.record_sent(&block(DEDUP_WINDOW));
        // Oldest should no longer be detected
        assert!(!dedup.is_duplicate(&block(0)));
        // Newest should be detected
        assert!(dedup.is_duplicate(&block(DEDUP_WINDOW)));
    }

    #[test]
    fn test_no_false_positives() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent(&block(1));
        assert!(!dedup.is_duplicate(&block(2)));
    }

    #[test]
    fn test_clear() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent(BLOCK);
        assert!(dedup.is_duplicate(BLOCK));
        dedup.clear();
        assert!(!dedup.is_duplicate(BLOCK));
    }

    #[test]
    fn whitespace_only_deltas_are_never_duplicates() {
        let mut dedup = StreamDedup::new();
        // Newline plus 12 spaces is a single BPE token when streaming indented code.
        let indent = "\n            ";
        assert!(!dedup.is_duplicate(indent));
        dedup.record_sent(indent);
        assert!(!dedup.is_duplicate(indent));
        assert!(!dedup.is_duplicate("\n\n\n\n\n\n\n\n\n\n"));
        assert!(!dedup.is_duplicate("                "));
    }

    #[test]
    fn short_code_tokens_padded_with_whitespace_are_not_duplicates() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent("        }\n");
        assert!(!dedup.is_duplicate("        }\n"));
        dedup.record_sent("\n    return");
        assert!(!dedup.is_duplicate("\n            return"));
        // Repeated table rows and code lines are ordinary token-level repetition.
        dedup.record_sent("| --- | --- |");
        assert!(!dedup.is_duplicate("| --- | --- |"));
    }

    #[test]
    fn case_variant_phrases_are_not_duplicates() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent("Hello World");
        assert!(!dedup.is_duplicate("hello world"));
        // Even at block size, a phrase that differs only in case is different content.
        let upper = BLOCK.to_uppercase();
        dedup.record_sent(BLOCK);
        assert!(!dedup.is_duplicate(&upper));
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("Hello  World"), "Hello World");
        assert_eq!(normalize("  spaced  out  "), "spaced out");
        assert_eq!(normalize("UPPER\n\tcase"), "UPPER case");
        assert_eq!(normalize("   \n\n  "), "");
    }
}
