//! Binary entry point for the Telegram sidecar adapter.

mod access;
mod adapter;
mod api;
mod dispatcher;
mod format;
mod reaction;
mod schema;
mod translator;

use adapter::TelegramAdapter;
use librefang_sidecar::run_stdio_main;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // `run_stdio_main` checks for `--describe` BEFORE calling the builder, so the schema is served even if `TELEGRAM_BOT_TOKEN` is unset at boot — the dashboard can render the configure form first, then the operator sets the token and the supervisor respawns.
    run_stdio_main(schema::telegram_schema, TelegramAdapter::from_env).await
}

#[cfg(test)]
mod packaging_tests {
    #[test]
    fn local_sdk_dependency_also_declares_its_registry_version() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let packaged_original = manifest_dir.join("Cargo.toml.orig");
        let manifest_path = if packaged_original.exists() {
            packaged_original
        } else {
            manifest_dir.join("Cargo.toml")
        };
        let manifest = std::fs::read_to_string(manifest_path).expect("read package manifest");

        assert!(manifest.contains(
            "librefang-sidecar = { path = \"../librefang-sidecar\", version = \"0.1.0\" }"
        ));
    }
}
