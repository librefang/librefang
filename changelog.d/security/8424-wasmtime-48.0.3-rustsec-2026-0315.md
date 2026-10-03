The workspace lockfile moves the wasmtime runtime and its `wasmtime-internal-*`, cranelift and pulley families from 48.0.2 to 48.0.3, closing RUSTSEC-2026-0315 (medium) and RUSTSEC-2026-0316 (low) in the WASM sandbox the runtime links against.
Both advisories were reported on 2026-09-24 — after the runs that last reported the Security lane green — so the lane fails on any branch that re-runs it against a 48.0.2 lockfile, including main's own.
The patched release sits inside the workspace's existing `wasmtime = "48"` requirement, so only `Cargo.lock` moves. (#8424) (@DaBlitzStein)
