use librefang_api::routes::secrets_env::upsert_secret;
use std::fs;
use tempfile::NamedTempFile;

#[test]
fn upsert_creates_file_with_600_perms() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    fs::remove_file(&path).unwrap(); // we want upsert to create it
    upsert_secret(&path, "FOO", "bar").unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert_eq!(content.trim(), "FOO=bar");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secrets file must be mode 600");
    }
}

#[test]
fn upsert_replaces_existing_key_preserves_other_lines() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(
        tmp.path(),
        "# top comment\n\
         A=1\n\
         FOO=old\n\
         B=2\n",
    )
    .unwrap();

    upsert_secret(tmp.path(), "FOO", "new").unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert_eq!(
        content,
        "# top comment\n\
         A=1\n\
         FOO=new\n\
         B=2\n"
    );
}

#[test]
fn upsert_appends_when_key_absent() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(tmp.path(), "A=1\n").unwrap();

    upsert_secret(tmp.path(), "B", "2").unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert_eq!(content, "A=1\nB=2\n");
}

#[test]
fn upsert_rejects_invalid_value_chars() {
    let tmp = NamedTempFile::new().unwrap();

    // Newline — preserves the historical "newline" substring assertion.
    let err = upsert_secret(tmp.path(), "K", "line1\nline2").unwrap_err();
    assert!(
        err.contains("newline") && err.contains("`K`"),
        "newline error must mention 'newline' and the key: {err}"
    );

    // NUL byte.
    let err = upsert_secret(tmp.path(), "K", "abc\0def").unwrap_err();
    assert!(
        err.contains("NUL") && err.contains("`K`"),
        "NUL error must mention 'NUL' and the key: {err}"
    );

    // Leading whitespace — would be lost by dotenv parsers.
    let err = upsert_secret(tmp.path(), "K", " abc").unwrap_err();
    assert!(
        err.contains("whitespace") && err.contains("`K`"),
        "leading-whitespace error must mention 'whitespace' and the key: {err}"
    );

    // Starts with a double quote — dotenv reader would strip the quotes
    // and process escape sequences inside, so we'd round-trip the wrong
    // value back to the caller.
    let err = upsert_secret(tmp.path(), "K", "\"x\"").unwrap_err();
    assert!(
        err.contains("quote") && err.contains("`K`"),
        "quoted-value error must mention 'quote' and the key: {err}"
    );
}
