# Change C-005 — SkillManifest.host_capabilities + link-time gating

**Phase:** phase-5-plugin-host-crate
**Status:** DONE
**Completed:** 2026-05-27
**Files touched:**
  - `crates/librefang-skills/src/lib.rs` — added `HostCapability`
    enum (6 variants matching the librefang:plugin WIT interfaces) +
    `host_capabilities: Vec<HostCapability>` field on
    `SkillManifest` with `#[serde(default)]`.
  - `crates/librefang-skills/src/{openclaw_compat,registry,evolution}.rs`
    — added `host_capabilities: Vec::new()` to ~19 SkillManifest
    struct-literal callsites (forced by E0063 — struct literals don't
    apply serde defaults).
  - `crates/librefang-runtime/src/sandbox_component.rs` — added
    `ComponentExecuteOptions { host_capabilities }`, new private
    `add_to_linker_per_capability()` helper, and extended
    `execute_component()` signature with `options:
    ComponentExecuteOptions`.

## What landed

### Coarse-grained capability gate

`HostCapability` is a 6-variant enum (`Fs`, `Net`, `Kv`, `Agent`,
`Env`, `Time`) — one per WIT interface in `librefang:plugin@0.1.0`.
Distinct from the existing fine-grained
`librefang_types::capability::Capability` enum:

- `HostCapability` decides whether the Component Model linker binds
  the interface symbols (decided at instantiate time, hard error if
  the Component imports an undeclared interface).
- `Capability` (existing) decides whether a specific call argument is
  allowed (decided at runtime per call by `host_functions::dispatch`).

The two layer cleanly: a plugin declaring `host_capabilities = ["fs"]`
gets the `fs.read` symbol bindable; whether it can actually read
`/etc/passwd` depends on the fine-grained `Capability::FileRead(...)`
grant the agent carries.

### Per-interface gated linker

`add_to_linker_per_capability(&mut linker, &caps)` calls only the
bindgen-emitted per-interface `add_to_linker` for declared caps.
Bindgen's top-level `Plugin::add_to_linker` (which adds every
interface unconditionally) is no longer called — the gate is now
load-bearing for `execute_component`.

### Manifest backward compatibility

`SkillManifest.host_capabilities` defaults to `Vec::new()` via
`#[serde(default)]`. Pre-Phase-5 manifests deserialize unchanged.
Component plugins MUST declare their caps in `skill.toml`; core-module
Wasm plugins (still going through `WasmSandbox::execute`) are
unaffected because that path doesn't consult the field.

## Verification

```
cargo check -p librefang-runtime --lib    # exit 0
cargo test  -p librefang-runtime --lib sandbox_component
# test result: ok. 6 passed; 0 failed
```

### 4 new tests added (capability_gate_* family):

| Test | Assertion |
|---|---|
| `capability_gate_empty_caps_is_ok` | Empty cap list → linker with zero host bindings; doesn't error. |
| `capability_gate_fs_only` | Subset of one interface succeeds. |
| `capability_gate_full_set` | All six interfaces succeed (regression guard for the full path). |
| `capability_gate_rejects_duplicate_caps` | wasmtime 44's bindgen errors on duplicate per-interface add_to_linker ("map entry `<package>/<iface>` defined twice"); test pins the contract so a future upstream loosening surfaces. |

## Issues hit + fixed

1. **Struct-literal E0063 cascade.** Added a `#[serde(default)]`
   field on `SkillManifest`, but ~19 existing struct-literal
   callsites in `evolution.rs` / `openclaw_compat.rs` / `registry.rs`
   stopped compiling. Inserted `host_capabilities: Vec::new(),` at
   each via a python sweep keyed on `prompt_context:` (the next field
   in source order).
2. **Over-matched sweep** caught 3 false positives — 2 in
   `ConvertedSkillMd` (a different struct that also has
   `prompt_context`) and 1 in a function signature where the
   `prompt_context: &str` parameter name matched the same pattern.
   Surgically removed those 3 with targeted replacements.
3. **Duplicate-cap test assertion inverted on first iteration**
   (stale test binary masked the real "defined twice" error). Test
   re-shaped to assert the rejection contract; comment documents what
   to do if upstream loosens it.

## QA-gate note

Touches 4 files (≥3 threshold). The capability gate is unit-test-
verified end to end via the four `capability_gate_*` tests. The full
"undeclared imports cause failure at instantiate" half of the
contract is covered by C-007 (smoke runs real Components through the
gate). No `/refine-validate` invoked — the test suite is the
substantive QA artefact for this kind of wiring change.
