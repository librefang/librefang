//! Streaming duplicate detection.
//!
//! Detects when the LLM repeats text that was already sent (e.g., repeating
//! tool output verbatim). Uses exact + normalized matching with a sliding window.

/// Minimum text length to consider for deduplication.
const MIN_DEDUP_LENGTH: usize = 10;

/// Number of recent chunks to keep in the dedup window.
const DEDUP_WINDOW: usize = 50;

/// Streaming duplicate detector.
pub struct StreamDedup {
    /// Recent chunks (exact text).
    recent_chunks: Vec<String>,
    /// Recent chunks (normalized: lowercased, whitespace-collapsed).
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
    /// recent chunk. Skips very short texts.
    pub fn is_duplicate(&self, text: &str) -> bool {
        if text.len() < MIN_DEDUP_LENGTH {
            return false;
        }

        // Exact match
        if self.recent_chunks.iter().any(|c| c == text) {
            return true;
        }

        // Normalized match
        let normalized = normalize(text);
        self.recent_normalized.iter().any(|c| c == &normalized)
    }

    /// Record text that was successfully sent to the client.
    pub fn record_sent(&mut self, text: &str) {
        if text.len() < MIN_DEDUP_LENGTH {
            return;
        }

        // Evict oldest if at capacity
        if self.recent_chunks.len() >= DEDUP_WINDOW {
            self.recent_chunks.remove(0);
            self.recent_normalized.remove(0);
        }

        self.recent_chunks.push(text.to_string());
        self.recent_normalized.push(normalize(text));
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

/// Normalize text for fuzzy matching: lowercase + collapse whitespace.
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
            result.extend(ch.to_lowercase());
            last_was_space = false;
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match_detected() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent("This is a test chunk of text that was sent.");
        assert!(dedup.is_duplicate("This is a test chunk of text that was sent."));
    }

    #[test]
    fn test_normalized_match_detected() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent("This is a test chunk");
        // Same text but different whitespace/case
        assert!(dedup.is_duplicate("this  is  a  test  chunk"));
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
            dedup.record_sent(&format!("chunk number {} is here", i));
        }
        // Add one more — should evict the oldest
        dedup.record_sent("new chunk that is quite long");
        // Oldest should no longer be detected
        assert!(!dedup.is_duplicate("chunk number 0 is here"));
        // Newest should be detected
        assert!(dedup.is_duplicate("new chunk that is quite long"));
    }

    #[test]
    fn test_no_false_positives() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent("The quick brown fox jumps over the lazy dog");
        assert!(!dedup.is_duplicate("A completely different sentence here"));
    }

    #[test]
    fn test_clear() {
        let mut dedup = StreamDedup::new();
        dedup.record_sent("This is test content here");
        assert!(dedup.is_duplicate("This is test content here"));
        dedup.clear();
        assert!(!dedup.is_duplicate("This is test content here"));
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("Hello  World"), "hello world");
        assert_eq!(normalize("  spaced  out  "), "spaced out");
        assert_eq!(normalize("UPPER case"), "upper case");
        assert_eq!(normalize("İSTANBUL"), "i\u{307}stanbul");
    }
}
