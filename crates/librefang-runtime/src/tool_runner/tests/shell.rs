use super::workspace::make_named_ws_kernel;
use super::*;

/// Regression for #3822: `shell_exec` must be blocked when the command
/// string references a path that falls inside a read-only named workspace.
/// Previously the shell tool had no such check; a plugin could bypass the
/// read-only restriction by issuing a shell command that writes to the
/// supposedly read-only workspace path.
#[tokio::test]
async fn test_shell_exec_blocked_for_readonly_workspace_path() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let shared_canon = shared.path().canonicalize().unwrap();

    // Configure the shared workspace as read-only.
    let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

    // Construct a command string that references the read-only path.
    let ro_path = shared_canon.to_str().unwrap();
    let command = format!("touch {ro_path}/evil.txt");

    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": command}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000008"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(
        result.is_error,
        "shell_exec referencing a read-only workspace path must be blocked; got: {}",
        result.content
    );
    assert!(
        result.content.contains("read-only"),
        "error must mention read-only; got: {}",
        result.content
    );
    // Verify the file was NOT created.
    assert!(!shared_canon.join("evil.txt").exists());
}

/// Read-only workspace enforcement must NOT block commands that do not
/// reference the read-only workspace path.
#[tokio::test]
async fn test_shell_exec_allowed_when_not_targeting_readonly_workspace() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let shared_canon = shared.path().canonicalize().unwrap();

    // Read-only shared workspace — but the command targets the primary workspace.
    let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

    // A command that does NOT reference the read-only path should go through
    // (it may still fail for other reasons — e.g., exec policy — but it must
    // not be blocked by the workspace read-only check).
    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "echo hello"}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000009"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    // Must NOT be blocked by read-only check. It may be blocked by exec policy
    // (if one is set) but should not contain "read-only" in the error.
    if result.is_error {
        assert!(
            !result.content.contains("read-only"),
            "must not be blocked by read-only check; got: {}",
            result.content
        );
    }
}

// ── Reproducer tests for #4903: argument-role-aware RO enforcement ────────
//
// These tests exercise `classify_shell_exec_ro_safety` directly so they
// are synchronous and fast — no kernel setup required.

#[test]
fn ro_safety_cat_is_allowed() {
    // Case 1: `cat /vaults-ro/x/foo.md` → ALLOWED (was blocked before #4903).
    let result = classify_shell_exec_ro_safety("cat /vaults-ro/x/foo.md", "/vaults-ro/x");
    assert_eq!(result, RoSafety::Allow, "cat of an RO path must be allowed");
}

#[test]
fn ro_safety_grep_is_allowed() {
    // Case 2: `grep pattern /vaults-ro/x/notes/*.md` → ALLOWED.
    let result =
        classify_shell_exec_ro_safety("grep pattern /vaults-ro/x/notes/*.md", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "grep of an RO path must be allowed"
    );
}

#[test]
fn ro_safety_head_is_allowed() {
    // Case 3: `head -n 5 /vaults-ro/x/foo.md` → ALLOWED.
    let result = classify_shell_exec_ro_safety("head -n 5 /vaults-ro/x/foo.md", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "head of an RO path must be allowed"
    );
}

#[test]
fn ro_safety_cat_ro_as_input_redirect_target_is_allowed() {
    // Case 4: `cat /vaults-ro/x/foo.md > /tmp/out` → ALLOWED (RO is input).
    let result =
        classify_shell_exec_ro_safety("cat /vaults-ro/x/foo.md > /tmp/out", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "cat with RO as input and /tmp as redirect target must be allowed"
    );
}

#[test]
fn ro_safety_redirect_to_ro_is_blocked() {
    // Case 5: `cat /tmp/in > /vaults-ro/x/out.md` → BLOCKED (RO is redirect target).
    let result = classify_shell_exec_ro_safety("cat /tmp/in > /vaults-ro/x/out.md", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "redirect into RO path must be blocked"
    );
    if let RoSafety::Block(msg) = result {
        assert!(
            msg.contains("redirect"),
            "error must mention redirect; got: {msg}"
        );
    }
}

#[test]
fn ro_safety_cp_ro_as_dst_is_blocked() {
    // Case 6: `cp /tmp/foo /vaults-ro/x/bar` → BLOCKED (RO is destination).
    let result = classify_shell_exec_ro_safety("cp /tmp/foo /vaults-ro/x/bar", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "cp with RO as destination must be blocked"
    );
}

