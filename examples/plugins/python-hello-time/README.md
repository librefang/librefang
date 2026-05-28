# `python-hello-time` — Python + componentize-py, exercises `time`

A minimal librefang:plugin Component written in Python. Calls
`time.now()` via the `librefang:plugin/time` host import to obtain
the current Unix epoch (seconds) and returns successfully.

## Build

The committed `pre-built/plugin.wasm` is the canonical artefact. To
regenerate it (e.g. after updating the WIT contract):

```bash
cargo xtask plugins-rebuild python-hello-time
```

Under the hood this invokes:

```bash
cd examples/plugins/python-hello-time
componentize-py --wit-path ../../../crates/librefang-skills/wit \
    --world plugin componentize app.py -o pre-built/plugin.wasm
```

The Phase-6 size budget is 200 KB per artefact.

## How the source maps to the librefang:plugin world

`componentize-py` reads the WIT at
`crates/librefang-skills/wit` and generates Python bindings under
`wit_world/` (in-process during the `componentize` call — these are
not committed). `app.py` imports
`wit_world.imports.time as time_host` and calls `time_host.now()`.

The class `App` implements the `WitWorld` protocol (the single
`run` export). A plain `return` maps to `Ok(())`.
