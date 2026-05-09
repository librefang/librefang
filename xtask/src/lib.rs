// Empty library target. The real entry point is `src/main.rs`.
//
// This file exists so the unit-fast CI lane (`ci.yml: Test / Unit
// (lib+bin)`) does not hard-fail when xtask is among the affected
// crates. The selective branch of that lane builds a per-crate
// argument list and runs:
//
//     cargo nextest run -p xtask … --lib --bins --no-tests=pass
//
// Without a lib target, cargo errors out at target-resolution with
// `error: no library targets found in package 'xtask'` *before*
// nextest sees it, so `--no-tests=pass` (which only forgives
// "zero tests collected", not "zero targets matched") cannot
// recover. The empty lib stub gives cargo a target to bind `--lib`
// to; nextest then collects zero tests from it and exits success.
