# [Low] Dependency hygiene roundup — version sprawl, transitively-old crates, build hygiene

**Severity:** Low
**Category:** Supply chain · Build
**Status:** Merges 5 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description |
|--------|-------------|
| this | `phf_generator 0.8` pins old `rand 0.7.3`, `rand_core 0.5.1` — four `rand` majors coexist in the build |
| proc-macro-error | `proc-macro-error 1.0.4` (RUSTSEC-2024-0370) is pulled in via gtk-rs; already `deny.toml`-ignored; cannot be removed until Tauri upgrades to GTK4 |
| tokio full | Workspace `tokio = ["full"]` is propagated to leaf crates that do not need it (e.g. `librefang-wire`) |
| pnpm audit ignore | `pnpm audit ignore GHSA-rmmr-r34h-pfm5` has no inline rationale; in the future nobody will be able to judge whether the ignore can be lifted |
| build.rs shim | `build.rs` calls `git` / `date` without path anchoring; on a CI runner with a shim, this is arbitrary code execution |
| version sprawl | `Cargo.lock` has 99 crates with multiple versions coexisting (`windows-sys ×5`, `hashbrown ×5`, `base64 ×3`, `nom ×3`) |

## Affected files

- `Cargo.toml:45` (workspace tokio)
- `Cargo.toml:197` (workspace serde_yaml — see also [serde-yaml-unmaintained](serde-yaml-unmaintained.md))
- `Cargo.lock` (multi-version chains)
- `deny.toml:64, 67-80, 117`
- `crates/librefang-api/build.rs:18-46`
- `crates/librefang-cli/build.rs:1-48`
- `crates/librefang-api/dashboard/package.json:13-17`

## Why merged

All six are dependency-maintenance / build-hygiene items; tracking them separately adds no value. Running them as a single quarterly dependency-audit pass is more efficient.

## Combined fix plan

1. **Multi-version convergence (this / version sprawl)**:
   - `cargo tree -d -e features` to surface the top 5 duplicate sources;
   - `phf_generator 0.8` → find consumers (`cargo tree -i phf_shared:0.8.0`), bump upstreams (`selectors` / `cssparser` already support `phf 0.11`);
   - In `deny.toml`, promote `multiple-versions` to `deny` for `windows-sys` / `hashbrown` / `rand` / `base64`, and use `[[bans.skip]]` to explicitly authorize a version.
2. **Workspace tokio convergence (tokio full)**: drop `features = ["full"]` at the workspace level; each crate opts in:
   ```toml
   tokio = { version = "1", default-features = false }
   ```
   Most leaf crates only need `["rt", "macros", "sync"]`; binaries and the kernel use `["full"]`.
3. **Build hygiene (build.rs shim)**:
   - Prefer `GITHUB_SHA` / `CI_COMMIT_SHA`; fall back to `git` only outside CI;
   - Replace `date` with `chrono::Utc::now()` (already a dependency);
   - When calling `git`, resolve it via `which::which("git")` to an absolute path.
4. **Audit annotations (pnpm audit ignore)**: add inline JSON comments to `package.json` (or a README) stating the reason and the unlock condition for each ignore.
5. **GTK chain (proc-macro-error)**: no new action; revisit quarterly and remove the ignore once Tauri migrates to GTK4.

## Tests

- `cargo tree -d` row count ≤ threshold (CI gate).
- `cargo deny check` is green under `multiple-versions = "deny"`.
- `build.rs` does not panic when `git` is absent from `PATH` (env fallback).
