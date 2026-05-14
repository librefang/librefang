//! Read-only workspace enforcement for `shell_exec` (fix #4903).
//!
//! The original implementation blocked any command whose argv contained a
//! read-only workspace path, regardless of whether the command could actually
//! write to it. That produced false-positives for clear reads like
//! `cat /vaults-ro/x/foo.md`.
//!
//! This module adds argument-role awareness:
//!   • Known read verbs (cat, less, grep, …) are unconditionally allowed to
//!     reference RO paths.
//!   • Known write verbs (rm, cp-as-dst, mv-as-dst, touch, mkdir, editors,
//!     sed -i, awk -i inplace) are blocked when the RO path appears in a write
//!     position.
//!   • Shell output redirects (>, >>, &>, 2>, 1>, 2>>, &>>, >|, >&, <<, <<<,
//!     >(…) process substitution) targeting RO paths are blocked regardless of
//!     the leading verb.
//!   • If the verb is unrecognised the old conservative behaviour is kept
//!     (deny) to avoid weakening the security posture.

/// Classification outcome from [`classify_shell_exec_ro_safety`].
#[derive(Debug, PartialEq)]
pub(super) enum RoSafety {
    /// The command is safe to run — it only reads from the RO path.
    Allow,
    /// The command must be blocked. The string is the human-readable reason.
    Block(String),
}

// ── Shell tokenizer (quote-aware) ────────────────────────────────────────────
//
// Splits a shell command into operator-separated fragments while honouring
// single-quotes, double-quotes, backslash escapes, and `$(…)` nesting so that
// operators embedded inside string literals are NOT treated as real operators.
//
// State machine:
//   Normal    → sees `'`  → SingleQuote (consume until matching `'`)
//             → sees `"`  → DoubleQuote (consume until matching `"`, honour `\`)
//             → sees `\`  → Escape (skip one byte)
//             → sees `$(` → return Err immediately (opaque subshell, fail-closed)
//             → sees `` ` `` → return Err immediately (opaque subshell, fail-closed)
//             → sees one of the CHAIN_OPS → emit current fragment, continue
//   SingleQuote → sees `'` → Normal (no escapes inside '')
//   DoubleQuote → sees `"` → Normal; sees `\` → skip one byte
//   Escape    → skip one byte → Normal
//
// Only operates on ASCII/UTF-8 byte sequences that shell parsers accept.
// This is intentionally minimal — enough to avoid the most common quoted-
// operator bypasses without reimplementing a full POSIX parser.

#[derive(Clone, Copy, PartialEq)]
enum TokenizerState {
    Normal,
    SingleQuote,
    DoubleQuote,
    Escape,   // after `\` in Normal
    DqEscape, // after `\` inside double-quote
}

/// Split `command` on unquoted shell chain operators (`&&`, `||`, `|`, `;`).
/// Returns `Err(reason)` if an unquoted `$(` or backtick is encountered —
/// those are opaque sub-commands that the caller must fail-closed on.
fn shell_split_chain(command: &str) -> Result<Vec<String>, String> {
    let mut fragments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut state = TokenizerState::Normal;
    let chars: Vec<char> = command.chars().collect();
    let len = chars.len();
    let mut i = 0usize;

    while i < len {
        let ch = chars[i];

        match state {
            TokenizerState::Escape => {
                // Skip the escaped character; return to Normal.
                current.push(ch);
                state = TokenizerState::Normal;
                i += 1;
            }
            TokenizerState::DqEscape => {
                current.push(ch);
                state = TokenizerState::DoubleQuote;
                i += 1;
            }
            TokenizerState::SingleQuote => {
                if ch == '\'' {
                    state = TokenizerState::Normal;
                }
                current.push(ch);
                i += 1;
            }
            TokenizerState::DoubleQuote => {
                if ch == '"' {
                    state = TokenizerState::Normal;
                } else if ch == '\\' {
                    state = TokenizerState::DqEscape;
                }
                current.push(ch);
                i += 1;
            }
            TokenizerState::Normal => {
                // Backtick: opaque subshell.
                if ch == '`' {
                    return Err("backtick subshell".to_string());
                }
                // Single-quote start.
                if ch == '\'' {
                    state = TokenizerState::SingleQuote;
                    current.push(ch);
                    i += 1;
                    continue;
                }
                // Double-quote start.
                if ch == '"' {
                    state = TokenizerState::DoubleQuote;
                    current.push(ch);
                    i += 1;
                    continue;
                }
                // Backslash escape.
                if ch == '\\' {
                    state = TokenizerState::Escape;
                    current.push(ch);
                    i += 1;
                    continue;
                }
                // `$(` — command substitution: opaque subshell.
                // `$'...'` — ANSI-C quoting: shell decodes escape sequences like
                // `$'\x3b'` → `;` at parse time, before we see the string.  We
                // cannot safely tokenize through that, so fail-closed (B3).
                if ch == '$' && i + 1 < len {
                    if chars[i + 1] == '(' {
                        return Err("$(...) command-substitution".to_string());
                    }
                    if chars[i + 1] == '\'' {
                        return Err(
                            "ANSI-C quoting ($'...') contains shell-decoded escapes".to_string()
                        );
                    }
                }
                // `&&` operator.
                if ch == '&' && i + 1 < len && chars[i + 1] == '&' {
                    fragments.push(current.clone());
                    current.clear();
                    i += 2;
                    continue;
                }
                // `||` operator.
                if ch == '|' && i + 1 < len && chars[i + 1] == '|' {
                    fragments.push(current.clone());
                    current.clear();
                    i += 2;
                    continue;
                }
                // `|` operator (single pipe, not `||`).
                if ch == '|' {
                    fragments.push(current.clone());
                    current.clear();
                    i += 1;
                    continue;
                }
                // `;` operator.
                if ch == ';' {
                    fragments.push(current.clone());
                    current.clear();
                    i += 1;
                    continue;
                }
                current.push(ch);
                i += 1;
            }
        }
    }

    fragments.push(current);
    Ok(fragments)
}

