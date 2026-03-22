//! Text chunking for long documents.
//!
//! Splits text into overlapping chunks suitable for embedding-based memory.
//! Respects paragraph and sentence boundaries to produce coherent chunks.

/// Split `text` into chunks of at most `max_size` characters with `overlap`
/// characters of overlap between consecutive chunks.
///
/// Splitting strategy:
/// 1. Split on paragraph boundaries (`\n\n`) first.
/// 2. If a single paragraph exceeds `max_size`, split it on sentence
///    boundaries (`. ` followed by an uppercase letter, or `.\n`).
/// 3. If a single sentence still exceeds `max_size`, hard-split at the
///    character limit.
///
/// Overlap is applied by prepending the last `overlap` characters of the
/// previous chunk to the beginning of the next chunk.
pub fn chunk_text(text: &str, max_size: usize, overlap: usize) -> Vec<String> {
    if text.is_empty() || max_size == 0 {
        return vec![];
    }

    // If text fits in a single chunk, return as-is.
    if text.len() <= max_size {
        return vec![text.to_string()];
    }

    // Build atomic segments: split paragraphs, then sentences within large paragraphs.
    let segments = build_segments(text, max_size);

    // Greedily pack segments into chunks respecting max_size.
    pack_with_overlap(&segments, max_size, overlap)
}

/// Break text into small segments (paragraphs and sentences) that are each
/// at most `max_size` characters. Segments that are still too large are
/// hard-split.
fn build_segments(text: &str, max_size: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut segments = Vec::new();

    for para in paragraphs {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.len() <= max_size {
            segments.push(trimmed.to_string());
        } else {
            // Split paragraph into sentences.
            for sentence in split_sentences(trimmed) {
                if sentence.len() <= max_size {
                    segments.push(sentence);
                } else {
                    // Hard-split oversized sentences.
                    let mut start = 0;
                    while start < sentence.len() {
                        let end = (start + max_size).min(sentence.len());
                        segments.push(sentence[start..end].to_string());
                        start = end;
                    }
                }
            }
        }
    }

    segments
}

/// Split a paragraph into sentences. Uses `. ` and `.\n` as delimiters,
/// keeping the period with the preceding sentence.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'.'
            && i + 1 < bytes.len()
            && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\n')
        {
            let end = i + 1; // include the period
            let segment = text[start..end].trim();
            if !segment.is_empty() {
                sentences.push(segment.to_string());
            }
            start = end;
            // skip whitespace after the period
            if i + 1 < bytes.len() {
                i += 1; // skip the space/newline
            }
        }
        i += 1;
    }

    // Remaining text
    let remaining = text[start..].trim();
    if !remaining.is_empty() {
        sentences.push(remaining.to_string());
    }

    sentences
}

/// Pack segments into chunks of at most `max_size` characters, applying
/// `overlap` characters from the end of the previous chunk to the start of
/// the next.
fn pack_with_overlap(segments: &[String], max_size: usize, overlap: usize) -> Vec<String> {
    if segments.is_empty() {
        return vec![];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for seg in segments {
        if current.is_empty() {
            current = seg.clone();
        } else {
            // Would adding this segment (with a paragraph separator) exceed the limit?
            let candidate_len = current.len() + 2 + seg.len(); // "\n\n" separator
            if candidate_len <= max_size {
                current.push_str("\n\n");
                current.push_str(seg);
            } else {
                // Flush current chunk
                chunks.push(current.clone());

                // Start new chunk with overlap from previous
                let overlap_text = if overlap > 0 && current.len() > overlap {
                    &current[current.len() - overlap..]
                } else if overlap > 0 {
                    &current
                } else {
                    ""
                };

                if overlap_text.is_empty() {
                    current = seg.clone();
                } else {
                    current = format!("{}\n\n{}", overlap_text, seg);
                    // If the overlap + new segment exceeds max_size, drop the overlap
                    if current.len() > max_size {
                        current = seg.clone();
                    }
                }
            }
        }
    }

    // Don't forget the last chunk
    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_text_passthrough() {
        let text = "Hello, world!";
        let chunks = chunk_text(text, 1500, 200);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn test_empty_text() {
        let chunks = chunk_text("", 1500, 200);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_paragraph_splitting() {
        let text = "First paragraph with some content.\n\nSecond paragraph with different content.\n\nThird paragraph with more content.";
        // Use a small max_size to force splitting
        let chunks = chunk_text(text, 60, 0);
        assert!(chunks.len() >= 2);
        // Each chunk should be within the limit
        for chunk in &chunks {
            assert!(chunk.len() <= 60, "chunk too long: {} chars", chunk.len());
        }
    }

    #[test]
    fn test_overlap_between_chunks() {
        // Create text that will be split into multiple chunks.
        // max_size must be large enough to fit the overlap (20) + separator (\n\n = 2)
        // + the next segment (100), i.e. >= 122, otherwise the overlap is dropped.
        let para1 = "A".repeat(100);
        let para2 = "B".repeat(100);
        let text = format!("{}\n\n{}", para1, para2);
        let chunks = chunk_text(&text, 150, 20);
        assert!(chunks.len() >= 2);
        // The second chunk should start with overlap from the first, followed by \n\n
        if chunks.len() >= 2 {
            let end_of_first = &chunks[0][chunks[0].len() - 20..];
            let expected_prefix = format!("{}\n\n", end_of_first);
            assert!(
                chunks[1].starts_with(&expected_prefix),
                "second chunk should begin with overlap from first chunk followed by separator"
            );
        }
    }

    #[test]
    fn test_sentence_splitting() {
        let long_para = "This is sentence one. This is sentence two. This is sentence three. This is sentence four. This is sentence five.";
        let chunks = chunk_text(long_para, 50, 0);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 50, "chunk too long: {} chars", chunk.len());
        }
    }

    #[test]
    fn test_hard_split_very_long_word() {
        let text = "a".repeat(200);
        let chunks = chunk_text(&text, 50, 0);
        assert_eq!(chunks.len(), 4);
        for chunk in &chunks {
            assert!(chunk.len() <= 50);
        }
    }

    #[test]
    fn test_zero_overlap() {
        let para1 = "A".repeat(100);
        let para2 = "B".repeat(100);
        let text = format!("{}\n\n{}", para1, para2);
        let chunks = chunk_text(&text, 120, 0);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_chunk_count_reasonable() {
        // A ~3000 char document with max_size 1500 should produce 2-3 chunks
        let text = "The quick brown fox jumps. ".repeat(120); // ~3120 chars
        let chunks = chunk_text(&text, 1500, 200);
        assert!(chunks.len() >= 2);
        assert!(chunks.len() <= 5);
    }
}
