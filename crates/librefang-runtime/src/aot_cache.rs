//! AOT (`.cwasm`) cache for Component Model plugins (Phase-5 C-006).
//!
//! `wasmtime::component::Component::deserialize_file` loads a
//! pre-compiled `.cwasm` artefact in milliseconds — orders of
//! magnitude faster than `Component::from_binary` which JITs every
//! call. The trade-off: `.cwasm` files are wasmtime-version-locked
//! (the on-disk format isn't a public ABI) so a wasmtime bump
//! invalidates every cached artefact.
//!
//! The cache file naming embeds three keys:
//!
//!   `<cache_dir>/<sha256(wasm)>.<wasmtime_version>.cwasm`
//!
//! - SHA of the source `.wasm` guards against source drift.
//! - wasmtime version guards against on-disk format drift.
//! - `cache_dir` is caller-supplied; typically per-skill at
//!   `~/.librefang/skills/<id>/`, which makes `librefang skill
//!   uninstall <id>` a clean `rm -rf` operation (D4 from the
//!   phase-5 plan).
//!
//! ## Compile modes
//!
//! `CompileMode::Auto` (default): try AOT load → fall back to JIT
//! compile → opportunistically write the cache after a successful
//! JIT compile so the next run is AOT.
//!
//! `CompileMode::Aot`: hard-require an AOT cache hit. Useful in CI /
//! production where you precompiled at install time and a JIT fall-
//! back would mask a problem.
//!
//! `CompileMode::Jit`: skip the cache entirely. Useful in tests or
//! when iterating on a plugin during development.
//!
//! ## Safety: `Component::deserialize_file` is `unsafe` upstream
//!
//! wasmtime's deserialize API is marked unsafe because it trusts the
//! `.cwasm` bytes to be both well-formed AND produced by the SAME
//! wasmtime binary. We mitigate by:
//!   1. Filename includes `<wasmtime_version>` — a binary built with
//!      a different wasmtime version would write to a different
//!      filename, so cross-version reuse is impossible.
//!   2. We never read `.cwasm` files we didn't produce (cache_dir is
//!      under `~/.librefang/`, root-owned in production).
//!   3. If deserialize fails for any reason (corruption, partial
//!      write, version mismatch despite the filename), we fall back
//!      to JIT in `Auto` mode and log a WARN.

use crate::sandbox::SandboxError;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tracing::warn;
use wasmtime::component::Component;
use wasmtime::Engine;

/// Wasmtime version pin for cache filenames. Bumping wasmtime in
/// `Cargo.toml` MUST also update this constant — otherwise old
/// `.cwasm` files would silently be reused under the new wasmtime
/// binary, triggering UB on deserialize.
///
/// Kept as a const (rather than `env!("CARGO_PKG_VERSION")` from
/// wasmtime) because (a) wasmtime doesn't export its own version
/// string at runtime and (b) we want bumping wasmtime to require an
/// explicit cache-version step the reviewer sees.
pub const WASMTIME_CACHE_VERSION: &str = "wasmtime-44";

/// Compile-mode knob. Default `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompileMode {
    /// JIT on first run; subsequent runs use AOT if a cache hit
    /// happens. Cache misses or deserialize failures silently fall
    /// back to JIT.
    #[default]
    Auto,
    /// AOT only — error if no cache hit. Use after a precompile
    /// step (e.g. `librefang skill install` warms the cache).
    Aot,
    /// JIT only — skip the cache entirely. Useful for tests and
    /// rapid iteration on a plugin during development.
    Jit,
}

/// Compute the cache path for a given wasm binary inside `cache_dir`.
///
/// Filename: `<sha256-of-wasm>.<wasmtime_version>.cwasm` so version
/// mismatches and source mismatches both cleanly miss the cache.
pub fn cache_path(cache_dir: &Path, wasm_bytes: &[u8]) -> PathBuf {
    let sha = sha256_hex(wasm_bytes);
    cache_dir.join(format!("{sha}.{WASMTIME_CACHE_VERSION}.cwasm"))
}

