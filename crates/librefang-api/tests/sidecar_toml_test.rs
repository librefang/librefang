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
        &["ALLOWED_USERS"],
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

    // OBSOLETE and TELEGRAM_BOT_TOKEN are both schema-managed in this
    // test's view of the world — the form clearing them should drop
    // them from the env table. (TELEGRAM_BOT_TOKEN being inline in
    // config.toml is a legacy / hand-edited situation; the real schema
    // marks it as `secret` so it would NOT be schema-managed here.
    // For this test we explicitly list it as managed to assert removal
    // semantics for a clearing form.)
    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "1,2")]),
        &["ALLOWED_USERS", "OBSOLETE", "TELEGRAM_BOT_TOKEN"],
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        !content.contains("OBSOLETE"),
        "schema-managed key cleared by form must be removed"
    );
    assert!(
        !content.contains("TELEGRAM_BOT_TOKEN"),
        "schema-managed key cleared by form must be removed"
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
        &["ALLOWED_USERS"],
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        content.contains("NTFY_TOPIC = \"alerts\""),
        "ntfy block must be untouched"
    );
    assert!(content.contains("ALLOWED_USERS = \"99\""));
}

#[test]
fn preserves_operator_tuned_fields_on_replace() {
    // Operator-tuned supervision fields (`restart`, retry/backoff
    // limits, `ready_timeout_secs`, `message_buffer`, `overflow`) live
    // on the same `[[sidecar_channels]]` table but are NOT part of the
    // configure form's schema-managed key set. Replacing the whole
    // block on every save would silently revert them to the serde
    // defaults — a regression the codex review caught. Schema-managed
    // env keys still replace wholesale (see existing
    // `replaces_existing_block_with_same_name`).
    let tmp = NamedTempFile::new().unwrap();
    fs::write(
        tmp.path(),
        "[[sidecar_channels]]\n\
         name = \"telegram\"\n\
         channel_type = \"telegram\"\n\
         command = \"python3\"\n\
         args = [\"-m\",\"librefang.sidecar.adapters.telegram\"]\n\
         restart = false\n\
         restart_max_retries = 5\n\
         ready_timeout_secs = 60\n\
         message_buffer = 200\n\
         \n\
         [sidecar_channels.env]\n\
         OBSOLETE = \"x\"\n",
    )
    .unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "1")]),
        &["ALLOWED_USERS", "OBSOLETE"],
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(content.contains("restart = false"), "restart preserved");
    assert!(content.contains("restart_max_retries = 5"));
    assert!(content.contains("ready_timeout_secs = 60"));
    assert!(content.contains("message_buffer = 200"));
    assert!(
        !content.contains("OBSOLETE"),
        "schema-managed env wholly replaced"
    );
    assert!(content.contains("ALLOWED_USERS = \"1\""));
}

#[test]
fn preserves_operator_custom_command_and_args_on_replace() {
    // Operators sometimes hand-edit `command` to a venv-pinned interpreter
    // (`/opt/venv/bin/python`) or add extra `args` (`--debug`). Saving from
    // the dashboard sends the static SIDECAR_CATALOG defaults (`python3` +
    // module-load args); without this guard those defaults would silently
    // overwrite the operator's edits on every save. INSERT path still
    // writes the catalog defaults — only UPDATE preserves.
    let tmp = NamedTempFile::new().unwrap();
    fs::write(
        tmp.path(),
        "[[sidecar_channels]]\n\
         name = \"telegram\"\n\
         channel_type = \"telegram\"\n\
         command = \"/opt/venv/bin/python\"\n\
         args = [\"-m\",\"librefang.sidecar.adapters.telegram\",\"--debug\"]\n\
         \n\
         [sidecar_channels.env]\n\
         OLD = \"x\"\n",
    )
    .unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3", // catalog default — must NOT overwrite the venv path
        &["-m", "librefang.sidecar.adapters.telegram"], // catalog default — must NOT drop --debug
        &pairs(&[("ALLOWED_USERS", "1")]),
        &["ALLOWED_USERS", "OLD"],
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        content.contains("/opt/venv/bin/python"),
        "operator's custom command path preserved: {content}"
    );
    assert!(
        content.contains("--debug"),
        "operator's extra args preserved: {content}"
    );
    // Schema-managed env keys are still cleared when the form clears them.
    assert!(!content.contains("OLD"));
    assert!(content.contains("ALLOWED_USERS = \"1\""));
}

