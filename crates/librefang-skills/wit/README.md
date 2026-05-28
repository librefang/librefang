# `librefang-skills/wit/` — Component Model interfaces for librefang plugins

This directory is the canonical WIT (WebAssembly Interface Type) home for
the librefang plugin contract. Anything a Component-Model plugin can
talk to the host through is described here.

## Files

- `host.wit` — `librefang:host@0.1.0` package. Six interfaces (`fs`,
  `net`, `kv`, `agent`, `env`, `time`) modeling the existing
  capability-checked host functions in
  `librefang-runtime::host_functions`.
- `world.wit` — `librefang:plugin@0.1.0` package. The `plugin` world a
  plugin's Component implements: imports any subset of the host
  interfaces, exports `run() -> result<_, plugin-error>`.

## Semver discipline

The WIT package versions follow strict semver from the plugin's
perspective:

- **0.1.x — additive only.** New functions, new interfaces, new
  variants on `host-error` / `plugin-error` MAY land. Renaming or
  removing anything MUST NOT.
- **0.2.0 — first breaking bump.** Reserved for the day a real
  redesign is needed. Existing 0.1.x plugins keep working through a
  deprecation window (host implements both worlds side-by-side for at
  least one release).
- **1.0.0 — long-term stable.** Targeted once the surface has shaken
  out via real-world plugins. Not on the immediate roadmap.

When you propose a WIT change in a PR, include a one-line semver
classification (`additive` vs `breaking`) in the PR description so
reviewers can apply the right policy.

## Validating

The phase-4 dev image (`Dockerfile.rust-dev`) ships `wasm-tools`, which
parses the WIT directly:

```bash
wasm-tools component wit crates/librefang-skills/wit/
```

That should print the world definition with all imports resolved. Any
typo or cross-package reference error surfaces here.

## Building a plugin against this WIT

In Rust, `cargo component` reads this directory via the plugin's own
`Cargo.toml`:

```toml
[package.metadata.component.target]
path = "../path/to/librefang-skills/wit"
world = "plugin"
```

For Python / JS / Go / C recipes that consume the same WIT files, see
`docs/development/polyglot-dev-image.md` (per-language sections).
