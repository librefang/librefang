# `examples/plugins/` — per-language WASM Component plugins

Phase-6 deliverable. Each subdirectory is a minimal, runnable plugin
targeting the **`librefang:plugin@0.1.0/plugin`** world (see
`crates/librefang-skills/wit/`). Together they exercise the
Component Model execute path landed in Phase 5
([`docs/development/plugin-host.md`](../../docs/development/plugin-host.md)).

## Layout contract

Each example lives in a self-contained subdirectory:

```
examples/plugins/<lang>-<feature>/
├── README.md                       # what this plugin does
├── skill.toml                      # manifest with host_capabilities = [...]
├── <language build manifest>       # Cargo.toml | package.json | go.mod | Makefile
├── <source>                        # plugin.rs | plugin.py | plugin.js | plugin.go | plugin.c
└── pre-built/
    └── plugin.wasm                 # committed, ≤ 200 KB, reproducible via xtask
```

The `pre-built/plugin.wasm` is the ONE artefact that gets checked in
per example. Everything else in the dir is either source (committed)
or build output (ignored via `.gitignore`).

## Examples (Phase 6 set)

| Directory | Toolchain | Capability | What it does |
|---|---|---|---|
| `rust-fs-cat/` | `cargo component build` | `fs` | Reads `/tmp/test-input.txt` and writes its content to `/tmp/test-output.txt`. |
| `python-hello-time/` | `componentize-py` | `time` | Reads wall-clock time via `host.time.now()`. |
| `js-kv-counter/` | `jco componentize` | `kv` | Increments the `counter` key in host key-value store. |
| `go-env-greet/` | TinyGo + `wit-bindgen-go` | `env` | Reads `PLUGIN_NAME` env var, returns a greeting. |
| `c-noop/` | `wasi-clang` + `wit-bindgen-c` | (none) | Proves the C build pipeline with an empty `run`. |

Capabilities NOT yet covered by Phase-6 examples: `net` and `agent`.
Both are Phase-7 candidates (net needs SSRF guard + mocked endpoint;
agent needs multi-agent fixture).

## Rebuilding the `.wasm` artefacts

The committed `pre-built/plugin.wasm` files must be reproducible
inside the pinned `librefang-rust-dev` image (Phase-4 dev image with
cargo-component / componentize-py / jco / TinyGo / wit-bindgen-c
preinstalled).

```bash
# Rebuild every example (inside the dev image):
cargo xtask plugins-rebuild

# Rebuild just one:
cargo xtask plugins-rebuild rust-fs-cat

# Dry-run (print plan without executing):
cargo xtask plugins-rebuild --dry-run
```

Each rebuild enforces a per-file size budget (`MAX_PLUGIN_WASM_BYTES
= 200 KB`) — a bust signals the example has drifted from its
"minimal canonical" goal and should be simplified rather than
indulged.

## Running an example through the librefang plugin host

The Phase-5 standalone harness loads any `.wasm` Component:

```bash
# Load-only — proves the file is a structurally valid Component
cargo run --example load_and_run -p librefang-runtime -- \
    examples/plugins/rust-fs-cat/pre-built/plugin.wasm

# Load + invoke — actually runs the plugin
cargo run --example load_and_run -p librefang-runtime -- \
    examples/plugins/rust-fs-cat/pre-built/plugin.wasm \
    --invoke
```

For automated test coverage, see the per-language integration tests
at `crates/librefang-runtime/tests/plugin_example_*.rs` (Phase-6
C-007).

## Authoring a new plugin

Start from the language whose example most closely matches your
needs, copy the directory under a new name, and adjust:

1. `skill.toml` — declare `host_capabilities` matching the
   interfaces your plugin imports.
2. Build manifest — bump package name and dependencies.
3. Source — implement `run` against the WIT bindings.
4. Rebuild — `cargo xtask plugins-rebuild <new-name>` (after
   registering the builder in `xtask/src/plugins.rs`).

See [`docs/development/plugin-host.md`](../../docs/development/plugin-host.md)
for the detailed authoring recipes per language.
