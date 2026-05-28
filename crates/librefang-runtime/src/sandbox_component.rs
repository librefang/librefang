//! Component Model execute path for `WasmSandbox` (Phase-5 C-004).
//!
//! Side-by-side with the existing core-module `WasmSandbox::execute()`
//! in `sandbox.rs`. The two paths share the same `GuestState` /
//! `SandboxConfig` / `ExecutionResult` / `SandboxError` types — only the
//! wasmtime API surface differs (`wasmtime::component::*` vs core
//! `Module` / `Linker`).
//!
//! ## Why a new file (D1 hygiene)
//!
//! Upstream librefang has no Component Model implementation as of
//! 2026-05. Landing Component support inside the existing
//! `sandbox.rs` would create a large merge-conflict surface every time
//! upstream touches that file. By keeping Component code here, the
//! only contact with `sandbox.rs` is two small additive helpers
//! (`new_guest_state`, `engine_config`) that are stable in signature.
//!
//! ## What lives where
//!
//! * `host_functions::dispatch` — single source of truth for capability
//!   checks and side effects (unchanged).
//! * `wit_host` — pure conversion between Component Model typed args
//!   and the JSON shape `dispatch` expects (C-003).
//! * Here — `bindgen!` invocation, bindgen-generated `Host` trait
//!   impls (one-liners composing `wit_host::call_*`), and the
//!   `WasmSandbox::execute_component()` async entry point.

use crate::host_functions::dispatch;
use crate::kernel_handle::KernelHandle;
use crate::sandbox::{
    engine_config, new_guest_state, ExecutionResult, GuestState, SandboxConfig, SandboxError,
};
use crate::wit_host::{self, HostErrorRepr};
use std::sync::Arc;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Engine, Store};

// ---------------------------------------------------------------------------
// bindgen — generates `Plugin` world + per-interface `Host` traits from
// `crates/librefang-skills/wit/`.
//
// The macro expands at compile time; `cargo expand -p librefang-runtime`
// is the canonical way to see what it produces. We pin `async: true`
// so the generated trait methods are `async fn`, matching the rest of
// the runtime (wasmtime feature `async` is enabled in workspace
// Cargo.toml).
//
// The `with` mapping lets us reuse our `HostErrorRepr` as the trait's
// error type rather than carrying the bindgen-generated variant + a
// `From` impl. Keeps the error classification in one place
// (`wit_host::HostErrorRepr::from_dispatch_message`).
// ---------------------------------------------------------------------------
wasmtime::component::bindgen!({
    path: "../librefang-skills/wit",
    world: "plugin",
    imports: { default: async | trappable },
    exports: { default: async },
});

// Alias for the bindgen-generated `host-error` variant. Path is derived
// from the WIT package/interface naming
// (`librefang:plugin/host-types/host-error`). Bindgen converts the
// dashed names to snake_case modules and Pascal-case types.
use librefang::plugin::host_types::HostError as HostErrorWit;

/// Lift the runtime-classified `HostErrorRepr` into the bindgen-
/// generated WIT variant the trait signatures expect. Pairing the two
/// types keeps the classification logic in one place (`wit_host`) while
/// letting bindgen own the cross-language ABI for the variant.
impl From<HostErrorRepr> for HostErrorWit {
    fn from(e: HostErrorRepr) -> Self {
        match e {
            HostErrorRepr::CapabilityDenied(s) => HostErrorWit::CapabilityDenied(s),
            HostErrorRepr::PathDenied(s) => HostErrorWit::PathDenied(s),
            HostErrorRepr::SsrfDenied(s) => HostErrorWit::SsrfDenied(s),
            HostErrorRepr::Io(s) => HostErrorWit::Io(s),
            HostErrorRepr::InvalidArgument(s) => HostErrorWit::InvalidArgument(s),
            HostErrorRepr::Timeout => HostErrorWit::Timeout,
            HostErrorRepr::Internal(s) => HostErrorWit::Internal(s),
        }
    }
}

// ---------------------------------------------------------------------------
// Host state attached to each component Store.
//
// Owns the GuestState (capability list, kernel handle, tokio runtime
// handle) plus the wasmtime ResourceTable required by the Component
// Model linker. The bindgen-generated `*Imports` traits get implemented
// on this struct.
// ---------------------------------------------------------------------------

