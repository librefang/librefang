# librefang-skill

SDK for writing [LibreFang](https://librefang.ai) WASM skills in Rust.

A WASM skill is a `cdylib` compiled for `wasm32-unknown-unknown` and executed inside LibreFang's in-process `WasmSandbox`: capability-gated, with fuel, linear-memory, and wall-clock limits enforced per invocation.
This crate hides the raw guest ABI behind one macro and a set of typed host-call wrappers, so you write a handler and nothing else.

## Quick start

`Cargo.toml`:

```toml
[package]
name = "greeter"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
librefang-skill = "0.1"
serde_json = "1"

[profile.release]
panic = "abort"   # the guest does not unwind; also shrinks the module
```

`src/lib.rs`:

```rust
use librefang_skill::{skill, Request};
use serde_json::{json, Value};

fn handle(req: Request) -> Result<Value, String> {
    match req.tool.as_str() {
        "greet" => {
            let who = req.input.get("name").and_then(Value::as_str).unwrap_or("world");
            Ok(json!({ "message": format!("hello, {who}") }))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

skill!(handle);
```

Build:

```bash
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
# → target/wasm32-unknown-unknown/release/greeter.wasm
```

`skill.toml` (next to the `.wasm`, in the skill directory):

```toml
[skill]
name = "greeter"

[runtime]
type = "wasm"
entry = "greeter.wasm"

[[tools.provided]]
name = "greet"
description = "Greet someone by name."
input_schema = { type = "object", properties = { name = { type = "string" } } }

# Only needed if the skill makes capability-bearing host calls:
# [requirements]
# capabilities = ["NetConnect(api.example.com:443)", "FileRead(/data/*)"]
```

## The request envelope

Every tool invocation calls your handler with a [`Request`]:

| field    | type    | meaning                                                        |
| -------- | ------- | -------------------------------------------------------------- |
| `tool`   | string  | which provided tool the agent invoked                          |
| `input`  | value   | the tool's arguments (`null` if none)                          |
| `config` | value   | the skill's `[config]` block from `skill.toml` (`null` if none)|

Return `Ok(value)` for success or `Err(message)` for failure — an `Err` (or a panic-free internal error) reaches the agent as `{"error": "..."}`, exactly as a non-zero exit does for the subprocess runtimes.

## Host calls

Capability-gated host functions are exposed under [`host`]. Each requires the matching capability in `[requirements] capabilities`, and a few charge fuel against the invocation budget (denial-of-wallet guard):

| function                         | capability            | fuel |
| -------------------------------- | --------------------- | ---- |
| `host::time_now()`               | —                     | free |
| `host::kv_get` / `kv_set`        | —                     | free |
| `host::env_read(name)`           | `EnvRead(<glob>)`     | free |
| `host::fs_read` / `fs_list`      | `FileRead(<glob>)`    | free |
| `host::fs_write`                 | `FileWrite(<glob>)`   | free |
| `host::net_fetch(url, m, body)`  | `NetConnect(<h:port>)`| paid |
| `host::shell_exec(command)`      | `ShellExec(<glob>)`   | paid |
| `host::agent_send(target, msg)`  | `AgentMessage(<glob>)`| paid |
| `host::agent_spawn(manifest)`    | `AgentSpawn`          | paid |

Use `librefang_skill::host_call(method, params)` directly for anything not wrapped, and `librefang_skill::log(level, msg)` to write to the host log.

A capability string that fails to parse is dropped host-side (deny-by-default), so a typo means "not granted" rather than a silent over-grant.

## How it maps to the sandbox ABI

The [`skill!`] macro emits the `alloc(i32) -> i32` and `execute(i32, i32) -> i64` exports the sandbox requires; the cdylib exports `memory` automatically. `execute` is handed a pointer/length to the input JSON and returns a packed `(ptr << 32) | len` pointing at the output JSON. You never touch pointers.
