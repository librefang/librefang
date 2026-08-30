Release builds produce their cross-compiled artifacts again.
`dtolnay/rust-toolchain` installed each target on `stable`, but `rust-toolchain.toml` pins the workspace to its MSRV and cargo builds with that toolchain instead, so every cross target failed with "can't find crate for `std`" while the native ones passed — v2026.8.30 shipped 23 of the 48 assets its predecessor did.
The target is now added to the toolchain that actually builds, in every workflow that declares one — including the manual re-release workflows any repair of a broken release has to run through.
(#8007) (@houko)
