// Empty library target. The real entry point is `src/main.rs`.
// This file exists so `cargo test -p xtask --lib` does not error in
// CI's unit-fast lane, which selects per-package targets with
// `-p <crate> --lib --bins` and treats "no lib target" as a hard
// failure regardless of whether other matching targets exist.
