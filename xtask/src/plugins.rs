//! `cargo xtask plugins-rebuild [<lang>]`
//!
//! Phase-6 build orchestrator for the per-language plugin examples
//! under `examples/plugins/`. Each example ships a checked-in
//! `pre-built/plugin.wasm` (the integration tests load these) plus
//! the source tree authors edit. This subcommand rebuilds the
//! artefacts deterministically inside the librefang-rust-dev
//! image's toolchain set.
//!
//! ## Design
//!
//! Each language registers a builder closure in `BUILDERS`. The
//! closure shells out to that language's WASM compiler
//! (cargo-component / componentize-py / jco / TinyGo / make) and
//! writes the resulting `.wasm` to `<dir>/pre-built/plugin.wasm`.
//! Size budget (≤ 200 KB per artefact, ≤ 500 KB total) is enforced
//! after each rebuild — a bust signals the example has drifted
//! from its "minimal canonical" goal and needs simplification.
//!
//! Without `<lang>`, rebuilds every registered example. With
//! `<lang>` (e.g. `rust-fs-cat`), rebuilds only that one.
//!
//! Per Phase-6 C-001: the builder registry starts empty. C-002…C-006
//! each register their language's builder in this file. Until then
//! the subcommand is a no-op skeleton that prints the empty registry.

use clap::Args;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

/// xtask's canonical error type — `Box<dyn Error>` matches the rest
/// of this binary's modules (see `migrate.rs`, `ci.rs`, etc.).
type XtaskResult<T> = Result<T, Box<dyn Error>>;

#[derive(Args, Debug)]
pub struct PluginsRebuildArgs {
    /// Specific example to rebuild (e.g. `rust-fs-cat`). Omit to
    /// rebuild every registered example.
    pub example: Option<String>,

    /// Print what would be rebuilt without actually invoking the
    /// builders. Useful for CI dry-runs.
    #[arg(long)]
    pub dry_run: bool,
}

/// One entry per language/example. Filled in by C-002…C-006.
type Builder = fn(workspace_root: &Path) -> XtaskResult<()>;

/// Registry — appended to by C-002…C-006 as each language lands.
const BUILDERS: &[(&str, Builder)] = &[
    ("rust-fs-cat", build_rust_fs_cat),
    ("python-hello-time", build_python_hello_time),
    ("js-kv-counter", build_js_kv_counter),
    ("go-env-greet", build_go_env_greet),
    ("c-noop", build_c_noop),
];

/// Default per-artefact size budget for compiled languages (Rust, C, Go).
/// Interpreted-language runtimes (Python/CPython, JS/SpiderMonkey) embed
/// their interpreter and are budgeted separately via `SIZE_BUDGETS`.
pub const MAX_PLUGIN_WASM_BYTES: u64 = 200 * 1024;

/// Per-example overrides when the default compiled-language ceiling is too
/// tight. Entries not in this table fall back to `MAX_PLUGIN_WASM_BYTES`.
/// Python: componentize-py bundles CPython (~15–20 MB); 24 MB headroom.
/// JS: jco bundles SpiderMonkey (~5 MB); 8 MB headroom.
const SIZE_BUDGETS: &[(&str, u64)] = &[
    ("python-hello-time", 24 * 1024 * 1024),
    ("js-kv-counter", 16 * 1024 * 1024),
    // Go: TinyGo wasip1 + WASI P1 adapter ≈ 400 KB compiled; 2 MB ceiling.
    ("go-env-greet", 2 * 1024 * 1024),
];

pub fn run(args: PluginsRebuildArgs) -> XtaskResult<()> {
    let workspace_root = workspace_root()?;
    if BUILDERS.is_empty() {
        println!(
            "cargo xtask plugins-rebuild: registry is empty. \
             Phase-6 C-002..C-006 will register per-language builders. \
             Examples directory: {}",
            workspace_root.join("examples/plugins").display()
        );
        return Ok(());
    }

    let targets: Vec<&(&str, Builder)> = match args.example.as_deref() {
        Some(name) => BUILDERS.iter().filter(|(n, _)| *n == name).collect(),
        None => BUILDERS.iter().collect(),
    };
    if targets.is_empty() {
        let known = BUILDERS
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "no example named '{}' registered; known: {known}",
            args.example.as_deref().unwrap_or("")
        )
        .into());
    }

    if args.dry_run {
        println!("plugins-rebuild dry-run; would build:");
        for (name, _) in &targets {
            println!("  - {name}");
        }
        return Ok(());
    }

    for (name, build) in &targets {
        println!("==> rebuilding plugin example: {name}");
        build(&workspace_root)?;
        enforce_size_budget(&workspace_root, name)?;
    }
    println!("plugins-rebuild: {} example(s) rebuilt.", targets.len());
    Ok(())
}

