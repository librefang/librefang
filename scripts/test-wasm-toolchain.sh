#!/usr/bin/env bash
#
# Phase-4 regression guard for the polyglot WASM toolchain.
#
# Runs inside the librefang-rust-dev image and proves each language's
# WASM-compile path still produces a runnable artefact:
#
#   Rust         cargo +nightly build --target wasm32-wasip2 → .wasm
#   Python       componentize-py componentize hello -o hello.wasm
#   TypeScript   asc hello.ts -o hello.wasm
#   JavaScript   javy build hello.js -o hello.wasm
#   Go           tinygo build -target=wasip2 -o hello.wasm hello.go
#   C            wasi-clang hello.c -o hello.wasm
#
# For each, executes the resulting module under `wasmtime run --wasi`
# and asserts the expected stdout substring.
#
# Designed for CI: a single failed language fails the whole script with
# a clear marker, but the remaining languages still run so the maintainer
# sees the full picture per build.
#
# Usage (inside the dev image, with the librefang repo bind-mounted):
#   /workspace/scripts/test-wasm-toolchain.sh
#
# Usage (from the host, via docker run):
#   docker run --rm -v "$PWD":/workspace -w /workspace \
#       librefang-rust-dev:latest \
#       /workspace/scripts/test-wasm-toolchain.sh

set -uo pipefail

# Aggregate failures rather than bailing on the first one — CI signal
# improves when all six language results are visible per run.
declare -A results=()
fail_count=0

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

note() { printf '\n==> %s\n' "$*"; }
pass() { results["$1"]="PASS ($2)"; }
fail() { results["$1"]="FAIL ($2)"; fail_count=$((fail_count + 1)); }

# ----- Rust ------------------------------------------------------------------
test_rust() {
    local d="$work/rust"
    mkdir -p "$d/src"
    cat > "$d/Cargo.toml" <<EOF
[package]
name = "rust-wasm-smoke"
version = "0.0.1"
edition = "2021"

[[bin]]
name = "rust-wasm-smoke"
path = "src/main.rs"
EOF
    cat > "$d/src/main.rs" <<'EOF'
fn main() { println!("hello-rust-wasm"); }
EOF
    (cd "$d" && cargo +nightly build --release --target wasm32-wasip2 --quiet) \
        || { fail rust "cargo build"; return; }
    local out="$d/target/wasm32-wasip2/release/rust-wasm-smoke.wasm"
    [[ -s "$out" ]] || { fail rust "no output"; return; }
    wasmtime run --wasi cli "$out" 2>&1 | grep -q hello-rust-wasm \
        || { fail rust "stdout mismatch"; return; }
    pass rust "$(rustc +nightly --version | awk '{print $1, $2}')"
}

# ----- Python ----------------------------------------------------------------
test_python() {
    local d="$work/python"
    mkdir -p "$d"
    # componentize-py expects a class implementing the world's exports.
    # The class name is `WitWorld` regardless of the wit world name —
    # the generated bindings look up this exact symbol.
    cat > "$d/hello.py" <<'EOF'
class WitWorld:
    def run(self) -> None:
        print("hello-python-wasm")
EOF
    cat > "$d/hello.wit" <<'EOF'
package local:hello;
world hello { export run: func(); }
EOF
    # `-d <wit-dir>` is required; `-w <world>` selects the world inside it.
    (cd "$d" && componentize-py -d . -w hello componentize hello -o hello.wasm) \
        || { fail python "componentize-py"; return; }
    [[ -s "$d/hello.wasm" ]] || { fail python "no output"; return; }
    # Componentize-py components target a custom world (not wasi:cli/run),
    # so `wasmtime run` can't auto-invoke them — calling the export from a
    # host requires generated bindings, which is well beyond a smoke test.
    # Verify the component is structurally valid instead.
    wasm-tools validate "$d/hello.wasm" 2>/dev/null \
        || { fail python "validate"; return; }
    wasm-tools component wit "$d/hello.wasm" 2>/dev/null | grep -q 'export run' \
        || { fail python "missing export"; return; }
    pass python "$(python3.13 --version | awk '{print $2}')"
}

