# Custom WASM Skill Example

A word-counter skill — the WASM twin of [`custom-skill-python`](../custom-skill-python) — written with the [`librefang-skill`](../../sdk/rust/librefang-skill) SDK.
It is pure compute (no host calls), so it needs no capabilities and runs fully under `librefang skill test`.

## Layout

- `Cargo.toml` — a `cdylib` crate; `[lib] name = "skill"` makes the artifact `skill.wasm`.
- `src/lib.rs` — the handler, registered with the `skill!` macro.
- `skill.toml` — the manifest; `entry = "skill.wasm"` (the built artifact at the skill root).

## Build and test

```bash
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/skill.wasm skill.wasm

librefang skill test . --input '{"text": "Hello world. Bye!"}'
# → { "words": 3, "sentences": 2, "characters": 17 }
```

The `.wasm` is a build artifact and is git-ignored; rebuild it with the steps above.
Because the skill packager excludes `target/`, the manifest references the artifact at the skill root, which is why the build copies it out of `target/`.

## Use it as a template

Copy this directory, then in `Cargo.toml` replace the path dependency with the published crate:

```toml
librefang-skill = "0.1"
```

`librefang skill create` (choose the `wasm` runtime) scaffolds the same structure for a new skill.
