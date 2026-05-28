# Change C-002 — WIT package for librefang plugin host world

**Phase:** phase-5-plugin-host-crate
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:** new `crates/librefang-skills/wit/host.wit`,
                   new `crates/librefang-skills/wit/world.wit`,
                   new `crates/librefang-skills/wit/README.md`

## What landed

Single-package WIT layout under `crates/librefang-skills/wit/`:

- `host.wit` — six host interfaces (`fs`, `net`, `kv`, `agent`, `env`,
  `time`) modeling the existing `host_functions::dispatch` JSON-RPC
  surface in librefang-runtime. Plus a shared `host-types` interface
  carrying the `host-error` variant. Parameter shapes mirror the
  existing dispatch exactly — Phase 5 does NOT redesign the surface.
- `world.wit` — the `plugin` world, imports each host interface,
  exports `run() -> result<_, plugin-error>`. Plus a `plugin-types`
  interface carrying the `plugin-error` variant.
- `README.md` — semver discipline (0.1.x additive-only), validation
  command, plugin authoring pointer.

Package id is `librefang:plugin@0.1.0`.

## Verification

```
docker run --rm -v "$PWD":/workspace -w /workspace \
    librefang-rust-dev:c006-test \
    wasm-tools component wit crates/librefang-skills/wit/
```

Exit 0; full world definition prints back with all imports resolved.

## Issues hit + fixed

1. **`list` is a WIT reserved keyword.** Initial `fs.list: func(...)`
   collided with the built-in `list<u8>` type constructor. Renamed to
   `list-entries`.
2. **Single package per directory rule.** Initially split into
   `librefang:host@0.1.0` and `librefang:plugin@0.1.0` in the same dir
   — wasm-tools rejected with "package identifier does not match".
   Collapsed both into `librefang:plugin@0.1.0`. A future split into
   distinct packages would require the `wit/deps/<owner>/<pkg>/...`
   layout.
3. **Leading `//` block before `package` parsed as a package doc
   comment**, conflicting across the two files. Moved the file-level
   blurb to AFTER the `package` decl (or used `///` if it should be a
   real doc comment).
4. **Interface name collision** — both files defined `interface
   types`. Renamed to `host-types` and `plugin-types`.

## QA-gate note

This change touched 3 new files (≥3 threshold). Documentation-only
content per spec (WIT + README). Refer to the assessment's quality
mitigation: the typed surface and the existing dispatch surface are
verified consistent by the C-003 shim's one-test-per-interface
round-trip; standalone QA on the WIT itself is `wasm-tools component
wit` (already passing).
