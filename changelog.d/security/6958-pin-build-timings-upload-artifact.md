The `build-timings` workflow still referenced `actions/upload-artifact` by the mutable `v4` tag, which is exactly the supply-chain gap this PR's sibling change to `cargo-deny.yml` was closing.
It now pins to the same immutable commit already used for `v4` elsewhere in the workflow set (`coverage.yml`), keeping the tag in a trailing comment for readability (#6958) (@houko)
