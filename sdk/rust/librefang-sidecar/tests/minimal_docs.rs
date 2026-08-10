#[test]
fn crate_quick_start_imports_only_the_items_it_uses() {
    let docs = include_str!("../src/lib.rs");
    assert!(docs.contains("use librefang_sidecar::{run_stdio, SendCommand, SidecarAdapter};"));
}