/// Determine whether a shell command is safe to execute when `ro_prefix` is a
/// read-only workspace path that appears somewhere in the command string.
///
/// Design choices:
/// - We use a quote-aware tokenizer to split on `&&`/`||`/`|`/`;` so that
///   operators embedded inside string literals are not treated as chain
///   operators (BLOCKER-2).
/// - Redirect detection covers the full set of POSIX + bash output-redirect
///   operators including `<<`/`<<<` (heredoc), `>|`, `>&`, `1>`, `2>>`,
///   `&>>`, and process-substitution `>(` (BLOCKER-1).
/// - For `cp` and `mv` the `-t <dir>` GNU form is checked in addition to the
///   last positional argument (HIGH-2).
/// - For `tee` only arguments that are inside the RO prefix are blocked, not
///   any invocation of `tee` (HIGH-2).
/// - For `find` with write-enabling primaries (`-delete`, `-exec`, etc.) the
///   command is rejected as a write op (HIGH-1).
///
/// # SAFETY
/// Verb classification trusts $PATH resolution and is NOT a security boundary
/// against malicious workspaces. A workspace containing an executable named
/// `cat` (or any other READ_VERB name) could run arbitrary code. Sandboxing
/// is provided by the RO workspace *mount* enforcement at the kernel layer and
/// by the OS filesystem permissions. This classifier's sole purpose is to
/// reduce false-positive blocks for legitimate read commands issued by trusted
/// agents — it is not designed to stop a determined attacker who controls the
/// workspace filesystem.
pub(super) fn classify_shell_exec_ro_safety(command: &str, ro_prefix: &str) -> RoSafety {
    // --- 0. Quote-aware shell-chain split ------------------------------------
    // The tokenizer (`shell_split_chain`) already fails-closed on:
    //   - backtick subshells
    //   - `$(...)` command substitution
    //   - `$'...'` ANSI-C quoting (B3 — decodes shell escapes before we see them)
    // The old raw `contains("$(")` fast-path was removed (M1): it blocked
    // legitimate reads like `grep '$(foo)' /vaults-ro/x/log` where `$(` is
    // inside a single-quoted argument.  The tokenizer handles the real unsafe
    // cases correctly and without false positives.
    //
    // Split on unquoted `&&`, `||`, `|`, `;` so that operators inside string
    // literals are not mistaken for chain operators (BLOCKER-2).
    match shell_split_chain(command) {
        Err(reason) => {
            return RoSafety::Block(format!(
                "shell_exec blocked: {reason} is not analyzable — \
                 RO path '{ro_prefix}' may be targeted by an embedded sub-command"
            ));
        }
        Ok(fragments) if fragments.len() > 1 => {
            for fragment in &fragments {
                let trimmed = fragment.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Segments that don't reference the RO path can never harm this
                // RO mount; skip them so an unrecognised verb on an unrelated
                // segment doesn't cause a false-positive deny.
                if !trimmed.contains(ro_prefix) {
                    continue;
                }
                if let RoSafety::Block(reason) =
                    classify_shell_exec_ro_safety_segment(trimmed, ro_prefix)
                {
                    return RoSafety::Block(reason);
                }
            }
            return RoSafety::Allow;
        }
        Ok(_) => {} // single fragment — fall through to segment classifier
    }

    classify_shell_exec_ro_safety_segment(command, ro_prefix)
}

