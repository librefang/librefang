# Polyglot dev image (`librefang-rust-dev`)

Phase-4 turned `Dockerfile.rust-dev` into a polyglot WASM compile host so
contributors authoring librefang plugins in Rust, Python, TypeScript,
JavaScript, Go, or C can target WebAssembly from a single image without
installing each toolchain on their own machine.

## What's in the image

| Stack | Versions (pinned in `Dockerfile.rust-dev`) |
|---|---|
| Rust | stable (from trixie base) + nightly + targets `wasm32-{unknown-unknown,wasip1,wasip2}` on both |
| Rust WASM CLIs | wasmtime-cli, wasm-tools, cargo-component, wit-bindgen, wasm-bindgen, wasm-pack, wasm-opt |
| wasmtime C-API | `libwasmtime.so` + `wasmtime.h` at `/usr/local/{lib,include}` (v45.x) |
| Python | apt python3.13 (+dev, +venv) for pyo3 builds; uv-managed `maturin` + `componentize-py` for plugin authoring |
| Node/JS/TS | Node 24 LTS + pnpm 10.x + Bun + AssemblyScript + `@bytecodealliance/componentize-js` + wabt |
| Go | Go 1.26.3 + TinyGo 0.41.1 |
| C/C++ | wasi-sdk 27 + binaryen (apt) + wabt (apt) |

It's the dev/CI image only — the production `Dockerfile` deliberately
stays slim and ships only the wasmtime C-API shared library (no compile
toolchain).

## Building locally

```bash
DOCKER_BUILDKIT=1 docker build \
    -f Dockerfile.rust-dev \
    -t librefang-rust-dev:latest .
```

A full cold build takes 8–15 minutes depending on bandwidth; subsequent
builds reuse BuildKit cache mounts (apt, cargo registry, uv cache) and
finish in well under a minute when only later layers change.

## Using the image

The repo's `cargo` wrapper script invokes this image when running
`cargo check --workspace --lib` on hosts without a native Rust toolchain:

```bash
LIBREFANG_MOUNT_BASE=/path/to/workspace-parent \
LIBREFANG_RUST_IMAGE=librefang-rust-dev:latest \
    cargo check --workspace --lib
```

Plugin authors can drop into an interactive shell with the workspace
mounted:

```bash
docker run --rm -it \
    -v "$PWD":/workspace -w /workspace \
    librefang-rust-dev:latest \
    bash
```

## Plugin recipes (one per language)

Each recipe assumes you've mounted the workspace at `/workspace` per the
shell snippet above and that you're inside the dev image. Every recipe
produces a `.wasm` artefact that wasmtime can load. The end-to-end smoke
that exercises every recipe at once lives at
`scripts/test-wasm-toolchain.sh` — run that to confirm the image is
healthy before committing toolchain changes.

### Rust → WASI Preview 2

```bash
cargo new --bin hello-rust && cd hello-rust
cargo +nightly build --release --target wasm32-wasip2
wasmtime run --wasi cli target/wasm32-wasip2/release/hello-rust.wasm
```

For Component Model output instead of a core WASI module, use
`cargo component build --release` (requires a `wit/` directory describing
the component world).

### Python → WASI 0.2 Component

```bash
mkdir hello-py && cd hello-py
cat > hello.wit <<EOF
package local:hello;
world hello { export run: func(); }
EOF
cat > hello.py <<EOF
class WitWorld:
    def run(self) -> None:
        print("hello-python-wasm")
EOF
componentize-py -d . -w hello componentize hello -o hello.wasm
wasm-tools validate hello.wasm    # sanity check
```

Componentize-py looks up exports as attributes of a `WitWorld` class
inside the named module. The class name is fixed regardless of the
world name in the WIT.

For raw-speed Python that doesn't need Component Model, `py2wasm` is
the Nuitka-based alternative:

```bash
py2wasm hello.py -o hello.wasm
```

### TypeScript → WASM (AssemblyScript)

