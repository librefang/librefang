//! SDK for writing LibreFang WASM skills in Rust.
//!
//! A WASM skill is a `cdylib` compiled for `wasm32-unknown-unknown` and run
//! inside LibreFang's in-process `WasmSandbox` (capability-gated,
//! fuel/memory/wall-clock bounded). This crate hides the raw guest ABI behind
//! a single macro plus typed host-call wrappers.
//!
//! # Guest ABI (what the sandbox expects)
//!
//! The module must export `memory`, `alloc(i32) -> i32`, and
//! `execute(i32, i32) -> i64`. `execute` receives a pointer/length to the
//! input JSON and returns a packed `i64` (`(ptr << 32) | len`) pointing at the
//! output JSON. The host provides two imports in the `"librefang"` module:
//! `host_call` (capability-checked RPC) and `host_log`. The [`skill!`] macro
//! emits the `alloc`/`execute` exports for you; `memory` is exported by the
//! `wasm32-unknown-unknown` cdylib automatically.
//!
//! # Example
//!
//! ```ignore
//! use librefang_skill::{skill, Request};
//! use serde_json::{json, Value};
//!
//! fn handle(req: Request) -> Result<Value, String> {
//!     match req.tool.as_str() {
//!         "greet" => {
//!             let who = req.input.get("name").and_then(Value::as_str).unwrap_or("world");
//!             Ok(json!({ "message": format!("hello, {who}") }))
//!         }
//!         other => Err(format!("unknown tool: {other}")),
//!     }
//! }
//!
//! skill!(handle);
//! ```
//!
//! Build it with `cargo build --release --target wasm32-unknown-unknown` and
//! point a `skill.toml` at the artifact:
//!
//! ```toml
//! [skill]
//! name = "greeter"
//!
//! [runtime]
//! type = "wasm"
//! entry = "greeter.wasm"
//! ```

use serde::Deserialize;
use serde_json::Value;

/// The request envelope handed to a skill on every tool invocation.
///
/// Identical in shape to the JSON the Python / Node / Shell runtimes receive,
/// so a skill's tool dispatch is the same regardless of runtime kind.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// The tool the agent invoked (a skill may provide several).
    pub tool: String,
    /// Tool input arguments. `Value::Null` when the agent passed none.
    #[serde(default)]
    pub input: Value,
    /// The skill's `[config]` block from `skill.toml`, or `Value::Null` when
    /// the skill declares no config.
    #[serde(default)]
    pub config: Value,
}

/// Run the handler against a raw input-envelope byte slice and produce the
/// output JSON bytes.
///
/// This is the pure core of [`__rt::execute`] — no pointers, no `unsafe` — so
/// the envelope decode / dispatch / error-wrapping contract is unit-testable
/// on the host. A handler `Err` and a malformed envelope both surface to the
/// agent as `{"error": "..."}`, matching how the subprocess runtimes report a
/// non-zero exit.
pub fn run<F>(input: &[u8], handler: F) -> Vec<u8>
where
    F: FnOnce(Request) -> Result<Value, String>,
{
    let result = match serde_json::from_slice::<Request>(input) {
        Ok(req) => match handler(req) {
            Ok(value) => value,
            Err(message) => serde_json::json!({ "error": message }),
        },
        Err(e) => serde_json::json!({ "error": format!("invalid skill input envelope: {e}") }),
    };
    // A serialize failure here is unreachable for the `Value` we built, but
    // never panic in guest code — fall back to an empty object.
    serde_json::to_vec(&result).unwrap_or_else(|_| b"{}".to_vec())
}

/// Pack a guest pointer and length into the `i64` the host unpacks as
/// `(packed >> 32)` for the pointer and `(packed & 0xFFFF_FFFF)` for the
/// length. The ABI is 32-bit (WASM linear memory), so both halves are `u32`.
#[inline]
pub fn pack(ptr: u32, len: u32) -> i64 {
    ((ptr as i64) << 32) | (len as i64)
}

