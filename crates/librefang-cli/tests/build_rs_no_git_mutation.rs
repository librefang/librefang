//! Regression guard: build.rs must not mutate user git config (#3641).

use std::path::PathBuf;

const BUILD_RS_PATH: &str = "build.rs";

fn read_build_rs() -> String {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join(BUILD_RS_PATH);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn strip_comments(src: &str) -> String {
    // Drop // comments so doc notes about the old bug do not trip the check.
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let cleaned = match line.find("//") {
            Some(idx) => &line[..idx],
            None => line,
        };
        out.push_str(cleaned);
        out.push('\n');
    }
    out
}

#[test]
fn build_rs_does_not_mutate_git_config() {
    let src = strip_comments(&read_build_rs());

    // Ban the bare `git config` token; if a future change needs read-only
    // `git config --get`, allow it explicitly here instead of silently.
    assert!(
        !src.contains("\"config\""),
        "build.rs must not invoke `git config` — issue #3641 forbids \
         mutating user git config from a build script. If you need to \
         read a value, use `git config --get` and add an explicit \
         allowance in this test."
    );
    assert!(
        !src.contains("hooksPath"),
        "build.rs must not touch core.hooksPath — see issue #3641."
    );
}

#[test]
fn build_rs_uses_only_read_only_git_subcommands() {
    let src = strip_comments(&read_build_rs());
    // Side-effecting git subcommands have no place in a build script.
    for forbidden in [
        "\"init\"",
        "\"clone\"",
        "\"commit\"",
        "\"push\"",
        "\"pull\"",
        "\"fetch\"",
        "\"checkout\"",
        "\"reset\"",
        "\"add\"",
        "\"rm\"",
    ] {
        assert!(
            !src.contains(forbidden),
            "build.rs must not invoke side-effecting git subcommand {forbidden} \
             — see issue #3641."
        );
    }
}
