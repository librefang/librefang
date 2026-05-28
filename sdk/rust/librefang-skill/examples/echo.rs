//! A minimal LibreFang WASM skill: echoes its input and exposes the wall clock.
//!
//! Built as a `cdylib` for `wasm32-unknown-unknown` this is a complete,
//! installable skill. It is shipped here as an `example` so it stays
//! compile-checked in CI on the host target too — the `skill!` macro emits the
//! `alloc` / `execute` exports, and `fn main` is only present to satisfy the
//! example bin target (it is never called inside the sandbox).
//!
//! Real skill crate `Cargo.toml`:
//!
//! ```toml
//! [lib]
//! crate-type = ["cdylib"]
//!
//! [dependencies]
//! librefang-skill = "0.1"
//! serde_json = "1"
//!
//! [profile.release]
//! panic = "abort"   # no unwinding in the guest; smaller module
//! ```
//!
//! Build: `cargo build --release --target wasm32-unknown-unknown`

use librefang_skill::{host, skill, Request};
use serde_json::{json, Value};

fn handle(req: Request) -> Result<Value, String> {
    match req.tool.as_str() {
        // Pure compute — no host calls, so this tool needs no capabilities.
        "echo" => Ok(json!({ "echoed": req.input })),
        // Uses the free `time_now` host call.
        "now" => {
            let ts = host::time_now().map_err(|e| e.to_string())?;
            Ok(json!({ "unix_secs": ts }))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

skill!(handle);

fn main() {}
