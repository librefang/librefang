The `cargo-deny` CI job pinned `EmbarkStudios/cargo-deny-action` to the mutable `v2` tag, so a compromised or repointed tag on that action would run inside CI with no additional review.
The workflow now pins to an immutable commit SHA, keeping the `v2` release tag in a trailing comment for readability (#6958) (@houko)