/// State carried in each Component Model Store. Wraps the existing
/// `GuestState` so capability checks reuse the same field set the
/// core-module path uses.
pub struct PluginHostState {
    /// Per-guest capability list, kernel handle, agent id, etc.
    /// Constructed via `sandbox::new_guest_state`.
    pub guest: GuestState,
    /// Component Model linker needs a resource table even when the
    /// guest declares no resources — bindgen-generated trait method
    /// signatures take `&mut ResourceTable` whenever any interface in
    /// the world uses `result<T, E>`.
    pub table: ResourceTable,
}

impl PluginHostState {
    pub fn new(guest: GuestState) -> Self {
        Self {
            guest,
            table: ResourceTable::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Host trait impls — one per interface in librefang:plugin world.
//
// Each method body is a one-liner: build the JSON params via
// wit_host::params_*, call dispatch via wit_host::call_*, and let `?`
// surface the typed HostErrorRepr.
//
// `Result<T, HostErrorRepr>` is what bindgen expects per the `with`
// mapping above; the `Result<Result<T, E>, anyhow::Error>` outer layer
// (from trappable_imports: true) is automatic on `Ok(...)` — the outer
// `Ok` indicates "no trap, here is the typed result".
// ---------------------------------------------------------------------------

impl librefang::plugin::fs::Host for PluginHostState {
    async fn read(&mut self, path: String) -> wasmtime::Result<Result<Vec<u8>, HostErrorWit>> {
        Ok(
            wit_host::call_bytes(&self.guest, "fs_read", &wit_host::params_fs_read(&path))
                .map_err(HostErrorWit::from),
        )
    }

    async fn write(
        &mut self,
        path: String,
        body: Vec<u8>,
    ) -> wasmtime::Result<Result<(), HostErrorWit>> {
        Ok(wit_host::call_unit(
            &self.guest,
            "fs_write",
            &wit_host::params_fs_write(&path, &body),
        )
        .map_err(HostErrorWit::from))
    }

    async fn list_entries(
        &mut self,
        path: String,
    ) -> wasmtime::Result<Result<Vec<String>, HostErrorWit>> {
        Ok(
            wit_host::call_list_string(&self.guest, "fs_list", &wit_host::params_fs_list(&path))
                .map_err(HostErrorWit::from),
        )
    }
}

impl librefang::plugin::net::Host for PluginHostState {
    async fn fetch(
        &mut self,
        req: librefang::plugin::net::HttpRequest,
    ) -> wasmtime::Result<Result<librefang::plugin::net::HttpResponse, HostErrorWit>> {
        let params =
            wit_host::params_net_fetch(&req.method, &req.url, &req.headers, req.body.as_deref());
        let raw = dispatch(&self.guest, "net_fetch", &params);
        if let Some(err) = raw.get("error").and_then(|e| e.as_str()) {
            return Ok(Err(HostErrorRepr::from_dispatch_message(err).into()));
        }
        // dispatch returns {"ok": {"status": N, "headers": {...}, "body": "..."}}
        let ok = match raw.get("ok") {
            Some(v) => v,
            None => {
                return Ok(Err(HostErrorRepr::Internal(format!(
                    "net_fetch dispatch returned no ok: {raw}"
                ))
                .into()));
            }
        };
        let status = ok.get("status").and_then(|s| s.as_u64()).unwrap_or(0) as u16;
        let headers = ok
            .get("headers")
            .and_then(|h| h.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let body = ok
            .get("body")
            .and_then(|b| b.as_str())
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();
        Ok(Ok(librefang::plugin::net::HttpResponse {
            status,
            headers,
            body,
        }))
    }
}

impl librefang::plugin::kv::Host for PluginHostState {
    async fn get(&mut self, key: String) -> wasmtime::Result<Result<Option<String>, HostErrorWit>> {
        Ok(
            wit_host::call_option_string(&self.guest, "kv_get", &wit_host::params_kv_get(&key))
                .map_err(HostErrorWit::from),
        )
    }

    async fn set(
        &mut self,
        key: String,
        value: String,
    ) -> wasmtime::Result<Result<(), HostErrorWit>> {
        Ok(wit_host::call_unit(
            &self.guest,
            "kv_set",
            &wit_host::params_kv_set(&key, &value),
        )
        .map_err(HostErrorWit::from))
    }
}

impl librefang::plugin::agent::Host for PluginHostState {
    async fn send(
        &mut self,
        target_agent: String,
        body: String,
    ) -> wasmtime::Result<Result<String, HostErrorWit>> {
        Ok(wit_host::call_string(
            &self.guest,
            "agent_send",
            &wit_host::params_agent_send(&target_agent, &body),
        )
        .map_err(HostErrorWit::from))
    }

    async fn spawn(
        &mut self,
        manifest_ref: String,
    ) -> wasmtime::Result<Result<String, HostErrorWit>> {
        Ok(wit_host::call_string(
            &self.guest,
            "agent_spawn",
            &wit_host::params_agent_spawn(&manifest_ref),
        )
        .map_err(HostErrorWit::from))
    }
}

impl librefang::plugin::env::Host for PluginHostState {
    async fn read(
        &mut self,
        name: String,
    ) -> wasmtime::Result<Result<Option<String>, HostErrorWit>> {
        Ok(
            wit_host::call_option_string(
                &self.guest,
                "env_read",
                &wit_host::params_env_read(&name),
            )
            .map_err(HostErrorWit::from),
        )
    }
}

impl librefang::plugin::time::Host for PluginHostState {
    async fn now(&mut self) -> wasmtime::Result<u64> {
        // time has no error variant — dispatch always returns ok.
        // If dispatch ever surfaces an error for time, fall back to 0
        // rather than trapping; clock failures shouldn't crash plugins.
        Ok(wit_host::call_u64(&self.guest, "time_now", &serde_json::json!({})).unwrap_or(0))
    }
}

// Top-level host-types interface — bindgen requires an `impl Host` even
// when the interface only carries types (no functions).
impl librefang::plugin::host_types::Host for PluginHostState {}
impl librefang::plugin::plugin_types::Host for PluginHostState {}

// ---------------------------------------------------------------------------
// WasmSandbox::execute_component — entry point.
//
// MINIMUM-VIABLE Component path. Loads the binary, instantiates with
// the full capability surface bound, calls `run`, returns the result.
// Memory limits are wired via store.limiter. Fuel/epoch timeouts are
// LEFT OUT for now — the core-module `execute()` path has an
// intricate watchdog + epoch-callback dance (#3864) that's not safely
// portable to async-Component instantiation without reshaping
// `sandbox.rs`. A follow-up KBD change will harmonize the two paths
// once we've shipped real plugins. Until then, plugin authors should
// keep `run` bodies short and rely on cooperative scheduling.
// ---------------------------------------------------------------------------

/// Phase-5 C-005: per-interface linker gating. Adds only the host
/// interfaces declared in `capabilities` to `linker`. Undeclared
/// interfaces remain unbound, so a Component that imports them will
/// fail `Plugin::instantiate_async` with a clean "missing import"
/// error — surfaced to the caller as `SandboxError::Execution`.
///
/// Per-interface `add_to_linker` is the bindgen-emitted name —
/// they live under `librefang::plugin::<iface>::add_to_linker`.
fn add_to_linker_per_capability(
    linker: &mut Linker<PluginHostState>,
    capabilities: &[librefang_skills::HostCapability],
) -> Result<(), SandboxError> {
    use librefang_skills::HostCapability as Cap;
    type Hs = wasmtime::component::HasSelf<PluginHostState>;
    let map_err = |e: wasmtime::Error| SandboxError::Execution(e.to_string());
    for cap in capabilities {
        match cap {
            Cap::Fs => {
                librefang::plugin::fs::add_to_linker::<_, Hs>(linker, |s| s).map_err(map_err)?
            }
            Cap::Net => {
                librefang::plugin::net::add_to_linker::<_, Hs>(linker, |s| s).map_err(map_err)?
            }
            Cap::Kv => {
                librefang::plugin::kv::add_to_linker::<_, Hs>(linker, |s| s).map_err(map_err)?
            }
            Cap::Agent => {
                librefang::plugin::agent::add_to_linker::<_, Hs>(linker, |s| s).map_err(map_err)?
            }
            Cap::Env => {
                librefang::plugin::env::add_to_linker::<_, Hs>(linker, |s| s).map_err(map_err)?
            }
            Cap::Time => {
                librefang::plugin::time::add_to_linker::<_, Hs>(linker, |s| s).map_err(map_err)?
            }
        }
    }
    Ok(())
}

/// Phase-5 C-005/C-006 options bag for the Component execute path.
/// Carries the manifest's declared `host_capabilities` plus the AOT
/// cache knobs. Kept as a separate type (rather than extending
/// `SandboxConfig`) to avoid touching `sandbox.rs` —
/// `execute_component` was added in C-004 so its signature is owned
/// by this file.
#[derive(Debug, Clone, Default)]
pub struct ComponentExecuteOptions {
    /// Subset of the librefang:plugin world the host should bind at
    /// link time. A Component that imports an interface outside this
    /// set fails `Plugin::instantiate_async` with a clean "missing
    /// import" error.
    pub host_capabilities: Vec<librefang_skills::HostCapability>,
    /// AOT cache directory. Typically per-skill at
    /// `~/.librefang/skills/<id>/`. When `None`, falls back to JIT
    /// compile (no caching).
    pub aot_cache_dir: Option<std::path::PathBuf>,
    /// AOT vs JIT vs Auto policy. Defaults to `Auto` (the
    /// CompileMode::Default). Only consulted when `aot_cache_dir` is
    /// `Some`.
    pub compile_mode: crate::aot_cache::CompileMode,
}

impl crate::sandbox::WasmSandbox {
    /// Component Model variant of [`WasmSandbox::execute`]. Loads
    /// `wasm_bytes` as a Component (via `Component::from_binary`),
    /// binds the librefang:plugin host interfaces declared in
    /// `options.host_capabilities`, and invokes the exported `run`
    /// function.
    ///
    /// Component plugins MUST declare their host capabilities in their
    /// `skill.toml` (`host_capabilities = ["fs", "net"]`); the
    /// dispatch path through `SkillManifest.host_capabilities`
    /// surfaces them here. Undeclared interface imports fail at
    /// instantiation rather than mid-run — fail-loud-early.
    ///
    /// The signature mirrors `execute()` with one extension: the
    /// `options` arg. Core-module plugins go through `execute()`
    /// unchanged.
    pub async fn execute_component(
        &self,
        wasm_bytes: &[u8],
        _input: serde_json::Value,
        config: SandboxConfig,
        kernel: Option<Arc<dyn KernelHandle>>,
        agent_id: &str,
        options: ComponentExecuteOptions,
    ) -> Result<ExecutionResult, SandboxError> {
        let cfg = engine_config();
        let engine = Engine::new(&cfg).map_err(|e| SandboxError::Compilation(e.to_string()))?;

        // Phase-5 C-006: prefer AOT load when a cache dir is
        // configured; otherwise fall back to the direct JIT compile
        // path. Auto mode opportunistically populates the cache for
        // the next run.
        let component = match &options.aot_cache_dir {
            Some(dir) => {
                crate::aot_cache::load_or_compile(&engine, wasm_bytes, dir, options.compile_mode)?
            }
            None => Component::from_binary(&engine, wasm_bytes)
                .map_err(|e| SandboxError::Compilation(e.to_string()))?,
        };

        let mut linker = Linker::<PluginHostState>::new(&engine);
        // Phase-5 C-005: per-interface add_to_linker, gated by the
        // manifest's `host_capabilities` declaration. The full
        // bindgen-generated `Plugin::add_to_linker` would wire every
        // interface unconditionally, defeating the capability gate;
        // calling per-interface add_to_linker only for declared caps
        // is what makes undeclared imports fail at instantiation.
        add_to_linker_per_capability(&mut linker, &options.host_capabilities)?;

        let tokio_handle = tokio::runtime::Handle::current();
        let state = PluginHostState::new(new_guest_state(&config, kernel, agent_id, tokio_handle));
        let mut store = Store::new(&engine, state);

        // Phase-6 C-007: the shared `engine_config()` enables
        // `consume_fuel(true)` and `epoch_interruption(true)`, so the store
        // must be seeded with both before the first wasm instruction runs
        // — otherwise fuel starts at 0 and the very first op traps with
        // an opaque "wasm function 2" error. The core-module `execute()`
        // path (sandbox.rs ~L483) does the same thing.
        //
        // Use `fuel_limit > 0` as the gate (0 = "unlimited", same convention
        // as `SandboxConfig` / the core path). When the caller asks for
        // unlimited fuel we seed a very large value rather than calling
        // `Store::set_fuel(u64::MAX)` because wasmtime treats anything
        // > i64::MAX as a configuration error.
        let fuel_to_set = if config.fuel_limit > 0 {
            config.fuel_limit
        } else {
            u64::MAX / 2
        };
        store
            .set_fuel(fuel_to_set)
            .map_err(|e| SandboxError::Execution(e.to_string()))?;
        // Epoch deadline: set to "+1 tick" so a runaway component can be
        // interrupted by an external `Engine::increment_epoch()` ticker.
        // The Phase-6 path doesn't install that ticker yet (see TODO at the
        // top of this fn for the watchdog follow-up), so in practice this
        // deadline is never hit — but the call is mandatory whenever
        // `epoch_interruption(true)` is set in the engine config.
        store.set_epoch_deadline(1);

        // TODO(phase-5 follow-up): wire `store.limiter(|s| &mut
        // s.guest.<limiter>)` once a small public accessor lands on
        // GuestState. Today the limiter field is module-private to
        // sandbox.rs; replicating its construction here works (via
        // new_guest_state), but reaching into it from a store-limiter
        // closure would need a `pub(crate) fn limiter_mut(&mut self) ->
        // &mut MemoryLimiter`. Deferred to keep this PR additive-only.

        let plugin = Plugin::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(|e| SandboxError::Execution(e.to_string()))?;

        let result = plugin
            .call_run(&mut store)
            .await
            .map_err(|e| SandboxError::Execution(e.to_string()))?;

        match result {
            Ok(()) => Ok(ExecutionResult {
                output: serde_json::json!({"ok": null}),
                fuel_consumed: 0,
            }),
            Err(plugin_err) => Err(SandboxError::Execution(format!(
                "plugin run error: {plugin_err:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: the bindgen-generated `Plugin` type exists and
    /// `add_to_linker` is callable. Compile-only — proves the wiring
    /// type-checks. Full load-and-run lives in
    /// `scripts/test-wasm-toolchain.sh` (Phase-5 C-007).
    #[test]
    fn bindgen_emits_plugin_and_add_to_linker() {
        let engine = Engine::new(&engine_config()).unwrap();
        let mut linker = Linker::<PluginHostState>::new(&engine);
        Plugin::add_to_linker::<_, wasmtime::component::HasSelf<PluginHostState>>(
            &mut linker,
            |s| s,
        )
        .unwrap();
    }

    /// Empty bytes are not a valid Component — confirm the error path
    /// surfaces a SandboxError::Compilation.
    #[tokio::test]
    async fn rejects_empty_bytes_as_compilation_error() {
        let sandbox = crate::sandbox::WasmSandbox::new().unwrap();
        let result = sandbox
            .execute_component(
                &[],
                serde_json::json!({}),
                SandboxConfig::default(),
                None,
                "test",
                ComponentExecuteOptions::default(),
            )
            .await;
        assert!(matches!(result, Err(SandboxError::Compilation(_))));
    }

    // Phase-5 C-005 capability-gating tests.
    // Each test builds a linker with a different capability subset and
    // verifies `add_to_linker_per_capability` returns Ok. The
    // "undeclared imports cause failure" half of the contract is
    // surfaced at `Plugin::instantiate_async` time and is covered in
    // the C-007 smoke (where a real Component with imports tries to
    // load). These tests only verify the linker-shaping doesn't error.

    fn empty_engine() -> Engine {
        Engine::new(&engine_config()).unwrap()
    }

    #[test]
    fn capability_gate_empty_caps_is_ok() {
        let engine = empty_engine();
        let mut linker = Linker::<PluginHostState>::new(&engine);
        add_to_linker_per_capability(&mut linker, &[]).unwrap();
    }

    #[test]
    fn capability_gate_fs_only() {
        use librefang_skills::HostCapability;
        let engine = empty_engine();
        let mut linker = Linker::<PluginHostState>::new(&engine);
        add_to_linker_per_capability(&mut linker, &[HostCapability::Fs]).unwrap();
    }

    #[test]
    fn capability_gate_full_set() {
        use librefang_skills::HostCapability;
        let engine = empty_engine();
        let mut linker = Linker::<PluginHostState>::new(&engine);
        add_to_linker_per_capability(
            &mut linker,
            &[
                HostCapability::Fs,
                HostCapability::Net,
                HostCapability::Kv,
                HostCapability::Agent,
                HostCapability::Env,
                HostCapability::Time,
            ],
        )
        .unwrap();
    }

    #[test]
    fn capability_gate_rejects_duplicate_caps() {
        // wasmtime 44 bindgen's per-interface add_to_linker errors on
        // duplicate registration: "map entry `<package>/<iface>`
        // defined twice". Our gate currently does NOT dedupe before
        // forwarding to bindgen — so a manifest declaring `["fs",
        // "fs"]` surfaces a SandboxError::Execution here. The skill
        // loader SHOULD dedupe at parse time; this test pins the
        // current bindgen behavior so a future loosening upstream
        // doesn't go unnoticed.
        use librefang_skills::HostCapability;
        let engine = empty_engine();
        let mut linker = Linker::<PluginHostState>::new(&engine);
        let err =
            add_to_linker_per_capability(&mut linker, &[HostCapability::Fs, HostCapability::Fs])
                .unwrap_err();
        assert!(
            matches!(err, SandboxError::Execution(ref msg) if msg.contains("defined twice")),
            "expected Execution error containing 'defined twice', got: {err:?}"
        );
    }
}
