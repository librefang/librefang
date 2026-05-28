# `js-kv-counter` — JavaScript + jco componentize, exercises `kv`

A minimal librefang:plugin Component written in JavaScript. Reads a
`"counter"` key from the host KV store, increments it, and writes it
back. On the first call the key starts at `"1"`.

## Build

The committed `pre-built/plugin.wasm` is the canonical artefact. To
regenerate it (e.g. after updating the WIT contract):

```bash
cargo xtask plugins-rebuild js-kv-counter
```

Under the hood this invokes:

```bash
cd examples/plugins/js-kv-counter
jco componentize app.js \
    --wit ../../../crates/librefang-skills/wit \
    --world-name plugin \
    -o pre-built/plugin.wasm
```

The Phase-6 size budget for JS is 8 MB (jco bundles StarlingMonkey JS engine).

## How the source maps to the librefang:plugin world

`app.js` is a standard ES module. `import { get, set } from
'librefang:plugin/kv'` uses the WIT interface ID as the module
specifier — jco resolves these to the host imports declared in the
`plugin` world. `export function run()` is the world's single
export. A plain `return` maps to `Ok(())`.
