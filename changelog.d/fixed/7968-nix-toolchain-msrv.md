`nix build` works again.
The flake compiled with `rust-bin.stable.latest`, resolved out of a pinned rust-overlay input five months behind the workspace, so every build was handed rustc 1.94.0 and rejected with "rustc 1.94.0 is not supported by the following packages ... requires rustc 1.94.1".
The flake now reads its channel from `rust-toolchain.toml` like every other consumer of that pin, and `scripts/check-toolchain-versions.sh` — which had never covered the flake — keeps it that way.
(#7968) (@houko)
