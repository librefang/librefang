use std::path::{Path, PathBuf};

pub fn dashboard_dir() -> PathBuf {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .expect("Cargo must provide CARGO_MANIFEST_DIR when the build script runs");
    dashboard_dir_under(Path::new(&manifest_dir))
}

pub(crate) fn dashboard_dir_under(manifest_dir: &Path) -> PathBuf {
    manifest_dir.join("static").join("react")
}
