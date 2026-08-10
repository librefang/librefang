Proactive-memory extraction now moves its prompt-size cutoff to a valid UTF-8
boundary before searching for a newline or truncating. Long conversations that
place a CJK character, emoji, or other multi-byte character across the 8,000-byte
boundary no longer abort automatic memory extraction with a slicing panic
(#6778) (@houko)
