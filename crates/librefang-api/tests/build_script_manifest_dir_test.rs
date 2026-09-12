use std::fs;
use std::path::Path;
use std::process::Command;

fn check(worktree: &Path, target: &Path) {
    let output = Command::new("cargo")
        .args(["check", "--quiet", "--package", "build-path-regression"])
        .arg("--manifest-path")
        .arg(worktree.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("run the build-script fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn dashboard_placeholder_follows_the_running_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let target = temp.path().join("target");
    let manifest = "[package]\nname = \"build-path-regression\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[build-dependencies]\nchrono = \"0.4\"\nwhich = \"8\"\n";
    for root in [&first, &second] {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
        fs::write(root.join("build.rs"), include_str!("../build.rs")).unwrap();
        fs::write(
            root.join("build_paths.rs"),
            include_str!("../build_paths.rs"),
        )
        .unwrap();
    }
    // Equal inputs exercise reuse of a build-script run, not a source edit.
    for file in ["Cargo.toml", "src/lib.rs", "build.rs", "build_paths.rs"] {
        let modified = fs::metadata(first.join(file)).unwrap().modified().unwrap();
        fs::File::options()
            .write(true)
            .open(second.join(file))
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }
    check(&first, &target);
    assert!(first.join("static/react").is_dir());
    check(&second, &target);
    assert!(
        second.join("static/react").is_dir(),
        "shared-target check must run the build script for the new checkout"
    );

    // Execute the cached binary with a third manifest directory. This catches
    // compile-time env! even when Cargo happens to recompile in the second tree.
    let third = temp.path().join("third");
    fs::create_dir(&third).unwrap();
    let executable = format!("build-script-build{}", std::env::consts::EXE_SUFFIX);
    let binary = fs::read_dir(target.join("debug/build"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(&executable))
        .find(|path| {
            path.is_file()
                && path
                    .parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("build-path-regression-")
        })
        .expect("compiled build script");
    let output = Command::new(binary)
        .current_dir(&third)
        .env("CARGO_MANIFEST_DIR", &third)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(third.join("static/react").is_dir());
}