/// Per-segment RO classification — the original single-verb analysis.
/// Called either directly (no shell chain) or for each fragment after
/// `classify_shell_exec_ro_safety` has split the command on `&&` / `||` /
/// `;` / `|`. Subshells / command-substitution are rejected before reaching
/// here, so this function can assume `command` is a single simple command.
fn classify_shell_exec_ro_safety_segment(command: &str, ro_prefix: &str) -> RoSafety {
    // --- 1. Redirect detection (quote-aware) ------------------------------------
    // Walk the command character-by-character tracking quote state so that
    // redirect operators that appear inside single- or double-quoted strings
    // are NOT treated as real redirects (H1).
    //
    // Example false-positive that the old raw `.find()` approach triggered:
    //   `grep '>' /vaults-ro/x/log`
    // The `>` is inside single quotes and is part of the grep pattern, not a
    // redirect — but the old scan found it and blocked the legitimate read.
    //
    // Operators covered (longer before shorter to avoid prefix shadowing):
    //   &>>  2>>  >>   >|   >&   &>   2>   1>   >    (output redirects)
    //   <<<  <<                                        (heredoc / herestring)
    //   >(                                             (bash process substitution)
    //
    // For heredoc / herestring the *following token* is the delimiter word, not
    // a path — we only block if the RO prefix appears after the operator token.
    {
        // Build a parallel byte-offset → in-Normal-state index so we can check
        // quote state at each candidate operator position.  We track quote state
        // over the raw bytes (ASCII operators only, so char == byte here).
        let ops: &[&str] = &[
            "&>>", "2>>", ">>", ">|", ">&", "&>", "2>", "1>", ">", "<<<", "<<", ">(",
        ];
        // Compute quote state at every byte offset using a simple state machine.
        // `true` = this position is in Normal (unquoted) state.
        let bytes = command.as_bytes();
        let n = bytes.len();
        let mut normal_at: Vec<bool> = vec![false; n + 1];
        {
            let mut sq = false; // inside single-quote
            let mut dq = false; // inside double-quote
            let mut esc = false; // backslash-escape active
            for (idx, &b) in bytes.iter().enumerate() {
                normal_at[idx] = !sq && !dq && !esc;
                if esc {
                    esc = false;
                } else if sq {
                    if b == b'\'' {
                        sq = false;
                    }
                } else if dq {
                    if b == b'\\' {
                        esc = true;
                    } else if b == b'"' {
                        dq = false;
                    }
                } else {
                    // Normal state
                    if b == b'\\' {
                        esc = true;
                    } else if b == b'\'' {
                        sq = true;
                    } else if b == b'"' {
                        dq = true;
                    }
                }
            }
            normal_at[n] = !sq && !dq && !esc;
        }

        for op in ops {
            let op_len = op.len();
            let mut search_from = 0usize;
            while search_from + op_len <= n {
                if let Some(rel) = command[search_from..].find(op) {
                    let op_start = search_from + rel;
                    // Only treat as a real redirect if the operator starts in
                    // Normal (unquoted) state (H1).
                    if normal_at[op_start] {
                        let after_op = command[op_start + op_len..].trim_start();
                        let dest_token = after_op.split_whitespace().next().unwrap_or("");
                        if is_ro_path(dest_token, ro_prefix) {
                            return RoSafety::Block(format!(
                                "shell_exec blocked: shell redirect '{}' targets \
                                 read-only workspace path '{}'",
                                op, ro_prefix
                            ));
                        }
                    }
                    search_from = op_start + op_len;
                } else {
                    break;
                }
            }
        } // for op in ops
    } // quote-aware redirect scan block

    // --- 2. Split into tokens for verb + arg analysis --------------------------
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let verb = match tokens.first() {
        Some(v) => *v,
        None => return RoSafety::Allow, // empty command
    };
    // Strip any leading path component (e.g. /usr/bin/cat → cat).
    //
    // SAFETY: See the function-level SAFETY note on `classify_shell_exec_ro_safety`.
    // Verb classification trusts $PATH resolution and is NOT a security boundary
    // against malicious workspaces. Sandboxing is provided by RO workspace mount
    // enforcement at the kernel layer.
    let verb_base = verb.rsplit('/').next().unwrap_or(verb);

    // --- 3. Known pure-read verbs -----------------------------------------------
    // These commands cannot write files when invoked normally.
    // `sed` and `awk` have write-enabling flags handled below.
    // `find` has write-enabling primaries handled below (HIGH-1).
    // NOTE: `xargs` is intentionally NOT in this list — `xargs rm <path>`
    // would bypass the gate entirely. Falls through to the conservative
    // "unrecognised verb → deny" branch.
    const READ_VERBS: &[&str] = &[
        "cat", "less", "more", "head", "tail", "grep", "egrep", "fgrep", "rg", "wc", "diff", "cmp",
        "file", "stat", "du", "ls", "zcat", "zless",
    ];
    if READ_VERBS.contains(&verb_base) {
        return RoSafety::Allow;
    }

    // --- 3b. find: allowed as a read verb UNLESS write-enabling primaries are
    //         present (HIGH-1).
    if verb_base == "find" {
        // These primaries instruct find to mutate the filesystem or write to
        // a file, making `find` a write operation even if it looks like a read.
        const FIND_WRITE_PRIMARIES: &[&str] = &[
            "-delete", "-exec", "-execdir",
            // `-ok` / `-okdir` are interactive variants of `-exec` / `-execdir`.
            // In non-interactive (AI agent) execution they silently run the
            // command, so they must be treated as write-enabling primaries (B2).
            "-ok", "-okdir", "-fprint", "-fprintf", "-fls", "-fprint0",
        ];
        let has_write_primary = tokens[1..].iter().any(|t| {
            // Match the primary exactly or when it's a prefix of a combined token
            // like `-exec{}` (unusual but technically valid).
            FIND_WRITE_PRIMARIES
                .iter()
                .any(|p| t == p || t.starts_with(p))
        });
        if has_write_primary {
            return RoSafety::Block(format!(
                "shell_exec blocked: 'find' with a write-enabling primary \
                 (e.g. -delete, -exec) targets read-only workspace path '{}'",
                ro_prefix
            ));
        }
        return RoSafety::Allow;
    }

    // --- 4. sed: allow `-n` (no-print), block `-i` (in-place edit) -------------
    if verb_base == "sed" {
        let has_inplace = tokens.iter().any(|t| {
            // `-i` alone (POSIX) or `-i<suffix>` (GNU sed extension).
            if *t == "-i" || (t.starts_with("-i") && t.len() > 2) {
                return true;
            }
            // Combined short flags e.g. `-ni` — only short-option bundles, not
            // long options (which start with `--`).
            if t.starts_with('-') && !t.starts_with("--") && t.contains('i') {
                return true;
            }
            false
        });
        if has_inplace {
            return RoSafety::Block(format!(
                "shell_exec blocked: 'sed -i' (in-place edit) targets read-only workspace path '{}'",
                ro_prefix
            ));
        }
        return RoSafety::Allow;
    }

    // --- 5. awk: block `-i inplace` (GNU awk) -----------------------------------
    if verb_base == "awk" {
        let mut iter = tokens.iter().peekable();
        let mut has_inplace = false;
        while let Some(tok) = iter.next() {
            if *tok == "-i" && iter.peek().map(|s| **s) == Some("inplace") {
                has_inplace = true;
                break;
            }
            // Also catch --inplace long form.
            if *tok == "--inplace" {
                has_inplace = true;
                break;
            }
        }
        if has_inplace {
            return RoSafety::Block(format!(
                "shell_exec blocked: 'awk -i inplace' (in-place edit) targets read-only workspace path '{}'",
                ro_prefix
            ));
        }
        return RoSafety::Allow;
    }

    // --- 6. Known write verbs ---------------------------------------------------

    // rm: any argument under the RO path is a write (deletion).
    if verb_base == "rm" {
        return RoSafety::Block(format!(
            "shell_exec blocked: 'rm' targets read-only workspace path '{}'",
            ro_prefix
        ));
    }

    // mkdir / touch: creating or touching files under RO path.
    if verb_base == "mkdir" || verb_base == "touch" {
        return RoSafety::Block(format!(
            "shell_exec blocked: '{}' targets read-only workspace path '{}'",
            verb_base, ro_prefix
        ));
    }

    // Editors: always block when RO path is mentioned.
    const EDITOR_VERBS: &[&str] = &["vi", "vim", "nvim", "nano", "emacs", "code", "gedit"];
    if EDITOR_VERBS.contains(&verb_base) {
        return RoSafety::Block(format!(
            "shell_exec blocked: editor '{}' targets read-only workspace path '{}'",
            verb_base, ro_prefix
        ));
    }

    // tee: block only when one of tee's *target* arguments is inside the RO
    // path. `tee /tmp/out` is fine even if the RO prefix appears elsewhere
    // in the command. (HIGH-2)
    if verb_base == "tee" {
        // tee flags: -a / --append, -i / --ignore-interrupts, -p / --output-error.
        // All other tokens are output file paths.
        let writes_to_ro = tokens[1..].iter().any(|t| {
            if t.starts_with('-') {
                return false; // flag, not a path
            }
            is_ro_path(t, ro_prefix)
        });
        if writes_to_ro {
            return RoSafety::Block(format!(
                "shell_exec blocked: 'tee' would write to read-only workspace path '{}'",
                ro_prefix
            ));
        }
        return RoSafety::Allow;
    }

    // cp / mv: block only when the RO path is the *destination*.
    // Destination is either:
    //   (a) the argument to the GNU `-t`/`--target-directory` flag, OR
    //   (b) the last positional argument (POSIX form) — only when `-t` is absent.
    // When `-t` is present the destination is fully determined by that flag;
    // the remaining positional args are all sources, so we skip the
    // last-positional check in that case (HIGH-2).
    if verb_base == "cp" || verb_base == "mv" {
        // Check GNU `-t <dir>` / `--target-directory=<dir>` form first.
        let mut explicit_target: Option<&str> = None;
        {
            let mut t_iter = tokens[1..].iter().peekable();
            while let Some(tok) = t_iter.next() {
                if *tok == "-t" || *tok == "--target-directory" {
                    if let Some(target) = t_iter.next() {
                        explicit_target = Some(target);
                        if is_ro_path(target, ro_prefix) {
                            return RoSafety::Block(format!(
                                "shell_exec blocked: '{}' -t destination '{}' is inside read-only workspace path '{}'",
                                verb_base, target, ro_prefix
                            ));
                        }
                    }
                } else if let Some(val) = tok.strip_prefix("--target-directory=") {
                    explicit_target = Some(val);
                    if is_ro_path(val, ro_prefix) {
                        return RoSafety::Block(format!(
                            "shell_exec blocked: '{}' --target-directory destination '{}' is inside read-only workspace path '{}'",
                            verb_base, val, ro_prefix
                        ));
                    }
                }
            }
        }

        if explicit_target.is_none() {
            // No `-t` flag: fall back to last positional argument as destination.
            let positional: Vec<&str> = tokens[1..]
                .iter()
                .filter(|t| !t.starts_with('-'))
                .copied()
                .collect();
            if let Some(dst) = positional.last() {
                if is_ro_path(dst, ro_prefix) {
                    return RoSafety::Block(format!(
                        "shell_exec blocked: '{}' destination '{}' is inside read-only workspace path '{}'",
                        verb_base, dst, ro_prefix
                    ));
                }
            }
        }
        // RO path appears only as a source — allow.
        return RoSafety::Allow;
    }

    // --- 7. Unrecognised verb: conservative deny --------------------------------
    // We don't know whether this command writes; keep the original strict behaviour.
    RoSafety::Block(format!(
        "shell_exec blocked: unrecognised command verb '{}' — RO path '{}' may be a write target. \
         Only known read-only verbs are permitted to reference read-only workspace paths.",
        verb_base, ro_prefix
    ))
}

/// Returns true if `token` refers to a path that starts with `ro_prefix` at
/// a path boundary (i.e. the token equals the prefix or has a '/' after it).
fn is_ro_path(token: &str, ro_prefix: &str) -> bool {
    // Strip surrounding quotes that the shell would have consumed.
    let token = token.trim_matches(|c| c == '"' || c == '\'');
    if let Some(rest) = token.strip_prefix(ro_prefix) {
        rest.is_empty() || rest.starts_with('/')
    } else {
        false
    }
}