# ----- TypeScript (AssemblyScript) -------------------------------------------
test_typescript() {
    local d="$work/typescript"
    mkdir -p "$d"
    # Direct WASI fd_write at known linear-memory offsets — avoids both
    # AssemblyScript's stdlib (and its abort import) AND any node module
    # resolution. The string is written as ASCII bytes at offset 100;
    # the iovec lives at offset 0; nwritten lands at offset 8.
    cat > "$d/hello.ts" <<'EOF'
@external("wasi_snapshot_preview1", "fd_write")
declare function fd_write(fd: i32, iovs: i32, iovs_len: i32, nwritten: i32): i32;

export function _start(): void {
    const msg = "hello-typescript-wasm\n";
    const buf: i32 = 100;
    for (let i: i32 = 0; i < msg.length; i++) {
        store<u8>(buf + i, msg.charCodeAt(i));
    }
    store<i32>(0, buf);
    store<i32>(4, msg.length);
    fd_write(1, 0, 1, 8);
}
EOF
    # `--use abort=` stubs out the abort import wasmtime won't provide;
    # `--runtime stub` strips the GC runtime (this program allocates nothing).
    (cd "$d" && asc hello.ts -O --runtime stub \
        --use abort= --outFile hello.wasm) \
        || { fail typescript "asc build"; return; }
    [[ -s "$d/hello.wasm" ]] || { fail typescript "no output"; return; }
    wasmtime run "$d/hello.wasm" 2>&1 | grep -q hello-typescript-wasm \
        || { fail typescript "stdout mismatch"; return; }
    pass typescript "AssemblyScript $(asc --version)"
}

# ----- JavaScript (componentize-js) ------------------------------------------
test_javascript() {
    local d="$work/javascript"
    mkdir -p "$d"
    cat > "$d/hello.js" <<'EOF'
export function run() {
    console.log("hello-javascript-wasm");
}
EOF
    cat > "$d/hello.wit" <<'EOF'
package local:hello;
world hello { export run: func(); }
EOF
    # componentize-js produces a WASI 0.2 Component; like componentize-py
    # the result isn't auto-invocable by `wasmtime run` — verify structure
    # instead. The npm package ships a `componentize.js` CLI under jco.
    (cd "$d" && jco componentize hello.js --wit hello.wit -o hello.wasm 2>/dev/null) \
        || { fail javascript "jco componentize"; return; }
    [[ -s "$d/hello.wasm" ]] || { fail javascript "no output"; return; }
    wasm-tools validate "$d/hello.wasm" 2>/dev/null \
        || { fail javascript "validate"; return; }
    wasm-tools component wit "$d/hello.wasm" 2>/dev/null | grep -q 'export run' \
        || { fail javascript "missing export"; return; }
    pass javascript "componentize-js (via jco)"
}

# ----- Go (TinyGo) -----------------------------------------------------------
test_go() {
    local d="$work/go"
    mkdir -p "$d"
    cat > "$d/hello.go" <<'EOF'
package main
func main() { println("hello-go-wasm") }
EOF
    (cd "$d" && tinygo build -target=wasip2 -o hello.wasm hello.go 2>/dev/null) \
        || { fail go "tinygo build"; return; }
    [[ -s "$d/hello.wasm" ]] || { fail go "no output"; return; }
    wasmtime run "$d/hello.wasm" 2>&1 | grep -q hello-go-wasm \
        || { fail go "stdout mismatch"; return; }
    pass go "tinygo $(tinygo version | awk '{print $3}')"
}

# ----- C (wasi-sdk) ----------------------------------------------------------
test_c() {
    local d="$work/c"
    mkdir -p "$d"
    cat > "$d/hello.c" <<'EOF'
#include <stdio.h>
int main(void) { puts("hello-c-wasm"); return 0; }
EOF
    # Use wasi-sdk's clang directly with its sysroot. WASI_SDK_PATH is
    # exported by the dev image (Dockerfile.rust-dev) at /opt/wasi-sdk.
    local clang="${WASI_SDK_PATH:-/opt/wasi-sdk}/bin/clang"
    local sysroot="${WASI_SDK_PATH:-/opt/wasi-sdk}/share/wasi-sysroot"
    (cd "$d" && "$clang" --target=wasm32-wasi --sysroot="$sysroot" \
        hello.c -o hello.wasm 2>/dev/null) \
        || { fail c "wasi-sdk clang"; return; }
    [[ -s "$d/hello.wasm" ]] || { fail c "no output"; return; }
    wasmtime run --wasi cli "$d/hello.wasm" 2>&1 | grep -q hello-c-wasm \
        || { fail c "stdout mismatch"; return; }
    pass c "wasi-sdk $("$clang" --version | head -1 | awk '{print $3}')"
}

note "Rust → wasm32-wasip2"
test_rust
note "Python → WASI 0.2 Component"
test_python
note "TypeScript → WASM (AssemblyScript)"
test_typescript
note "JavaScript → WASM (Javy)"
test_javascript
note "Go → wasip2 (TinyGo)"
test_go
note "C → WASI (wasi-sdk)"
test_c

note "Results"
for lang in rust python typescript javascript go c; do
    printf '  %-12s %s\n' "$lang" "${results[$lang]:-MISSING}"
done

if (( fail_count > 0 )); then
    printf '\n%s\n' "FAILED: $fail_count language(s) regressed"
    exit 1
fi

printf '\n%s\n' "All language WASM toolchains green."
