use librefang_api::routes::sidecar_describe::{describe_sidecar, SidecarSchema};

#[tokio::test]
async fn describe_telegram_returns_schema_or_skips_when_sdk_missing() {
    let result = describe_sidecar(
        "python3",
        &["-m".into(), "librefang.sidecar.adapters.telegram".into()],
    )
    .await;
    let schema: SidecarSchema = match result {
        Ok(s) => s,
        // Local dev without `pip install -e sdk/python` is a valid state;
        // skip rather than fail so CI without the SDK works.
        Err(e) => {
            eprintln!("describe failed (SDK not installed?): {e}");
            return;
        }
    };
    assert_eq!(schema.name, "telegram");
    let bot_token = schema
        .fields
        .iter()
        .find(|f| f.key == "TELEGRAM_BOT_TOKEN")
        .expect("schema must declare TELEGRAM_BOT_TOKEN");
    assert_eq!(bot_token.field_type, "secret");
    assert!(bot_token.required);
}

#[tokio::test]
async fn describe_failing_command_returns_err() {
    let result =
        describe_sidecar("python3", &["-c".into(), "import sys; sys.exit(2)".into()]).await;
    assert!(result.is_err());
}
