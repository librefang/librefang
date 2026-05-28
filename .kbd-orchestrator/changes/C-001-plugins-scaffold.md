# Change C-001 — examples/plugins/ scaffold + xtask plugins-rebuild

**Phase:** phase-6-plugin-examples
**Status:** DONE
**Completed:** 2026-05-28
**Files touched:**
  - new `examples/plugins/README.md`
  - new `examples/plugins/.gitignore`
  - new `xtask/src/plugins.rs` (137 lines)
  - `xtask/src/main.rs` — module declaration + Subcommand variant +
    dispatch arm

## What landed

### `examples/plugins/` directory + author contract

- **README.md** — defines the layout contract (per-example
  subdirectory; build manifest + source + `skill.toml` +
  `pre-built/plugin.wasm`), the five Phase-6 examples, rebuild
  invocation, harness invocation via load_and_run, and how to
  author a new plugin.
- **.gitignore** — keeps build outputs (Rust target/, Node
  node_modules/, Go gen/ + go.sum, C *.o, etc.) out of the index
  while whitelisting the ONE committed artefact path per example
  (`*/pre-built/plugin.wasm`).

### `cargo xtask plugins-rebuild [<example>]`

- **xtask/src/plugins.rs** — registry-driven builder dispatcher.
  Registry starts empty; C-002…C-006 each append one entry.
  - `PluginsRebuildArgs` — `example: Option<String>` + `--dry-run`.
  - `BUILDERS: &[(&str, Builder)]` — type-aliased function
    pointers, one per language. Each builder takes
    `&workspace_root` and writes a `pre-built/plugin.wasm`.
  - `MAX_PLUGIN_WASM_BYTES: u64 = 200 * 1024` — per-artefact size
    guardrail. Fail-loud if a plugin bloats past the 200 KB
    ceiling.
  - `enforce_size_budget` — runs after each builder; prints the
    actual byte count for visibility.
  - Empty-registry early return prints a clear "C-002..C-006 will
    register" message so the no-op state is self-documenting.
- **xtask/src/main.rs** — mod declaration (alphabetical, between
  `migrate` and `pre_commit`) + `PluginsRebuild(plugins::PluginsRebuildArgs)`
  Subcommand variant + dispatch arm.

## Verification

```
$ cargo check -p xtask
   Compiling xtask
    Finished `dev` profile in 21s; exit 0
$ cargo run -q -p xtask -- plugins-rebuild
cargo xtask plugins-rebuild: registry is empty. Phase-6 C-002..C-006 will
register per-language builders. Examples directory: examples/plugins
$ cargo run -q -p xtask -- plugins-rebuild --dry-run
(same — empty registry short-circuits)
$ cargo run -q -p xtask -- plugins-rebuild bogus
(same — empty registry short-circuits before name lookup)
```

All three paths short-circuit on the empty registry as designed.
C-002 will exercise the dry-run + unknown-name branches once a
builder is registered.

## Issues hit + fixed

1. **xtask convention is `Box<dyn std::error::Error>`, not `anyhow`.**
   Initial draft used `anyhow::Result<()>` + `anyhow::bail!` —
   anyhow isn't in `xtask/Cargo.toml`. Rewrote to match the
   project's existing pattern (see `xtask/src/migrate.rs`).
2. **Closure error mapping was over-engineered.** Final version
   uses `&str` and `format!(...).into()` patterns matching
   `Box<dyn Error>::From` impls — same as the rest of the
   binary.

## QA-gate note

Touches 4 files (≥3 threshold). The substantive QA artefact is
`cargo check -p xtask` + the three smoke invocations above. No
`/refine-validate` needed for a scaffolding change with no
behavioural surface yet.
