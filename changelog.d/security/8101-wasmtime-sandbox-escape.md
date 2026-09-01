Move `wasmtime` off 47.0.3, which carries a filesystem sandbox escape.
RUSTSEC-2026-0269 (CVSS 8.8) lets a guest reach outside the preopened directory when a path or symlink ends in a trailing slash, and RUSTSEC-2026-0268 (CVSS 6.9) lets a guest drive an unbounded host heap allocation through WASIp3 streams; both were published 2026-08-20 and are fixed in 47.0.4.
This is the sandbox `librefang-runtime` runs WASM skills inside, so the escape sits on the boundary the sandbox exists to enforce rather than on a transitive dependency nobody reaches.
`main` has been red since 2026-08-31 because `cargo audit` was correctly refusing it — 22 of 24 CI jobs pass and only `Security` (and `CI Gate` downstream of it) fail.
Staying on the 47 line rather than taking 48.0.1 is deliberate: 48 requires Rust 1.95.0, above this workspace's pinned toolchain. (#8101) (@houko)
