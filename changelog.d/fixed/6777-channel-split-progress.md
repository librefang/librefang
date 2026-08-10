The public Rust channel message splitter now always consumes at least one complete character, including when an incomplete HTML entity begins a chunk or the byte limit falls inside a multi-byte UTF-8 character.
Custom Rust channel consumers can no longer enter a non-progressing split loop on those inputs (#6777) (@houko)