#[test]
fn ro_safety_cp_ro_as_src_is_allowed() {
    // Case 7: `cp /vaults-ro/x/foo /tmp/bar` → ALLOWED (RO is source).
    let result = classify_shell_exec_ro_safety("cp /vaults-ro/x/foo /tmp/bar", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "cp with RO as source must be allowed"
    );
}

#[test]
fn ro_safety_rm_is_blocked() {
    // Case 8: `rm /vaults-ro/x/foo` → BLOCKED.
    let result = classify_shell_exec_ro_safety("rm /vaults-ro/x/foo", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "rm of an RO path must be blocked"
    );
}

#[test]
fn ro_safety_sed_inplace_is_blocked() {
    // Case 9: `sed -i 's/x/y/' /vaults-ro/x/foo.md` → BLOCKED.
    let result =
        classify_shell_exec_ro_safety("sed -i 's/x/y/' /vaults-ro/x/foo.md", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "sed -i on an RO path must be blocked"
    );
}

#[test]
fn ro_safety_sed_n_is_allowed() {
    // Case 10: `sed -n '1,5p' /vaults-ro/x/foo.md` → ALLOWED.
    let result = classify_shell_exec_ro_safety("sed -n '1,5p' /vaults-ro/x/foo.md", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "sed -n (no-print, no-write) on an RO path must be allowed"
    );
}

#[test]
fn ro_safety_unrecognised_verb_is_blocked() {
    // Case 11: `weirdcmd /vaults-ro/x/foo` → BLOCKED (conservative default).
    let result = classify_shell_exec_ro_safety("weirdcmd /vaults-ro/x/foo", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "unrecognised verb referencing an RO path must be blocked"
    );
    if let RoSafety::Block(msg) = result {
        assert!(
            msg.contains("unrecognised"),
            "error must mention unrecognised verb; got: {msg}"
        );
    }
}

// --- Shell-chain bypass regressions (depth-2 verb check) -------------------
//
// Pre-fix the verb classifier only inspected the first whitespace token,
// so a leading READ_VERB ushered any number of trailing write commands
// through `sh -c`. These tests pin the fix.

