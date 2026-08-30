Move `chacha20` off the yanked 0.10.0 so the `cargo-deny (advisories)` gate passes again.
The job had been failing on `main` since 2026-08-28 with `error[yanked]: detected yanked crate (try 'cargo update -p chacha20')`, which meant a supply-chain gate was red on every branch cut after that point — and a gate that is always red is one nobody reads, so a genuine advisory landing behind it would have arrived looking exactly like the noise.
`chacha20` is transitive (`rand 0.10.2` and the `cipher`-family crates pull it in), so this is a lockfile bump to 0.10.2 and nothing else. (#8035) (@houko)
