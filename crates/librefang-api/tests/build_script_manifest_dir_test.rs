#[path = "../build_paths.rs"]
#[allow(dead_code)]
mod build_paths;

use std::path::Path;

#[test]
fn dashboard_placeholder_follows_the_running_worktree() {
    let first = build_paths::dashboard_dir_under(Path::new("/worktrees/first/librefang-api"));
    let second = build_paths::dashboard_dir_under(Path::new("/worktrees/second/librefang-api"));

    assert_eq!(
        first,
        Path::new("/worktrees/first/librefang-api/static/react")
    );
    assert_eq!(
        second,
        Path::new("/worktrees/second/librefang-api/static/react")
    );
    assert_ne!(first, second);
}

#[test]
fn build_script_delegates_runtime_manifest_resolution() {
    let build_script = include_str!("../build.rs");

    assert!(build_script.contains("build_paths::dashboard_dir()"));
    assert!(!build_script.contains("env!(\"CARGO_MANIFEST_DIR\")"));
}
