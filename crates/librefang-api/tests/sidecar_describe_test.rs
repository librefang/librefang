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

#[tokio::test]
async fn describe_missing_sdk_returns_actionable_install_hint() {
    // Simulate the exact failure operators hit when librefang-sdk
    // isn't installed in the interpreter the daemon picked. The raw
    // python traceback is cryptic; the translator should rewrite it
    // into a one-line "install librefang-sdk" message that names the
    // command they should run AND warns about the multi-interpreter
    // footgun under mise / pyenv / conda.
    let stderr_payload = "Error while finding module specification for \
         'librefang.sidecar.adapters.telegram' (ModuleNotFoundError: \
         No module named 'librefang')";
    let result = describe_sidecar(
        "python3",
        &[
            "-c".into(),
            format!("import sys; sys.stderr.write({stderr_payload:?}); sys.exit(1)"),
        ],
    )
    .await;
    let err = result.expect_err("missing-SDK shape must surface as Err");
    assert!(
        err.contains("librefang-sdk is not installed"),
        "expected install hint; got: {err}"
    );
    assert!(
        err.contains("pip install librefang-sdk"),
        "expected the install command verbatim; got: {err}"
    );
    assert!(
        err.contains("mise / pyenv / conda"),
        "expected the multi-interpreter footgun warning; got: {err}"
    );
    // The original cryptic ModuleNotFoundError string MUST NOT leak
    // through — that's exactly the noise this translation eliminates.
    assert!(
        !err.contains("ModuleNotFoundError"),
        "raw traceback leaked through translation: {err}"
    );
}

#[tokio::test]
async fn describe_other_failure_modes_keep_raw_stderr() {
    // A non-SDK failure (here: adapter raising a normal ImportError
    // for a typo in its own code) must NOT trigger the install hint —
    // that would mask real bugs. The raw stderr should pass through
    // verbatim so the operator sees the actual problem.
    let result = describe_sidecar(
        "python3",
        &[
            "-c".into(),
            "import sys; sys.stderr.write('ImportError: cannot import name foo'); sys.exit(1)"
                .into(),
        ],
    )
    .await;
    let err = result.expect_err("non-SDK failure must surface as Err");
    assert!(
        !err.contains("librefang-sdk is not installed"),
        "install hint incorrectly fired for unrelated ImportError: {err}"
    );
    assert!(
        err.contains("cannot import name foo"),
        "raw stderr should pass through for non-SDK failures: {err}"
    );
}
