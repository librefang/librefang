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
