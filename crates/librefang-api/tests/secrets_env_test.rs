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
fn upsert_rejects_value_with_newline() {
    let tmp = NamedTempFile::new().unwrap();
    let err = upsert_secret(tmp.path(), "K", "line1\nline2").unwrap_err();
    assert!(err.to_string().contains("newline"));
}