/// Inverse of [`pack`].
#[inline]
pub fn unpack(packed: i64) -> (u32, u32) {
    // `as u32` truncates to the low 32 bits, so the high half is recovered by
    // shifting first and the low half by a plain cast.
    ((packed >> 32) as u32, packed as u32)
}

/// An error from a host call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// The host returned `{"error": "..."}`.
    Host(String),
    /// The host response could not be decoded as JSON.
    Decode(String),
    /// The host response was valid JSON but neither `ok` nor `error`.
    Shape(String),
    /// A host call was made outside the WASM sandbox (e.g. in a host-side
    /// unit test). Host imports only exist in the `wasm32` guest.
    NotInGuest,
}

impl core::fmt::Display for HostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HostError::Host(m) => write!(f, "host error: {m}"),
            HostError::Decode(m) => write!(f, "host response decode failed: {m}"),
            HostError::Shape(m) => write!(f, "unexpected host response shape: {m}"),
            HostError::NotInGuest => {
                write!(f, "host calls are only available inside the WASM sandbox")
            }
        }
    }
}

impl std::error::Error for HostError {}

/// Interpret a host response `Value` as either its `ok` payload or a
/// [`HostError::Host`]. Pure — unit-testable on the host.
pub fn parse_envelope(response: Value) -> Result<Value, HostError> {
    if let Some(ok) = response.get("ok") {
        Ok(ok.clone())
    } else if let Some(err) = response.get("error") {
        Err(HostError::Host(
            err.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string()),
        ))
    } else {
        Err(HostError::Shape(response.to_string()))
    }
}

/// Severity for [`log`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

#[cfg(target_arch = "wasm32")]
mod imports {
    #[link(wasm_import_module = "librefang")]
    extern "C" {
        #[link_name = "host_call"]
        pub fn host_call(request_ptr: i32, request_len: i32) -> i64;
        #[link_name = "host_log"]
        pub fn host_log(level: i32, msg_ptr: i32, msg_len: i32);
    }
}

/// Invoke a capability-checked host method, e.g.
/// `host_call("fs_read", json!({"path": "data.txt"}))`.
///
/// Returns the `ok` payload on success. The set of methods and the parameters
/// each expects are documented on the typed wrappers in [`host`]; this is the
/// escape hatch when you need one they don't cover. Outside the sandbox (host
/// unit tests) it returns [`HostError::NotInGuest`].
pub fn host_call(method: &str, params: Value) -> Result<Value, HostError> {
    #[cfg(target_arch = "wasm32")]
    {
        let request = serde_json::json!({ "method": method, "params": params });
        let request_bytes =
            serde_json::to_vec(&request).map_err(|e| HostError::Decode(e.to_string()))?;
        // SAFETY: `request_bytes` lives until the end of this scope, which is
        // strictly after the synchronous host call returns, so the host reads
        // a valid slice. The response is written by the host into memory it
        // allocated via our `alloc` export; we copy it out before returning.
        let packed = unsafe {
            imports::host_call(request_bytes.as_ptr() as i32, request_bytes.len() as i32)
        };
        let (ptr, len) = unpack(packed);
        let response_bytes =
            unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) }.to_vec();
        let response: Value = serde_json::from_slice(&response_bytes)
            .map_err(|e| HostError::Decode(e.to_string()))?;
        parse_envelope(response)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (method, params);
        Err(HostError::NotInGuest)
    }
}

/// Emit a log line to the host's structured log (capped and newline-sanitized
/// host-side). No-op outside the sandbox.
pub fn log(level: LogLevel, message: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        imports::host_log(level as i32, message.as_ptr() as i32, message.len() as i32);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (level, message);
    }
}

