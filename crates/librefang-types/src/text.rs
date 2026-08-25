//! Shared text-normalization constants used across security boundaries.

/// The canonical set of invisible / format Unicode code points stripped at prompt security boundaries.
///
/// This is the single source of truth for sanitization.
/// Every place that strips these code points before scanning skill content or interpolating text into an LLM prompt references this const instead of maintaining its own copy, so the set cannot silently drift between crates.
/// `librefang-skills::verify` keeps a parallel labeled table and pins it to this set in a unit test.
///
/// Sanitization deliberately includes format characters that are legitimate inside ordinary emoji and other display sequences.
/// Standalone injection signaling has different false-positive costs and uses [`INJECTION_SIGNAL_CHARS`] instead.
pub const INVISIBLE_FORMAT_CHARS: &[char] = &[
    // Zero-width & joiner code points
    '\u{00AD}', // soft hyphen
    '\u{034F}', // combining grapheme joiner
    '\u{115F}', // hangul choseong filler
    '\u{1160}', // hangul jungseong filler
    '\u{17B4}', // khmer vowel inherent aq
    '\u{17B5}', // khmer vowel inherent aa
    '\u{180E}', // mongolian vowel separator
    '\u{200B}', // zero-width space
    '\u{200C}', // zero-width non-joiner
    '\u{200D}', // zero-width joiner
    '\u{2060}', // word joiner
    '\u{2061}', // function application
    '\u{2062}', // invisible times
    '\u{2063}', // invisible separator
    '\u{2064}', // invisible plus
    '\u{3164}', // hangul filler
    '\u{FEFF}', // zero-width no-break space / BOM
    '\u{FFA0}', // halfwidth hangul filler
    // Bidi marks / embeddings / overrides / isolates
    '\u{061C}', // arabic letter mark
    '\u{200E}', // left-to-right mark
    '\u{200F}', // right-to-left mark
    '\u{202A}', // left-to-right embedding
    '\u{202B}', // right-to-left embedding
    '\u{202C}', // pop directional formatting
    '\u{202D}', // left-to-right override
    '\u{202E}', // right-to-left override
    '\u{2066}', // left-to-right isolate
    '\u{2067}', // right-to-left isolate
    '\u{2068}', // first strong isolate
    '\u{2069}', // pop directional isolate
    // Variation selectors (text-injection hiding)
    '\u{FE00}', // variation selector-1
    '\u{FE01}', // variation selector-2
    '\u{FE02}', // variation selector-3
    '\u{FE03}', // variation selector-4
    '\u{FE04}', // variation selector-5
    '\u{FE05}', // variation selector-6
    '\u{FE06}', // variation selector-7
    '\u{FE07}', // variation selector-8
    '\u{FE08}', // variation selector-9
    '\u{FE09}', // variation selector-10
    '\u{FE0A}', // variation selector-11
    '\u{FE0B}', // variation selector-12
    '\u{FE0C}', // variation selector-13
    '\u{FE0D}', // variation selector-14
    '\u{FE0E}', // variation selector-15
    '\u{FE0F}', // variation selector-16
];

/// Invisible / format code points whose presence is a standalone prompt-injection signal.
///
/// This excludes variation selectors and U+200D because Unicode defines them as components of ordinary presentation and emoji ZWJ sequences.
/// They remain in [`INVISIBLE_FORMAT_CHARS`], so prompt sanitizers still remove them before matching or interpolation.
pub const INJECTION_SIGNAL_CHARS: &[char] = &[
    '\u{00AD}', // soft hyphen
    '\u{034F}', // combining grapheme joiner
    '\u{115F}', // hangul choseong filler
    '\u{1160}', // hangul jungseong filler
    '\u{17B4}', // khmer vowel inherent aq
    '\u{17B5}', // khmer vowel inherent aa
    '\u{180E}', // mongolian vowel separator
    '\u{200B}', // zero-width space
    '\u{200C}', // zero-width non-joiner
    '\u{2060}', // word joiner
    '\u{2061}', // function application
    '\u{2062}', // invisible times
    '\u{2063}', // invisible separator
    '\u{2064}', // invisible plus
    '\u{3164}', // hangul filler
    '\u{FEFF}', // zero-width no-break space / BOM
    '\u{FFA0}', // halfwidth hangul filler
    '\u{061C}', // arabic letter mark
    '\u{200E}', // left-to-right mark
    '\u{200F}', // right-to-left mark
    '\u{202A}', // left-to-right embedding
    '\u{202B}', // right-to-left embedding
    '\u{202C}', // pop directional formatting
    '\u{202D}', // left-to-right override
    '\u{202E}', // right-to-left override
    '\u{2066}', // left-to-right isolate
    '\u{2067}', // right-to-left isolate
    '\u{2068}', // first strong isolate
    '\u{2069}', // pop directional isolate
];

/// Format characters retained for sanitization but excluded as standalone injection signals because they participate in ordinary emoji or presentation sequences.
pub const EMOJI_SEQUENCE_CHARS: &[char] = &[
    '\u{200D}', // zero-width joiner
    '\u{FE00}', // variation selector-1
    '\u{FE01}', // variation selector-2
    '\u{FE02}', // variation selector-3
    '\u{FE03}', // variation selector-4
    '\u{FE04}', // variation selector-5
    '\u{FE05}', // variation selector-6
    '\u{FE06}', // variation selector-7
    '\u{FE07}', // variation selector-8
    '\u{FE08}', // variation selector-9
    '\u{FE09}', // variation selector-10
    '\u{FE0A}', // variation selector-11
    '\u{FE0B}', // variation selector-12
    '\u{FE0C}', // variation selector-13
    '\u{FE0D}', // variation selector-14
    '\u{FE0E}', // variation selector-15
    '\u{FE0F}', // variation selector-16
];

#[cfg(test)]
mod tests {
    use super::{EMOJI_SEQUENCE_CHARS, INJECTION_SIGNAL_CHARS, INVISIBLE_FORMAT_CHARS};
    use std::collections::BTreeSet;

    #[test]
    fn invisible_format_chars_has_no_duplicates() {
        let mut sorted: Vec<char> = INVISIBLE_FORMAT_CHARS.to_vec();
        sorted.sort_unstable();
        let len_before = sorted.len();
        sorted.dedup();
        assert_eq!(
            len_before,
            sorted.len(),
            "INVISIBLE_FORMAT_CHARS must not contain duplicate code points"
        );
    }

    #[test]
    fn injection_signal_and_emoji_sequence_chars_partition_sanitizer_set() {
        let sanitizer: BTreeSet<char> = INVISIBLE_FORMAT_CHARS.iter().copied().collect();
        let signals: BTreeSet<char> = INJECTION_SIGNAL_CHARS.iter().copied().collect();
        let emoji_sequences: BTreeSet<char> = EMOJI_SEQUENCE_CHARS.iter().copied().collect();
        let union: BTreeSet<char> = signals.union(&emoji_sequences).copied().collect();

        assert_eq!(signals.len(), INJECTION_SIGNAL_CHARS.len());
        assert_eq!(emoji_sequences.len(), EMOJI_SEQUENCE_CHARS.len());
        assert!(signals.is_disjoint(&emoji_sequences));
        assert_eq!(union, sanitizer);
    }
}