/// Load `wasm_bytes` as a `Component`, preferring AOT when the cache
/// has a fresh artefact. Behavior per `CompileMode` is documented at
/// the enum.
///
/// On `Auto` after a JIT compile this opportunistically writes the
/// cache. Write failures are warn-logged and ignored — the
/// just-compiled Component is returned successfully either way.
pub fn load_or_compile(
    engine: &Engine,
    wasm_bytes: &[u8],
    cache_dir: &Path,
    mode: CompileMode,
) -> Result<Component, SandboxError> {
    let cpath = cache_path(cache_dir, wasm_bytes);

    // --- Cache hit branch (Auto or Aot) -------------------------
    if matches!(mode, CompileMode::Auto | CompileMode::Aot) && cpath.exists() {
        // SAFETY: cpath is under `cache_dir` which is caller-trusted;
        // wasmtime version is keyed into the filename so cross-binary
        // reuse is impossible. See module-level Safety section.
        match unsafe { Component::deserialize_file(engine, &cpath) } {
            Ok(c) => return Ok(c),
            Err(e) => {
                if matches!(mode, CompileMode::Aot) {
                    return Err(SandboxError::Compilation(format!(
                        "AOT cache deserialize failed (mode=aot, path={}): {e}",
                        cpath.display()
                    )));
                }
                warn!(
                    cache_path = %cpath.display(),
                    error = %e,
                    "AOT cache deserialize failed; falling back to JIT"
                );
                // Best-effort delete the corrupt artefact so the next
                // run doesn't repeat the same failure.
                let _ = std::fs::remove_file(&cpath);
            }
        }
    }

    // --- Aot mode without a cache file = hard error -------------
    if matches!(mode, CompileMode::Aot) {
        return Err(SandboxError::Compilation(format!(
            "AOT cache miss in mode=aot; expected pre-warmed cache at {}",
            cpath.display()
        )));
    }

    // --- JIT compile branch -------------------------------------
    let component = Component::from_binary(engine, wasm_bytes)
        .map_err(|e| SandboxError::Compilation(e.to_string()))?;

    // --- Opportunistically populate the cache (Auto only) -------
    if matches!(mode, CompileMode::Auto) {
        if let Err(e) = write_cache(engine, wasm_bytes, &cpath) {
            warn!(
                cache_path = %cpath.display(),
                error = %e,
                "AOT cache write failed; next run will JIT again"
            );
        }
    }

    Ok(component)
}

/// Force-precompile and write to cache. Useful at install time:
/// `librefang skill install <id>` calls this so the first execution
/// is already AOT.
pub fn precompile(
    engine: &Engine,
    wasm_bytes: &[u8],
    cache_dir: &Path,
) -> Result<PathBuf, SandboxError> {
    let cpath = cache_path(cache_dir, wasm_bytes);
    write_cache(engine, wasm_bytes, &cpath)?;
    Ok(cpath)
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    // Hex without dashes; 64 chars.
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", byte);
    }
    out
}