/// Typed wrappers over [`host_call`] for the built-in host methods.
///
/// Each maps to a method dispatched by the sandbox host (`host_functions`).
/// `time_now`, `kv_*`, `env_read`, and `fs_*` are free; `net_fetch`,
/// `shell_exec`, `agent_send`, and `agent_spawn` charge fuel against the
/// invocation budget (denial-of-wallet guard). Every call is additionally
/// gated by the capability the skill declared in `[requirements] capabilities`.
pub mod host {
    use super::{host_call, HostError};
    use serde_json::{json, Value};

    /// Unix timestamp in seconds. Requires no capability.
    pub fn time_now() -> Result<u64, HostError> {
        host_call("time_now", json!({})).and_then(|v| {
            v.as_u64()
                .ok_or_else(|| HostError::Shape(format!("time_now: expected u64, got {v}")))
        })
    }

    /// Read a file. Needs `FileRead(<glob>)`.
    pub fn fs_read(path: &str) -> Result<Value, HostError> {
        host_call("fs_read", json!({ "path": path }))
    }

    /// Write a file. Needs `FileWrite(<glob>)`.
    pub fn fs_write(path: &str, content: &str) -> Result<Value, HostError> {
        host_call("fs_write", json!({ "path": path, "content": content }))
    }

    /// List a directory. Needs `FileRead(<glob>)`.
    pub fn fs_list(path: &str) -> Result<Value, HostError> {
        host_call("fs_list", json!({ "path": path }))
    }

    /// Read an environment variable. Needs `EnvRead(<glob>)`.
    pub fn env_read(name: &str) -> Result<Value, HostError> {
        host_call("env_read", json!({ "name": name }))
    }

    /// Read a value from the skill's key/value store. Requires no capability.
    pub fn kv_get(key: &str) -> Result<Value, HostError> {
        host_call("kv_get", json!({ "key": key }))
    }

    /// Write a value to the skill's key/value store. Requires no capability.
    pub fn kv_set(key: &str, value: Value) -> Result<Value, HostError> {
        host_call("kv_set", json!({ "key": key, "value": value }))
    }

    /// Perform an HTTP request (`method` defaults host-side to `GET`). Needs
    /// `NetConnect(<host:port>)`. Returns `{ "status": u16, "body": String }`.
    pub fn net_fetch(url: &str, method: &str, body: &str) -> Result<Value, HostError> {
        host_call(
            "net_fetch",
            json!({ "url": url, "method": method, "body": body }),
        )
    }

    /// Run a shell command. Needs `ShellExec(<glob>)`.
    pub fn shell_exec(command: &str) -> Result<Value, HostError> {
        host_call("shell_exec", json!({ "command": command }))
    }

    /// Send a message to another agent. Needs `AgentMessage(<glob>)`.
    pub fn agent_send(target: &str, message: &str) -> Result<Value, HostError> {
        host_call(
            "agent_send",
            json!({ "target": target, "message": message }),
        )
    }

    /// Spawn a sub-agent from a manifest TOML string. Needs `AgentSpawn`.
    pub fn agent_spawn(manifest_toml: &str) -> Result<Value, HostError> {
        host_call("agent_spawn", json!({ "manifest": manifest_toml }))
    }
}

/// Internal runtime glue invoked by the [`skill!`] macro. Not a stable API —
/// call the macro, not these functions.
#[doc(hidden)]
pub mod __rt {
    use super::{pack, run, Request};
    use serde_json::Value;

    /// Allocate `size` bytes of guest memory and return a pointer the host can
    /// write into. The buffer is intentionally leaked: the sandbox `Store` is
    /// torn down after a single `execute`, so per-invocation allocations are
    /// reclaimed wholesale rather than individually freed.
    pub fn alloc(size: i32) -> i32 {
        let buf: Vec<u8> = Vec::with_capacity(size.max(0) as usize);
        let ptr = buf.as_ptr() as i32;
        core::mem::forget(buf);
        ptr
    }