#[test]
fn ro_safety_chained_write_after_read_is_blocked() {
    // `cat /tmp/in && rm /vaults-ro/x/data` — verb=`cat` (allowed), but
    // the second segment is `rm` against the RO path. Must Block.
    let result =
        classify_shell_exec_ro_safety("cat /tmp/in && rm /vaults-ro/x/data", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "chained write after read must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_pipe_of_reads_is_allowed() {
    // `cat /vaults-ro/x/foo | grep needle` — both segments are reads.
    let result =
        classify_shell_exec_ro_safety("cat /vaults-ro/x/foo | grep needle", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "pipe of two read verbs against an RO path must be allowed"
    );
}

#[test]
fn ro_safety_semicolon_then_touch_in_ro_is_blocked() {
    // `cat foo; touch /vaults-ro/x/marker` — `;` chains the touch write.
    let result = classify_shell_exec_ro_safety(
        "cat /vaults-ro/x/foo; touch /vaults-ro/x/marker",
        "/vaults-ro/x",
    );
    assert!(
        matches!(result, RoSafety::Block(_)),
        "`;`-chained `touch` under RO must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_command_substitution_is_blocked() {
    // `cat $(echo /vaults-ro/x/foo)` — sub-shell expansion is opaque to
    // the verb classifier; must fail-closed.
    let result = classify_shell_exec_ro_safety("cat $(echo /vaults-ro/x/foo)", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "`$(...)` substitution touching RO must be blocked; got {result:?}"
    );
    if let RoSafety::Block(msg) = result {
        assert!(
            msg.contains("command-substitution"),
            "error must mention substitution; got: {msg}"
        );
    }
}

#[test]
fn ro_safety_backtick_substitution_is_blocked() {
    // Same as the `$(...)` test but with the legacy backtick form.
    let cmd = "cat `echo /vaults-ro/x/foo`";
    let result = classify_shell_exec_ro_safety(cmd, "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "backtick substitution touching RO must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_unrelated_segment_does_not_false_positive() {
    // `cat /vaults-ro/x/foo && echo done` — second segment doesn't
    // touch the RO path, so an unrecognised verb on it must NOT cause
    // a deny when the RO-touching segment is itself safe.
    // (`echo` is intentionally not in READ_VERBS — see the unrecognised
    // verb test above. We rely on the "skip segments without RO ref"
    // fast path to keep this case allowed.)
    let result = classify_shell_exec_ro_safety("cat /vaults-ro/x/foo && echo done", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "unrelated trailing segment must not deny a safe read of RO"
    );
}

// ── BLOCKER-1: heredoc / extended redirect operators ──────────────────────

#[test]
fn ro_safety_heredoc_redirect_to_ro_is_blocked() {
    // `cat > /vaults-ro/x/out <<EOF` — the `>` redirect targets RO.
    let result =
        classify_shell_exec_ro_safety("cat > /vaults-ro/x/out <<EOF\ndata\nEOF", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "heredoc with `>` redirect to RO must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_noclobber_override_to_ro_is_blocked() {
    // `echo data >| /vaults-ro/x/foo` — `>|` overrides noclobber; must block.
    let result = classify_shell_exec_ro_safety("echo data >| /vaults-ro/x/foo", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "`>|` redirect to RO must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_fd_merge_redirect_to_ro_is_blocked() {
    // `cmd >& /vaults-ro/x/out` — bash fd-merge redirect; must block.
    let result = classify_shell_exec_ro_safety("cmd >& /vaults-ro/x/out", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "`>&` redirect to RO must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_fd1_redirect_to_ro_is_blocked() {
    // `cmd 1> /vaults-ro/x/out` — explicit stdout redirect; must block.
    let result = classify_shell_exec_ro_safety("cmd 1> /vaults-ro/x/out", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "`1>` redirect to RO must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_stderr_append_redirect_to_ro_is_blocked() {
    // `cmd 2>> /vaults-ro/x/err.log` — stderr append; must block.
    let result = classify_shell_exec_ro_safety("cmd 2>> /vaults-ro/x/err.log", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "`2>>` redirect to RO must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_both_append_redirect_to_ro_is_blocked() {
    // `cmd &>> /vaults-ro/x/out` — both stdout+stderr append; must block.
    let result = classify_shell_exec_ro_safety("cmd &>> /vaults-ro/x/out", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "`&>>` redirect to RO must be blocked; got {result:?}"
    );
}

// ── BLOCKER-2: quote-aware chain splitting ─────────────────────────────────

#[test]
fn ro_safety_operator_inside_single_quotes_not_split() {
    // `grep "rm /vaults-ro/x/data" /tmp/log` — the string literal contains
    // `/vaults-ro/x/data` but the verb is `grep` (a read); must be allowed.
    let result =
        classify_shell_exec_ro_safety("grep 'rm /vaults-ro/x/data' /tmp/log", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "operator inside single quotes must not trigger a split; got {result:?}"
    );
}

#[test]
fn ro_safety_semicolon_inside_double_quotes_not_split() {
    // Semicolon inside a double-quoted string must not split into two segments.
    let result =
        classify_shell_exec_ro_safety(r#"grep "pat;tern" /vaults-ro/x/foo"#, "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "semicolon inside double quotes must not be a chain separator; got {result:?}"
    );
}

#[test]
fn ro_safety_pipe_inside_single_quotes_not_split() {
    // `grep 'a|b' /vaults-ro/x/foo` — the `|` is inside quotes; single segment.
    let result = classify_shell_exec_ro_safety("grep 'a|b' /vaults-ro/x/foo", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "pipe inside single quotes must not split; got {result:?}"
    );
}

#[test]
fn ro_safety_real_chain_outside_quotes_still_splits() {
    // Operators outside quotes must still split: `cat /vaults-ro/x/f && rm /vaults-ro/x/f`.
    let result =
        classify_shell_exec_ro_safety("cat /vaults-ro/x/f && rm /vaults-ro/x/f", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "real unquoted `&&` must still split and detect the `rm`; got {result:?}"
    );
}

#[test]
fn ro_safety_escaped_operator_not_split() {
    // `grep foo\/bar /vaults-ro/x/log` — backslash before `/` is not an
    // operator; plain read should be allowed. (This exercises the Escape state.)
    let result = classify_shell_exec_ro_safety(r"grep foo\/bar /vaults-ro/x/log", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "backslash-escaped char must not trigger a false split; got {result:?}"
    );
}

// ── HIGH-1: find with write-enabling primaries ─────────────────────────────

#[test]
fn ro_safety_find_plain_is_allowed() {
    // `find /vaults-ro/x -name "*.md"` — pure read; must be allowed.
    let result = classify_shell_exec_ro_safety(r#"find /vaults-ro/x -name "*.md""#, "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "plain find without write primaries must be allowed; got {result:?}"
    );
}

#[test]
fn ro_safety_find_delete_is_blocked() {
    // `find /vaults-ro/x -name "*.tmp" -delete` — `-delete` is a write primary.
    let result =
        classify_shell_exec_ro_safety(r#"find /vaults-ro/x -name "*.tmp" -delete"#, "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "find -delete must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_find_exec_rm_is_blocked() {
    // `find /vaults-ro/x -type f -exec rm {} \;` — `-exec` is a write primary.
    let result =
        classify_shell_exec_ro_safety(r"find /vaults-ro/x -type f -exec rm {} \;", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "find -exec must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_find_execdir_is_blocked() {
    // `find /vaults-ro/x -execdir rm {} \;` — `-execdir` is also a write primary.
    let result =
        classify_shell_exec_ro_safety(r"find /vaults-ro/x -execdir rm {} \;", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "find -execdir must be blocked; got {result:?}"
    );
}

// ── HIGH-2: tee path-aware blocking ───────────────────────────────────────

#[test]
fn ro_safety_tee_to_tmp_is_allowed() {
    // `cat /vaults-ro/x/foo | tee /tmp/copy` — tee writes to /tmp, not RO.
    // (The pipe splits into two segments; the tee segment doesn't contain
    // the RO prefix so the fast-path skips it, and the cat segment is Allow.)
    let result =
        classify_shell_exec_ro_safety("cat /vaults-ro/x/foo | tee /tmp/copy", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "tee writing to /tmp (not RO) must be allowed; got {result:?}"
    );
}

#[test]
fn ro_safety_tee_to_ro_is_blocked() {
    // `cat /tmp/in | tee /vaults-ro/x/out` — tee target is inside RO; block.
    let result =
        classify_shell_exec_ro_safety("cat /tmp/in | tee /vaults-ro/x/out", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "tee writing to RO path must be blocked; got {result:?}"
    );
}

// ── HIGH-2: cp/mv -t <RO> GNU form ────────────────────────────────────────

#[test]
fn ro_safety_cp_t_ro_is_blocked() {
    // `cp -t /vaults-ro/x /tmp/foo` — GNU `-t` puts the target first.
    let result = classify_shell_exec_ro_safety("cp -t /vaults-ro/x /tmp/foo", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "cp -t <RO> must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_mv_t_ro_is_blocked() {
    // `mv -t /vaults-ro/x /tmp/foo` — GNU `-t` form.
    let result = classify_shell_exec_ro_safety("mv -t /vaults-ro/x /tmp/foo", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "mv -t <RO> must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_cp_target_directory_long_form_ro_is_blocked() {
    // `cp --target-directory=/vaults-ro/x /tmp/foo` — GNU long form.
    let result = classify_shell_exec_ro_safety(
        "cp --target-directory=/vaults-ro/x /tmp/foo",
        "/vaults-ro/x",
    );
    assert!(
        matches!(result, RoSafety::Block(_)),
        "cp --target-directory=<RO> must be blocked; got {result:?}"
    );
}

#[test]
fn ro_safety_cp_t_tmp_with_ro_src_is_allowed() {
    // `cp -t /tmp /vaults-ro/x/foo` — target is /tmp, source is RO; allow.
    let result = classify_shell_exec_ro_safety("cp -t /tmp /vaults-ro/x/foo", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "cp -t /tmp with RO as source must be allowed; got {result:?}"
    );
}

// ── Second-pass security regression tests (PR #4935 review) ─────────────

#[test]
fn at_boundary_handles_shared_prefix() {
    // B1: `/vaults-roxxx` (longer shared prefix) appears first in the
    // command; the real RO path `/vaults-ro/x` appears after a semicolon.
    // The boundary check must scan ALL occurrences, not just the first.
    // `classify_shell_exec_ro_safety` is called after the `at_boundary`
    // gate passes, so we test the gate directly via a command that
    // contains both the longer decoy prefix and the real one.
    //
    // The full command after the at_boundary gate is passed to the
    // classifier; the classifier must then block the `rm` segment.
    let result = classify_shell_exec_ro_safety(
        "cat /vaults-roxxx/dummy; rm /vaults-ro/x/foo",
        "/vaults-ro/x",
    );
    assert!(
        matches!(result, RoSafety::Block(_)),
        "shared-prefix decoy must not prevent detection of real RO path; got {result:?}"
    );
}

#[test]
fn find_ok_and_okdir_blocked() {
    // B2: `-ok` and `-okdir` are interactive variants of `-exec`/`-execdir`.
    // In non-interactive (AI agent) execution they silently run the command.
    let result_ok =
        classify_shell_exec_ro_safety(r"find /vaults-ro/x -ok rm {} \;", "/vaults-ro/x");
    assert!(
        matches!(result_ok, RoSafety::Block(_)),
        "find -ok must be blocked; got {result_ok:?}"
    );
    let result_okdir =
        classify_shell_exec_ro_safety(r"find /vaults-ro/x -okdir rm {} \;", "/vaults-ro/x");
    assert!(
        matches!(result_okdir, RoSafety::Block(_)),
        "find -okdir must be blocked; got {result_okdir:?}"
    );
}

#[test]
fn ansi_c_quoting_blocked() {
    // B3: `$'\x3b'` is ANSI-C quoting for `;`. The shell decodes this to
    // a real semicolon before execution, splitting the command. Our
    // tokenizer cannot safely decode these escapes, so it must fail-closed.
    let result =
        classify_shell_exec_ro_safety(r"cat /tmp/in $'\x3b' rm /vaults-ro/x/foo", "/vaults-ro/x");
    assert!(
        matches!(result, RoSafety::Block(_)),
        "ANSI-C quoting ($'...') must cause a block; got {result:?}"
    );
}

#[test]
fn redirect_in_single_quotes_allowed() {
    // H1: `grep '>' /vaults-ro/x/log` — the `>` is inside a single-quoted
    // grep pattern, not a shell redirect operator. The quote-aware redirect
    // scanner must not treat it as a write operation.
    let result = classify_shell_exec_ro_safety("grep '>' /vaults-ro/x/log", "/vaults-ro/x");
    assert_eq!(
        result,
        RoSafety::Allow,
        "redirect operator inside single quotes must not block a read; got {result:?}"
    );
}

#[tokio::test]
async fn test_web_search() {
    let result = execute_tool(
        "test-id",
        "web_search",
        &serde_json::json!({"query": "rust programming"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    // web_search now attempts a real fetch; may succeed or fail depending on network
    assert!(!result.tool_use_id.is_empty());
}

#[tokio::test]
async fn test_unknown_tool() {
    let result = execute_tool(
        "test-id",
        "nonexistent_tool",
        &serde_json::json!({}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("Unknown tool"));
}

#[tokio::test]
async fn test_agent_tools_without_kernel() {
    let result = execute_tool(
        "test-id",
        "agent_list",
        &serde_json::json!({}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("Kernel handle not available"));
}

#[tokio::test]
async fn test_capability_enforcement_denied() {
    let allowed = vec!["file_read".to_string(), "file_list".to_string()];
    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "ls"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("Permission denied"));
}

#[tokio::test]
async fn test_capability_enforcement_allowed() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let allowed = vec!["file_read".to_string()];
    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": "nonexistent_12345/file.txt"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    // Should fail for path resolution, NOT for permission denied
    assert!(
        result.is_error,
        "Expected error but got: {}",
        result.content
    );
    assert!(
        !result.content.contains("Permission denied"),
        "Unexpected permission denied: {}",
        result.content
    );
    assert!(
        result.content.contains("Failed to read")
            || result.content.contains("Failed to resolve")
            || result.content.contains("not found")
            || result.content.contains("No such file")
            || result.content.contains("does not exist"),
        "Expected file-not-found error, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_capability_enforcement_aliased_tool_name() {
    // Agent has "file_write" in allowed tools, but LLM calls "fs-write".
    // After normalization, this should pass the capability check.
    let workspace = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(workspace.path().join("output")).expect("create output dir");
    let allowed = vec![
        "file_read".to_string(),
        "file_write".to_string(),
        "file_list".to_string(),
        "shell_exec".to_string(),
    ];
    let result = execute_tool(
        "test-id",
        "fs-write", // LLM-hallucinated alias
        &serde_json::json!({"path": "output/file.txt", "content": "hello"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        !result.is_error,
        "fs-write should normalize to file_write and pass capability check, got: {}",
        result.content
    );
    assert!(workspace.path().join("output/file.txt").exists());
}

#[tokio::test]
async fn test_capability_enforcement_aliased_denied() {
    // Agent does NOT have file_write, and LLM calls "fs-write" — should be denied.
    let allowed = vec!["file_read".to_string()];
    let result = execute_tool(
        "test-id",
        "fs-write",
        &serde_json::json!({"path": "/tmp/test.txt", "content": "hello"}),
        None,
        Some(&allowed),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("Permission denied"),
        "fs-write should normalize to file_write which is not in allowed list"
    );
}

#[tokio::test]
async fn test_shell_exec_full_policy_skips_approval_gate() {
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::clone(&approval_requests),
        user_gate_override: None,
    });
    let policy = librefang_types::config::ExecPolicy {
        mode: librefang_types::config::ExecSecurityMode::Full,
        ..Default::default()
    };
    let workspace = tempfile::tempdir().expect("tempdir");

    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "echo ok"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        Some(&policy),
        None,
        None,
        None,
        None,
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;

    assert!(
        !result.content.contains("requires human approval"),
        "full exec policy should bypass approval gate, got: {}",
        result.content
    );
    assert_eq!(approval_requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_shell_exec_non_full_policy_still_requires_approval() {
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::clone(&approval_requests),
        user_gate_override: None,
    });
    let policy = librefang_types::config::ExecPolicy {
        mode: librefang_types::config::ExecSecurityMode::Allowlist,
        ..Default::default()
    };

    let result = execute_tool(
        "test-id",
        "shell_exec",
        &serde_json::json!({"command": "echo ok"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None, // media_engine
        None, // media_drivers
        Some(&policy),
        None,
        None,
        None,
        None,
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;

    // With non-blocking approval (Step 5), the tool is deferred rather than blocked.
    // The result should be WaitingApproval (not is_error) with the appropriate message.
    assert!(!result.is_error, "WaitingApproval should not be an error");
    assert!(
        result.content.contains("requires human approval"),
        "content should mention approval requirement, got: {}",
        result.content
    );
    assert_eq!(
        result.status,
        librefang_types::tool::ToolExecutionStatus::WaitingApproval
    );
    assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
}

/// Regression: shell_exec must NOT deadlock when a child writes more
/// stdout than the OS pipe buffer can hold. Container kernels often have
/// 8 KB pipe buffers; the previous implementation polled `try_wait()`
/// without draining stdout/stderr, so any child that exceeded the buffer
/// blocked on `write()` forever and only timed out at `timeout_secs`.
/// This test produces ~30 KB of stdout — well past 8 KB but well under
/// the 100 KB result cap — and asserts that it returns quickly with all
/// bytes preserved. Without the fix this test hangs the full
/// `timeout_secs` (set short here so a regression fails CI fast).
#[cfg(unix)]
#[tokio::test]
async fn test_shell_exec_drains_pipe_above_buffer_size() {
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::clone(&approval_requests),
        user_gate_override: None,
    });
    let policy = librefang_types::config::ExecPolicy {
        mode: librefang_types::config::ExecSecurityMode::Full,
        timeout_secs: 5, // Short timeout — regression hangs at this value.
        ..Default::default()
    };
    let workspace = tempfile::tempdir().expect("tempdir");

    let started = std::time::Instant::now();
    let result = execute_tool(
        "test-id",
        "shell_exec",
        // ~30 KB of stdout — 4× the typical 8 KB container pipe buffer.
        &serde_json::json!({"command": "yes hello | head -c 30000"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        Some(&policy),
        None,
        None,
        None,
        None,
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        !result.is_error,
        "shell_exec must not error on >pipe-buffer output, got: {}",
        result.content
    );
    assert!(
        !result.content.contains("timed out"),
        "shell_exec must not time out on drainable output, got: {}",
        result.content
    );
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "shell_exec finished in {elapsed:?}; expected sub-second on drainable output. Pipe drain regression?"
    );
    // 30000 'hello' chars (5 bytes + newline = 6 each), so output count
    // is in the same order — assert > buffer size, < truncation cap.
    let len = result.content.len();
    assert!(
        len > 8_000 && len < 100_000,
        "expected output between 8 KB and 100 KB, got {len} bytes"
    );
}