#[test]
fn backfills_command_and_args_when_existing_block_is_a_stub() {
    // An existing block that lacks `command` / `args` entirely
    // (hand-written stub, partial migration, …) should be backfilled
    // with the catalog defaults on the next save — otherwise the kernel
    // would refuse to spawn the sidecar at all.
    let tmp = NamedTempFile::new().unwrap();
    fs::write(
        tmp.path(),
        "[[sidecar_channels]]\n\
         name = \"telegram\"\n\
         channel_type = \"telegram\"\n\
         \n\
         [sidecar_channels.env]\n",
    )
    .unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "1")]),
        &["ALLOWED_USERS"],
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        content.contains("command = \"python3\""),
        "stub block missing command was backfilled: {content}"
    );
    assert!(
        content.contains("librefang.sidecar.adapters.telegram"),
        "stub block missing args was backfilled: {content}"
    );
}

#[test]
fn preserves_non_schema_env_keys_on_replace() {
    // Operators sometimes hand-edit the `[sidecar_channels.env]` table
    // with operational vars the schema doesn't know about — `PYTHONPATH`
    // for a custom adapter import path, `HTTP_PROXY` for an outbound
    // proxy, locale variables, even a legacy hand-edited
    // `TELEGRAM_BOT_TOKEN` inline. The form only owns the keys it
    // renders (the "schema-managed" set passed via `managed_env_keys`);
    // every other key must survive the save untouched.
    let tmp = NamedTempFile::new().unwrap();
    fs::write(
        tmp.path(),
        "[[sidecar_channels]]\n\
         name = \"telegram\"\n\
         channel_type = \"telegram\"\n\
         command = \"python3\"\n\
         args = [\"-m\",\"librefang.sidecar.adapters.telegram\"]\n\
         \n\
         [sidecar_channels.env]\n\
         PYTHONPATH = \"/custom\"\n\
         HTTP_PROXY = \"http://p\"\n\
         ALLOWED_USERS = \"old\"\n",
    )
    .unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "new")]),
        // Schema-managed non-secret keys only — PYTHONPATH / HTTP_PROXY
        // are intentionally absent and must be preserved.
        &["ALLOWED_USERS", "TELEGRAM_CLEAR_DONE_REACTION"],
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        content.contains("PYTHONPATH = \"/custom\""),
        "non-schema key preserved across save: {content}"
    );
    assert!(
        content.contains("HTTP_PROXY = \"http://p\""),
        "non-schema key preserved across save: {content}"
    );
    assert!(
        content.contains("ALLOWED_USERS = \"new\""),
        "schema-managed value updated from form: {content}"
    );
}

#[test]
fn removes_schema_managed_env_keys_when_form_clears_them() {
    // When the form omits a key it owns (managed_env_keys contains it
    // but the env map does not, or the value is empty), the key must
    // be removed from the env table. Non-schema keys are still preserved.
    let tmp = NamedTempFile::new().unwrap();
    fs::write(
        tmp.path(),
        "[[sidecar_channels]]\n\
         name = \"telegram\"\n\
         channel_type = \"telegram\"\n\
         command = \"python3\"\n\
         args = [\"-m\",\"librefang.sidecar.adapters.telegram\"]\n\
         \n\
         [sidecar_channels.env]\n\
         PYTHONPATH = \"/custom\"\n\
         ALLOWED_USERS = \"1,2\"\n",
    )
    .unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[]), // form cleared ALLOWED_USERS
        &["ALLOWED_USERS", "TELEGRAM_CLEAR_DONE_REACTION"],
    )
    .unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(
        content.contains("PYTHONPATH"),
        "non-schema key preserved when form has no value: {content}"
    );
    assert!(
        !content.contains("ALLOWED_USERS"),
        "schema-managed key removed when form cleared it: {content}"
    );
}
