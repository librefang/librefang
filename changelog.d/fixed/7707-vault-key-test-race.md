`cargo test -p librefang-api --lib` no longer fails roughly one run in two on a vault decryption error.
Two test modules each pinned the process-global `LIBREFANG_VAULT_KEY` to a different value, each under its own `std::sync::Once` and each documenting that it was the only writer.
`cargo test` runs a crate's tests as threads in one process, so whichever module wrote last won and the other's freshly written vault could no longer be decrypted — surfacing as `Crypto("Decryption failed: aead::Error")` in whichever test lost the race.
CI never saw it because nextest gives every test its own process, which is also why the contributor-facing command in CLAUDE.md was the one that broke.
The key and its `Once` now have a single owner, `crate::test_vault`, and both modules call it (#7707) (@houko)
