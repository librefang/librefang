//! Build automation tasks for the LibreFang workspace.

mod build_web;
mod changelog;
mod check_links;
mod ci;
mod codegen;
mod coverage;
mod deps;
mod dist;
mod docker;
mod integration_test;
mod publish_sdks;
mod release;
mod setup;
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

    /// Publish SDKs to npm, PyPI, and crates.io
    PublishSdks(publish_sdks::PublishSdksArgs),

    /// Build release binaries for multiple platforms
    Dist(dist::DistArgs),

    /// Build and optionally push Docker image
    Docker(docker::DockerArgs),

    /// Set up local development environment
    Setup(setup::SetupArgs),

    /// Generate test coverage report
    Coverage(coverage::CoverageArgs),

    /// Audit dependencies for vulnerabilities and updates
    Deps(deps::DepsArgs),

    /// Run code generation (OpenAPI spec, etc.)
    Codegen(codegen::CodegenArgs),

    /// Check for broken links in documentation
    CheckLinks(check_links::CheckLinksArgs),
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
        Command::PublishSdks(args) => publish_sdks::run(args),
        Command::Dist(args) => dist::run(args),
        Command::Docker(args) => docker::run(args),
        Command::Setup(args) => setup::run(args),
        Command::Coverage(args) => coverage::run(args),
        Command::Deps(args) => deps::run(args),
        Command::Codegen(args) => codegen::run(args),
        Command::CheckLinks(args) => check_links::run(args),
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