fn write_cache(engine: &Engine, wasm_bytes: &[u8], cpath: &Path) -> Result<(), SandboxError> {
    if let Some(parent) = cpath.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            SandboxError::Compilation(format!("create cache dir {}: {e}", parent.display()))
        })?;
    }
    // engine.precompile_component returns serialized bytes suitable
    // for Component::deserialize_file. Atomic write via tempfile +
    // rename so an interrupted precompile doesn't leave a half-
    // written artefact the next deserialize would happily load.
    let bytes = engine
        .precompile_component(wasm_bytes)
        .map_err(|e| SandboxError::Compilation(format!("precompile_component: {e}")))?;
    let tmp = cpath.with_extension("cwasm.tmp");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| SandboxError::Compilation(format!("write tmp {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, cpath).map_err(|e| {
        SandboxError::Compilation(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            cpath.display()
        ))
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::engine_config;
    use tempfile::tempdir;

    fn engine() -> Engine {
        Engine::new(&engine_config()).unwrap()
    }

    /// Minimal valid Component bytes — `(component)` is the empty
    /// component in WAT, produced by wat2wasm in the dev image but
    /// also constructible by hand. We use the wat crate to keep the
    /// test self-contained.
    fn empty_component_bytes() -> Vec<u8> {
        wat::parse_str("(component)").unwrap()
    }

    #[test]
    fn cache_path_includes_sha_and_version() {
        let dir = std::path::Path::new("/tmp/test-cache");
        let p = cache_path(dir, b"hello");
        // sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("2cf24dba5fb0a30e"), "got: {name}");
        assert!(
            name.ends_with(&format!(".{WASMTIME_CACHE_VERSION}.cwasm")),
            "got: {name}"
        );
    }

    #[test]
    fn precompile_creates_file() {
        let dir = tempdir().unwrap();
        let bytes = empty_component_bytes();
        let path = precompile(&engine(), &bytes, dir.path()).unwrap();
        assert!(path.exists());
        assert!(path.metadata().unwrap().len() > 0);
    }

    #[test]
    fn auto_cache_miss_then_hit() {
        let dir = tempdir().unwrap();
        let bytes = empty_component_bytes();
        let eng = engine();

        // First call: cache miss, JIT, opportunistic write.
        let _c1 = load_or_compile(&eng, &bytes, dir.path(), CompileMode::Auto).unwrap();
        let cpath = cache_path(dir.path(), &bytes);
        assert!(cpath.exists(), "Auto mode should populate the cache");

        // Second call: cache hit via deserialize_file.
        let _c2 = load_or_compile(&eng, &bytes, dir.path(), CompileMode::Auto).unwrap();
    }

    #[test]
    fn aot_mode_missing_cache_errors() {
        let dir = tempdir().unwrap();
        let bytes = empty_component_bytes();
        // `unwrap_err` requires Debug on the success type, which
        // wasmtime::component::Component doesn't impl. Match instead.
        match load_or_compile(&engine(), &bytes, dir.path(), CompileMode::Aot) {
            Ok(_) => panic!("expected AOT-mode miss to error, got Ok"),
            Err(SandboxError::Compilation(msg)) => {
                assert!(
                    msg.contains("AOT cache miss"),
                    "expected 'AOT cache miss' in error, got: {msg}"
                );
            }
            Err(other) => panic!("expected SandboxError::Compilation, got: {other:?}"),
        }
    }

    #[test]
    fn aot_mode_after_precompile_succeeds() {
        let dir = tempdir().unwrap();
        let bytes = empty_component_bytes();
        let eng = engine();
        precompile(&eng, &bytes, dir.path()).unwrap();
        // Now AOT mode succeeds via cache hit.
        let _c = load_or_compile(&eng, &bytes, dir.path(), CompileMode::Aot).unwrap();
    }

    #[test]
    fn jit_mode_never_writes_cache() {
        let dir = tempdir().unwrap();
        let bytes = empty_component_bytes();
        let _c = load_or_compile(&engine(), &bytes, dir.path(), CompileMode::Jit).unwrap();
        let cpath = cache_path(dir.path(), &bytes);
        assert!(
            !cpath.exists(),
            "Jit mode must not populate the cache; found {}",
            cpath.display()
        );
    }

    #[test]
    fn corrupt_cache_falls_back_in_auto_mode() {
        let dir = tempdir().unwrap();
        let bytes = empty_component_bytes();
        let cpath = cache_path(dir.path(), &bytes);
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(&cpath, b"definitely not a valid cwasm").unwrap();

        // Auto: deserialize fails, fallback to JIT succeeds.
        let _c = load_or_compile(&engine(), &bytes, dir.path(), CompileMode::Auto).unwrap();
        // Corrupt file should have been removed.
        assert!(
            !cpath.exists() || cpath.metadata().unwrap().len() > 32,
            "corrupt cache should be removed (or rewritten with real bytes)"
        );
    }
}