```bash
mkdir hello-ts && cd hello-ts
cat > hello.ts <<EOF
@external("wasi_snapshot_preview1", "fd_write")
declare function fd_write(fd: i32, iovs: i32, iovs_len: i32, nwritten: i32): i32;
export function _start(): void {
    const msg = "hello-typescript-wasm\\n";
    const buf: i32 = 100;
    for (let i: i32 = 0; i < msg.length; i++) {
        store<u8>(buf + i, msg.charCodeAt(i));
    }
    store<i32>(0, buf);
    store<i32>(4, msg.length);
    fd_write(1, 0, 1, 8);
}
EOF
asc hello.ts -O --runtime stub --use abort= --outFile hello.wasm
wasmtime run hello.wasm
```

`--runtime stub` strips the GC runtime; `--use abort=` stubs the abort
import that AssemblyScript otherwise expects the host to provide.

### JavaScript → WASI 0.2 Component (componentize-js via jco)

```bash
cat > hello.js <<EOF
export function run() {
    console.log("hello-js-wasm");
}
EOF
cat > hello.wit <<EOF
package local:hello;
world hello { export run: func(); }
EOF
jco componentize hello.js --wit hello.wit -o hello.wasm
wasm-tools validate hello.wasm   # sanity check
```

componentize-js (invoked via `jco componentize`) produces a WASI 0.2
Component embedding SpiderMonkey ahead-of-time. Like componentize-py,
the output is a Component (not a wasi:cli module), so
`wasmtime run hello.wasm` won't auto-invoke; the librefang plugin host
loads it via the Component Model linker.

### Go → WASI Preview 2 (TinyGo, primary)

```bash
cat > hello.go <<EOF
package main
func main() { println("hello-go-wasm") }
EOF
tinygo build -target=wasip2 -o hello.wasm hello.go
wasmtime run hello.wasm
```

For wasip1 (or a binary that needs full Go reflection / cgo), use
mainline Go instead:

```bash
GOOS=wasip1 GOARCH=wasm go build -o hello.wasm hello.go
wasmtime run --wasi cli hello.wasm
```

### C → WASI (wasi-sdk)

```bash
cat > hello.c <<EOF
#include <stdio.h>
int main(void) { puts("hello-c-wasm"); return 0; }
EOF
${WASI_SDK_PATH}/bin/clang --target=wasm32-wasi \
    --sysroot=${WASI_SDK_PATH}/share/wasi-sysroot \
    hello.c -o hello.wasm
wasmtime run --wasi cli hello.wasm
```

`WASI_SDK_PATH` is exported by the dev image to `/opt/wasi-sdk`. For
convenience the env vars `CC_wasm32_wasip1` etc. are pre-set so
cargo/cmake builds pick up the right clang automatically.

## Loading the compiled `.wasm` via librefang

This doc covers **compile**. The next step — loading a `.wasm` and
binding it to librefang's typed host interfaces — is the Phase-5
Component Model plugin host. See
[`docs/development/plugin-host.md`](plugin-host.md) for the WIT
contract (`librefang:plugin@0.1.0`), the `HostCapability` link-time
gate, AOT cache, and per-language authoring recipes that turn each
hello-world above into a real librefang plugin.

## What the image does NOT ship

These belong in the production runtime image, not the dev image:

- The librefang daemon binary
- The dashboard SPA bundle
- The production entrypoint

These are kept out of the dev image even though they share the same
codebase — the production `Dockerfile` is a separate build:

- A future plugin host crate that uses the wasmtime C-API. The C-API is
  staged in production via `COPY --from=builder` (C-007).
- WASI 0.3 (async). Upstream not yet stable.
- WasmEdge AI/tensor extensions. Defer until a concrete need.

## Verifying the image after changes

Whenever `Dockerfile.rust-dev` or `scripts/test-wasm-toolchain.sh`
changes, run:

```bash
docker run --rm -v "$PWD":/workspace -w /workspace \
    librefang-rust-dev:latest \
    /workspace/scripts/test-wasm-toolchain.sh
```

The smoke compiles a "hello-${lang}-wasm" program in each of the six
languages and validates the result. CI runs the same script via
`.github/workflows/wasm-toolchain.yml` on every PR that touches the
relevant files.

## Phase-4 traceability

Phase plan: [`.kbd-orchestrator/phases/phase-4-multilang-wasm-toolchain/plan.md`](../../.kbd-orchestrator/phases/phase-4-multilang-wasm-toolchain/plan.md).
Per-change records: [`.kbd-orchestrator/changes/C-00{1..9}-*.md`](../../.kbd-orchestrator/changes/).
