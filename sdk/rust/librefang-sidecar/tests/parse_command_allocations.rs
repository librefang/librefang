#[test]
fn parse_command_does_not_clone_the_full_json_value() {
    let protocol = include_str!("../src/protocol.rs");
    assert!(
        !protocol.contains("serde_json::from_value(v.clone())"),
        "known commands must not clone their entire params tree"
    );
}