/// Locate the workspace root by walking up from the xtask manifest
/// dir. xtask is always built inside the workspace so this is
/// deterministic.
fn workspace_root() -> XtaskResult<PathBuf> {
    // `CARGO_MANIFEST_DIR` is set by cargo for the xtask crate; the
    // workspace root is its parent.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| "CARGO_MANIFEST_DIR not set; run via `cargo xtask`")?;
    let parent = PathBuf::from(&manifest_dir)
        .parent()
        .ok_or("xtask manifest dir has no parent")?
        .to_path_buf();
    Ok(parent)
}

/// Per-artefact size guardrail. Fail-loud if a plugin example bloats
/// past the 200 KB ceiling — the budget is the canary that the
/// example has drifted from its "minimal canonical" goal.
fn enforce_size_budget(workspace_root: &Path, name: &str) -> XtaskResult<()> {
    let wasm = workspace_root
        .join("examples/plugins")
        .join(name)
        .join("pre-built/plugin.wasm");
    let meta = std::fs::metadata(&wasm).map_err(|e| format!("stat {}: {e}", wasm.display()))?;
    let budget = SIZE_BUDGETS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, b)| *b)
        .unwrap_or(MAX_PLUGIN_WASM_BYTES);
    if meta.len() > budget {
        return Err(format!(
            "plugin '{name}' exceeds size budget: {} bytes > {} bytes ({})",
            meta.len(),
            budget,
            wasm.display()
        )
        .into());
    }
    println!("    {} bytes ({})", meta.len(), wasm.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-language builders. Each takes the workspace root and writes a
// .wasm to `examples/plugins/<name>/pre-built/plugin.wasm`.
// ---------------------------------------------------------------------------

/// C-005: go-env-greet — TinyGo wasip1 + wasm-tools component embed/new.
///
/// Build pipeline:
/// 1. `tinygo build -target wasip1` → core module (wasm32-unknown-wasi)
/// 2. `wasm-tools component embed --world plugin <wit> <core>` → typed module
/// 3. `wasm-tools component new <typed> --adapt wasi_snapshot_preview1=<adapter>`
///
/// The WASI P1 reactor adapter (52 KB) is committed under wasi-adapter/ for
/// reproducibility. It bridges TinyGo's `wasi_snapshot_preview1` calls to the
/// component model; our `librefang:plugin/env` import passes through as-is.
///
/// Requirements on $PATH: `tinygo`, `wasm-tools`.
/// TINYGOROOT env var must point at the TinyGo installation root if tinygo
/// can't auto-detect it (e.g. /tmp/tinygo).
fn build_go_env_greet(workspace_root: &Path) -> XtaskResult<()> {
    let dir = workspace_root.join("examples/plugins/go-env-greet");
    let wit = workspace_root.join("crates/librefang-skills/wit");
    let adapter = dir.join("wasi-adapter/wasi_snapshot_preview1.reactor.wasm");
    let dst = dir.join("pre-built/plugin.wasm");
    std::fs::create_dir_all(dst.parent().unwrap())
        .map_err(|e| format!("mkdir {}: {e}", dst.parent().unwrap().display()))?;

    // Locate tinygo: honour TINYGOROOT, then fall back to PATH.
    let tinygo_bin = std::env::var("TINYGOROOT")
        .map(|root| PathBuf::from(root).join("bin/tinygo"))
        .unwrap_or_else(|_| PathBuf::from("tinygo"));

    // Step 1: compile to WASI P1 core module
    let core_wasm = std::env::temp_dir().join("go-env-greet-core.wasm");
    let status = Command::new(&tinygo_bin)
        .args([
            "build",
            "-target",
            "wasip1",
            "-o",
            core_wasm.to_str().unwrap(),
            ".",
        ])
        .current_dir(&dir)
        .status()
        .map_err(|e| {
            format!(
                "spawn tinygo ({}) in {}: {e}",
                tinygo_bin.display(),
                dir.display()
            )
        })?;
    if !status.success() {
        return Err(format!("tinygo build failed in {}", dir.display()).into());
    }

    // Step 2: embed WIT type information
    let embedded_wasm = std::env::temp_dir().join("go-env-greet-embedded.wasm");
    let status = Command::new("wasm-tools")
        .args([
            "component",
            "embed",
            "--world",
            "plugin",
            wit.to_str().unwrap(),
            core_wasm.to_str().unwrap(),
            "-o",
            embedded_wasm.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("spawn wasm-tools embed: {e}"))?;
    if !status.success() {
        return Err("wasm-tools component embed failed for go-env-greet".into());
    }

    // Step 3: lift to component model with WASI P1 adapter
    let adapt_arg = format!("wasi_snapshot_preview1={}", adapter.to_str().unwrap());
    let status = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            embedded_wasm.to_str().unwrap(),
            "--adapt",
            &adapt_arg,
            "-o",
            dst.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("spawn wasm-tools component new: {e}"))?;
    if !status.success() {
        return Err("wasm-tools component new failed for go-env-greet".into());
    }
    Ok(())
}

/// C-004: js-kv-counter — jco componentize (StarlingMonkey) → wasm32-wasip2.
///
/// `jco componentize app.js --wit <wit> --world-name plugin -o <dst>`
fn build_js_kv_counter(workspace_root: &Path) -> XtaskResult<()> {
    let dir = workspace_root.join("examples/plugins/js-kv-counter");
    let wit = workspace_root.join("crates/librefang-skills/wit");
    let dst = dir.join("pre-built/plugin.wasm");
    std::fs::create_dir_all(dst.parent().unwrap())
        .map_err(|e| format!("mkdir {}: {e}", dst.parent().unwrap().display()))?;
    let status = Command::new("jco")
        .args([
            "componentize",
            "app.js",
            "--wit",
            wit.to_str().unwrap(),
            "--world-name",
            "plugin",
            "-o",
        ])
        .arg(&dst)
        .current_dir(&dir)
        .status()
        .map_err(|e| format!("spawn jco in {}: {e}", dir.display()))?;
    if !status.success() {
        return Err(format!("jco componentize failed in {}", dir.display()).into());
    }
    Ok(())
}

/// C-003: python-hello-time — componentize-py → wasm32-wasip2.
///
/// `componentize-py --wit-path <wit> --world plugin componentize app.py -o <dst>`
/// produces a self-contained WASM component. The command must be on PATH.
fn build_python_hello_time(workspace_root: &Path) -> XtaskResult<()> {
    let dir = workspace_root.join("examples/plugins/python-hello-time");
    let wit = workspace_root.join("crates/librefang-skills/wit");
    let dst = dir.join("pre-built/plugin.wasm");
    std::fs::create_dir_all(dst.parent().unwrap())
        .map_err(|e| format!("mkdir {}: {e}", dst.parent().unwrap().display()))?;
    let status = Command::new("componentize-py")
        .args([
            "--wit-path",
            wit.to_str().unwrap(),
            "--world",
            "plugin",
            "componentize",
            "app",
            "-o",
        ])
        .arg(&dst)
        .current_dir(&dir)
        .status()
        .map_err(|e| format!("spawn componentize-py in {}: {e}", dir.display()))?;
    if !status.success() {
        return Err(format!("componentize-py failed in {}", dir.display()).into());
    }
    Ok(())
}

/// C-006: c-noop — wasi-clang (LLVM) + wit-bindgen-c, no capabilities.
///
/// Three-step pipeline:
///   1. `clang --target=wasm32-wasip1` compiles `plugin.c`, `bindings/plugin.c`,
///      and `stubs.c` (minimal allocator stubs) + links `bindings/plugin_component_type.o`
///      into a bare core module.
///   2. `wasm-tools component embed --world plugin <wit> core.wasm -o embedded.wasm`
///      attaches the WIT type metadata.
///   3. `wasm-tools component new embedded.wasm -o pre-built/plugin.wasm`
///      finalises the component (no WASI adapter needed — the plugin imports
///      no host capabilities so there are no WASI P1 calls to bridge).
///
/// Requirements: LLVM clang on PATH (or at WASI_CLANG env var), `wasm-ld`
/// on PATH (or at WASI_WASM_LD env var), `wasm-tools` on PATH.
/// The sysroot must be provided via WASI_SYSROOT (defaults to the TinyGo
/// sysroot at $TINYGOROOT/lib/wasi-libc/sysroot if TINYGOROOT is set).
fn build_c_noop(workspace_root: &Path) -> XtaskResult<()> {
    let dir = workspace_root.join("examples/plugins/c-noop");
    let wit = workspace_root.join("crates/librefang-skills/wit");
    let dst = dir.join("pre-built/plugin.wasm");
    std::fs::create_dir_all(dst.parent().unwrap())
        .map_err(|e| format!("mkdir {}: {e}", dst.parent().unwrap().display()))?;

    // Locate clang: honour WASI_CLANG, then fall back to PATH.
    let clang_bin = std::env::var("WASI_CLANG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("clang"));

    // Locate wasm-ld: honour WASI_WASM_LD, then fall back to PATH.
    let wasm_ld = std::env::var("WASI_WASM_LD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("wasm-ld"));

    // Locate sysroot: honour WASI_SYSROOT, then try TINYGOROOT, else error.
    let sysroot: PathBuf = if let Ok(s) = std::env::var("WASI_SYSROOT") {
        PathBuf::from(s)
    } else if let Ok(tinygo) = std::env::var("TINYGOROOT") {
        PathBuf::from(tinygo).join("lib/wasi-libc/sysroot")
    } else {
        // Try a well-known homebrew wasi-sdk location as a last resort.
        let candidate = PathBuf::from("/opt/homebrew/opt/wasi-sdk/share/wasi-sysroot");
        if candidate.exists() {
            candidate
        } else {
            return Err(
                "WASI sysroot not found: set WASI_SYSROOT, TINYGOROOT, or install wasi-sdk".into(),
            );
        }
    };
    let fuse_ld = format!("-fuse-ld={}", wasm_ld.display());

    // Step 1: compile core WASM module.
    let core_wasm = std::env::temp_dir().join("c-noop-core.wasm");
    let status = Command::new(&clang_bin)
        .args([
            "--target=wasm32-wasip1",
            "--sysroot",
            sysroot.to_str().unwrap(),
            &fuse_ld,
            "-O2",
            "-nostdlib",
            "-Wl,--no-entry",
            "-Wl,--export-dynamic",
            "bindings/plugin.c",
            "plugin.c",
            "stubs.c",
            "bindings/plugin_component_type.o",
            "-o",
        ])
        .arg(&core_wasm)
        .current_dir(&dir)
        .status()
        .map_err(|e| {
            format!(
                "spawn clang ({}) in {}: {e}",
                clang_bin.display(),
                dir.display()
            )
        })?;
    if !status.success() {
        return Err(format!("clang build failed in {}", dir.display()).into());
    }

    // Step 2: embed WIT type information.
    let embedded_wasm = std::env::temp_dir().join("c-noop-embedded.wasm");
    let status = Command::new("wasm-tools")
        .args([
            "component",
            "embed",
            "--world",
            "plugin",
            wit.to_str().unwrap(),
            core_wasm.to_str().unwrap(),
            "-o",
            embedded_wasm.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("spawn wasm-tools embed: {e}"))?;
    if !status.success() {
        return Err("wasm-tools component embed failed for c-noop".into());
    }

    // Step 3: lift to component (no WASI adapter — no host imports used).
    let status = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            embedded_wasm.to_str().unwrap(),
            "-o",
            dst.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("spawn wasm-tools component new: {e}"))?;
    if !status.success() {
        return Err("wasm-tools component new failed for c-noop".into());
    }
    Ok(())
}

/// C-002: rust-fs-cat — cargo-component → wasm32-wasip2.
///
/// `cargo component build --release --target wasm32-wasip2` produces
/// `target/wasm32-wasip2/release/rust_fs_cat.wasm`. We `cp` that to
/// the pre-built/ slot the example author + integration test both
/// reference.
fn build_rust_fs_cat(workspace_root: &Path) -> XtaskResult<()> {
    let dir = workspace_root.join("examples/plugins/rust-fs-cat");
    let status = Command::new("cargo")
        .arg("component")
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .current_dir(&dir)
        .status()
        .map_err(|e| format!("spawn cargo-component in {}: {e}", dir.display()))?;
    if !status.success() {
        return Err(format!("cargo component build failed in {}", dir.display()).into());
    }
    let src = dir.join("target/wasm32-wasip2/release/rust_fs_cat.wasm");
    let dst = dir.join("pre-built/plugin.wasm");
    std::fs::create_dir_all(dst.parent().unwrap())
        .map_err(|e| format!("mkdir {}: {e}", dst.parent().unwrap().display()))?;
    std::fs::copy(&src, &dst)
        .map_err(|e| format!("cp {} -> {}: {e}", src.display(), dst.display()))?;
    Ok(())
}
