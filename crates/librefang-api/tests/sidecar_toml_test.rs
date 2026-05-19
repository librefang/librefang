use librefang_api::routes::sidecar_toml::upsert_sidecar_block;
use std::collections::BTreeMap;
use std::fs;
use tempfile::NamedTempFile;

fn pairs(input: &[(&str, &str)]) -> BTreeMap<String, String> {
    input
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn appends_when_absent_preserves_existing_keys() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(tmp.path(), "[default_model]\nprovider = \"ollama\"\n").unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "1,2")]),
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(content.contains("[default_model]"));
    assert!(content.contains("[[sidecar_channels]]"));
    assert!(content.contains("name = \"telegram\""));
    assert!(content.contains("channel_type = \"telegram\""));
    assert!(content.contains("ALLOWED_USERS = \"1,2\""));
}

#[test]
fn replaces_existing_block_with_same_name() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(
        tmp.path(),
        "[[sidecar_channels]]\n\
         name = \"telegram\"\n\
         channel_type = \"telegram\"\n\
         command = \"python3\"\n\
         args = [\"-m\", \"librefang.sidecar.adapters.telegram\"]\n\
         \n\
         [sidecar_channels.env]\n\
         TELEGRAM_BOT_TOKEN = \"old\"\n\
         OBSOLETE = \"x\"\n",
    )
    .unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "1,2")]),
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        !content.contains("OBSOLETE"),
        "stale env keys must be replaced wholesale, not merged"
    );
    assert!(
        !content.contains("TELEGRAM_BOT_TOKEN"),
        "token field is never in config.toml — goes to secrets.env"
    );
    assert!(content.contains("ALLOWED_USERS = \"1,2\""));
}

#[test]
fn does_not_touch_other_sidecar_blocks() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(
        tmp.path(),
        "[[sidecar_channels]]\nname = \"ntfy\"\nchannel_type = \"ntfy\"\n\
         command = \"python3\"\nargs = [\"-m\",\"librefang.sidecar.adapters.ntfy\"]\n\
         [sidecar_channels.env]\nNTFY_TOPIC = \"alerts\"\n\
         \n\
         [[sidecar_channels]]\nname = \"telegram\"\nchannel_type = \"telegram\"\n\
         command = \"python3\"\nargs = [\"-m\",\"librefang.sidecar.adapters.telegram\"]\n\
         [sidecar_channels.env]\n",
    )
    .unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "99")]),
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        content.contains("NTFY_TOPIC = \"alerts\""),
        "ntfy block must be untouched"
    );
    assert!(content.contains("ALLOWED_USERS = \"99\""));
}
