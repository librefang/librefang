// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;

#[derive(Parser)]
#[command(name = "librefang-desktop", about = "LibreFang Desktop — Agent OS")]
struct Cli {
    /// Connect to a remote LibreFang server URL (e.g. http://192.168.1.100:4545)
    #[arg(long, value_name = "URL")]
    server_url: Option<String>,

    /// Start local server without showing connection screen
    #[arg(long)]
    local: bool,
}

fn main() {
    // Load ~/.librefang/.env / secrets.env / vault into process env before
    // spawning the runtime. `std::env::set_var` is UB once other threads
    // exist, so this must happen at the synchronous main() boundary — not
    // from inside `LibreFangKernel::boot_with_config`.
    librefang_extensions::dotenv::load_dotenv();

    let cli = Cli::parse();
    librefang_desktop::run(cli.server_url, cli.local);
}