    /// Read the input envelope, run `handler`, write the result back into guest
    /// memory, and return the packed `(ptr, len)`.
    pub fn execute<F>(ptr: i32, len: i32, handler: F) -> i64
    where
        F: FnOnce(Request) -> Result<Value, String>,
    {
        // SAFETY: the host wrote `len` bytes at `ptr` (a pointer it obtained
        // from our `alloc`) immediately before calling `execute`.
        let input = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
        let output = run(input, handler);
        let out_len = output.len() as u32;
        let out_ptr = output.as_ptr() as u32;
        // Leak the output so the host can read it after we return.
        core::mem::forget(output);
        pack(out_ptr, out_len)
    }
}

/// Define a WASM skill's entry points.
///
/// Pass a handler `fn(Request) -> Result<serde_json::Value, String>`. The macro
/// emits the `alloc` and `execute` exports the sandbox requires; `memory` is
/// exported by the cdylib automatically.
#[macro_export]
macro_rules! skill {
    ($handler:expr) => {
        #[no_mangle]
        pub extern "C" fn alloc(size: i32) -> i32 {
            $crate::__rt::alloc(size)
        }

        #[no_mangle]
        pub extern "C" fn execute(ptr: i32, len: i32) -> i64 {
            $crate::__rt::execute(ptr, len, $handler)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pack_unpack_roundtrips_32bit_values() {
        for &(p, l) in &[
            (0u32, 0u32),
            (1024, 16),
            (0xDEAD_BEEF, 0x00FF_FF00),
            (u32::MAX, u32::MAX),
        ] {
            assert_eq!(unpack(pack(p, l)), (p, l));
        }
        // Layout matches what the host unpacks.
        let packed = pack(0x1234, 0x56);
        assert_eq!((packed >> 32) as u32, 0x1234);
        assert_eq!((packed & 0xFFFF_FFFF) as u32, 0x56);
    }

    #[test]
    fn request_envelope_deserializes_with_and_without_config() {
        let with = b"{\"tool\":\"t\",\"input\":{\"a\":1},\"config\":{\"k\":\"v\"}}";
        let req: Request = serde_json::from_slice(with).unwrap();
        assert_eq!(req.tool, "t");
        assert_eq!(req.input, json!({"a": 1}));
        assert_eq!(req.config, json!({"k": "v"}));

        let without = b"{\"tool\":\"t\"}";
        let req: Request = serde_json::from_slice(without).unwrap();
        assert_eq!(req.input, Value::Null);
        assert_eq!(req.config, Value::Null);
    }

    #[test]
    fn run_dispatches_handler_and_serializes_output() {
        let input =
            serde_json::to_vec(&json!({"tool": "greet", "input": {"name": "ada"}})).unwrap();
        let out = run(&input, |req| {
            assert_eq!(req.tool, "greet");
            let name = req.input.get("name").and_then(Value::as_str).unwrap();
            Ok(json!({ "message": format!("hi {name}") }))
        });
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed, json!({"message": "hi ada"}));
    }

    #[test]
    fn run_wraps_handler_error_as_error_object() {
        let input = serde_json::to_vec(&json!({"tool": "boom"})).unwrap();
        let out = run(&input, |_| Err("kaboom".to_string()));
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed, json!({"error": "kaboom"}));
    }

    #[test]
    fn run_wraps_malformed_envelope_as_error_object() {
        let out = run(b"not json", |_| Ok(json!({})));
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("invalid skill input envelope"));
    }

    #[test]
    fn parse_envelope_handles_ok_error_and_shape() {
        assert_eq!(parse_envelope(json!({"ok": 42})).unwrap(), json!(42));
        assert_eq!(
            parse_envelope(json!({"error": "nope"})),
            Err(HostError::Host("nope".to_string()))
        );
        assert!(matches!(
            parse_envelope(json!({"weird": true})),
            Err(HostError::Shape(_))
        ));
    }

    #[test]
    fn host_call_off_guest_is_not_in_guest() {
        assert_eq!(host_call("time_now", json!({})), Err(HostError::NotInGuest));
        assert_eq!(host::time_now(), Err(HostError::NotInGuest));
    }
}
