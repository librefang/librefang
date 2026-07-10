//! Shared text-normalization constants used across security boundaries.

/// The canonical set of invisible / format Unicode code points that carry no
/// legitimate semantic content in human text but are frequently used to smuggle
/// hidden instructions past a literal scanner or to reorder visible text.
///
/// This is the single source of truth for that set.
/// Every place that strips these code points before scanning skill content or before interpolating text into an LLM prompt references this const instead of maintaining its own copy, so the set cannot silently drift out of sync between crates:
/// `librefang-runtime::injection_guard`, `librefang-runtime::prompt_builder::sanitize_for_prompt`, and `librefang-kernel::kernel::prompt_context` all point here.
/// `librefang-skills::verify` keeps a parallel `(char, &str)` table because it also emits a human-readable label per code point in its warnings; a unit test there asserts its char set equals this const so the two never diverge.
///
/// Covers zero-width / joiner code points, bidi marks / embeddings / overrides / isolates, and the text-variation selectors (U+FE00–U+FE0F).
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

#[cfg(test)]
mod tests {
    use super::INVISIBLE_FORMAT_CHARS;

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
}
