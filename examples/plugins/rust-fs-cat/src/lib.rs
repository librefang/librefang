//! rust-fs-cat — Phase-6 plugin example exercising the `fs`
//! host capability through the librefang:plugin world.
//!
//! Reads `/tmp/test-input.txt` via `fs.read` and writes its content
//! back to `/tmp/test-output.txt` via `fs.write`. Returns a typed
//! `plugin-error` on any host-side failure so the integration test
//! can assert success / failure cleanly.
//!
//! Build:  `cargo xtask plugins-rebuild rust-fs-cat`
//! Run:    `cargo run --example load_and_run -p librefang-runtime --
//!          examples/plugins/rust-fs-cat/pre-built/plugin.wasm --invoke`
//! (Run requires a host that grants the `fs` HostCapability AND the
//! fine-grained `Capability::FileRead/FileWrite` for the target paths.)

#[allow(warnings)]
mod bindings;

use bindings::librefang::plugin::fs;
use bindings::librefang::plugin::plugin_types::PluginError;
use bindings::Guest;

const INPUT_PATH: &str = "/tmp/test-input.txt";
const OUTPUT_PATH: &str = "/tmp/test-output.txt";

struct Component;

impl Guest for Component {
    fn run() -> Result<(), PluginError> {
        let contents = fs::read(INPUT_PATH)
            .map_err(|e| PluginError::Internal(format!("fs.read({INPUT_PATH}) failed: {e:?}")))?;
        fs::write(OUTPUT_PATH, &contents)
            .map_err(|e| PluginError::Internal(format!("fs.write({OUTPUT_PATH}) failed: {e:?}")))?;
        Ok(())
    }
}

bindings::export!(Component with_types_in bindings);
