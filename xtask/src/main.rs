//! Build automation tasks for the LibreFang workspace.

mod build_web;
mod changelog;
mod ci;
mod integration_test;
mod release;
mod sync_versions;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "LibreFang workspace automation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the full release flow (changelog + sync-versions + commit + tag + PR)
    Release(release::ReleaseArgs),

    /// Build frontend assets (web dashboard and/or docs site)
    BuildWeb(build_web::BuildWebArgs),

    /// Run the full CI check suite locally (build + test + clippy + web lint)
    Ci(ci::CiArgs),

    /// Generate CHANGELOG.md entry from merged PRs since last tag
    Changelog(changelog::ChangelogArgs),

    /// Sync version strings across Cargo.toml, JS/Python/Rust SDKs, Tauri, etc.
    SyncVersions(sync_versions::SyncVersionsArgs),

    /// Run live integration tests against a running daemon
    IntegrationTest(integration_test::IntegrationTestArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Release(args) => release::run(args),
        Command::BuildWeb(args) => build_web::run(args),
        Command::Ci(args) => ci::run(args),
        Command::Changelog(args) => changelog::run(args),
        Command::SyncVersions(args) => sync_versions::run(args),
        Command::IntegrationTest(args) => integration_test::run(args),
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
