use std::process::Command;

fn main() {
    // Automatically configure git hooks for all developers on first build.
    let _ = Command::new("git")
        .args(["config", "core.hooksPath", "scripts/hooks"])
        .status();
}
