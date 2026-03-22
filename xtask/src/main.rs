//! Build automation tasks for the LibreFang workspace.

pub(crate) mod common;

mod api_docs;
mod bench;
mod build_web;
mod changelog;
mod check_links;
mod ci;
mod clean_all;
mod codegen;
mod contributors;
mod coverage;
mod db;
mod deps;
mod dev;
mod dist;
mod docker;
mod doctor;
mod fmt;
mod integration_test;
mod license_check;
mod loc;
mod migrate;
mod pre_commit;
mod publish_npm_binaries;
mod publish_pypi_binaries;
mod publish_sdks;
mod release;
mod setup;
mod sync_versions;
mod update_deps;
mod validate_config;

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

    /// Run criterion benchmarks
    Bench(bench::BenchArgs),

    /// Migrate agents from other frameworks (OpenClaw, OpenFang)
    Migrate(migrate::MigrateArgs),

    /// Check or fix formatting (Rust + web)
    Fmt(fmt::FmtCheckArgs),

    /// Clean all build artifacts (target/, node_modules/, dist/, .next/)
    CleanAll(clean_all::CleanAllArgs),

    /// Diagnose development environment issues
    Doctor(doctor::DoctorArgs),

    /// Start development environment (daemon + dashboard hot reload)
    Dev(dev::DevArgs),

    /// Database management (info, backup, reset)
    Db(db::DbArgs),

    /// Check dependency licenses for compliance
    LicenseCheck(license_check::LicenseCheckArgs),

    /// Code statistics (lines of code, crate dependency graph)
    Loc(loc::LocArgs),

    /// Update dependencies (Rust + web)
    UpdateDeps(update_deps::UpdateDepsArgs),

    /// Validate config.toml
    ValidateConfig(validate_config::ValidateConfigArgs),

    /// Run pre-commit checks (fmt + clippy + test)
    PreCommit(pre_commit::PreCommitArgs),

    /// Generate API documentation from OpenAPI spec
    ApiDocs(api_docs::ApiDocsArgs),

    /// Generate contributors SVG and star history SVG
    Contributors(contributors::ContributorsArgs),

    /// Publish platform-specific CLI binaries to npm
    PublishNpmBinaries(publish_npm_binaries::PublishNpmBinariesArgs),

    /// Publish platform-specific CLI wheels to PyPI
    PublishPypiBinaries(publish_pypi_binaries::PublishPypiBinariesArgs),
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
        Command::Bench(args) => bench::run(args),
        Command::Migrate(args) => migrate::run(args),
        Command::Fmt(args) => fmt::run(args),
        Command::CleanAll(args) => clean_all::run(args),
        Command::Doctor(args) => doctor::run(args),
        Command::Dev(args) => dev::run(args),
        Command::Db(args) => db::run(args),
        Command::LicenseCheck(args) => license_check::run(args),
        Command::Loc(args) => loc::run(args),
        Command::UpdateDeps(args) => update_deps::run(args),
        Command::ValidateConfig(args) => validate_config::run(args),
        Command::PreCommit(args) => pre_commit::run(args),
        Command::ApiDocs(args) => api_docs::run(args),
        Command::Contributors(args) => contributors::run(args),
        Command::PublishNpmBinaries(args) => publish_npm_binaries::run(args),
        Command::PublishPypiBinaries(args) => publish_pypi_binaries::run(args),
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
