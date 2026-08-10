#[test]
fn command_parser_documents_its_required_field_policy() {
    let protocol = include_str!("../src/protocol.rs");
    assert!(protocol.contains(
        "Unlike the Python SDK's legacy parser, the Rust SDK intentionally rejects missing required fields"
    ));
}
