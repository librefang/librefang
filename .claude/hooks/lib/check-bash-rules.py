#!/usr/bin/env python3
"""Shared rule-checker for the librefang Claude Code hooks.

Tokenizes a Bash command with shlex (quote-aware) and runs the requested rule.
Returns the violation message on stdout, empty stdout on no violation.
Always exits 0 — the calling shell hook decides what to do with the message.

Why shlex? Earlier versions used `grep -qE` regexes against the raw command
string. That conflated *real* command invocations with *string content* —
for example, a commit message body containing the literal text
"... use cargo check ...; cargo test is a HUMAN workflow ..." matched the
"cargo test" regex even though no cargo invocation existed. shlex respects
quotes, so any text inside `-m "..."` arrives as a single token and we can
skip it when looking for command-position tokens.

Usage (called from the hook shell scripts):

    msg="$(printf '%s' "$cmd" | python3 path/to/check-bash-rules.py \\
        --rule <rule-name> [--cwd ...] [--main-root ...] [--kind main|worktree])"
    if [ -n "$msg" ]; then ... refuse ... fi

Available rules:

  cargo-build-run           -> banned anywhere in the librefang repo
  cargo-test-unscoped       -> banned (allow `cargo test -p <crate>` only)
  worktree-remove-main      -> banned (`git worktree remove/move` of main path)
  git-mutation-main         -> banned (when kind=main)
  sed-i-main / perl-pi-main -> banned (when kind=main)
  redirect-into-main        -> banned (when kind=main)
  force-push-main           -> banned
  no-verify                 -> banned
  broad-git-add             -> banned (`git add -A` / `git add .`)
  sensitive-file-add        -> banned (.env, *.pem, id_rsa, …)
  claude-attribution        -> banned (-m / --message containing Co-Authored-By: Claude etc.)
  rm-rf-dangerous           -> banned (rm -rf against /, ~, target, .git, …)
  librefang-daemon-launch   -> banned (`librefang start`, target/release/librefang start)
  gh-pr-merge               -> banned (publish-level)
"""

from __future__ import annotations

import argparse
import os
import re
import shlex
import sys


# -----------------------------------------------------------------------------
# Tokenization
# -----------------------------------------------------------------------------

SKIP_AFTER_FLAGS = {"-m", "--message", "-F", "--file", "-c", "-C"}

# Operators shlex returns as their own tokens (sometimes glued to surrounding
# text). We split tokens on these to recover command-position info.
SHELL_OPS = ("&&", "||", ";;", ";", "|", "&", "(", ")")


def tokenize(cmd: str) -> list[str]:
    """shlex.split with a fallback so a malformed quote can't crash the hook."""
    try:
        return shlex.split(cmd, posix=True)
    except ValueError:
        return cmd.split()


def is_quoted_content(t: str) -> bool:
    """A token containing whitespace came from inside a quoted string in the
    original command; shlex would otherwise have split it."""
    return any(c in t for c in " \t\n\r")


def strip_parens(t: str) -> str:
    """Bash subshells like `(cargo build)` keep the parens glued in shlex
    tokens. Strip them so equality checks work."""
    return t.lstrip("(").rstrip(")")


def at_command_position(toks: list[str], i: int) -> bool:
    """Return True if the token at index i sits at the start of a command —
    either at index 0, after a shell operator, or after a paren."""
    if i == 0:
        return True
    p = toks[i - 1]
    if p in SHELL_OPS:
        return True
    if p in SKIP_AFTER_FLAGS:
        return False
    return False


def has_p_flag(rest: list[str]) -> bool:
    """`cargo test` is scoped if any of -p / --package / -p<crate> /
    --package=<crate> appears in the trailing args."""
    for x in rest:
        if x in ("-p", "--package"):
            return True
        if x.startswith("--package="):
            return True
        if x.startswith("-p") and len(x) > 2 and not x[2:].startswith("-"):
            return True
    return False


# -----------------------------------------------------------------------------
# Helpers shared across rules
# -----------------------------------------------------------------------------


def find_cargo_subcommand(toks: list[str], wanted_subs: set[str]):
    """Walk tokens looking for a real `cargo <sub>` invocation where <sub> is
    in wanted_subs. Returns (sub, index_of_sub) or None."""
    for i, t in enumerate(toks):
        if is_quoted_content(t):
            continue
        prev = toks[i - 1] if i > 0 else None
        if prev in SKIP_AFTER_FLAGS:
            continue
        if strip_parens(t) != "cargo":
            continue
        if i + 1 >= len(toks):
            continue
        sub = strip_parens(toks[i + 1])
        if sub in wanted_subs:
            return sub, i + 1
    return None


def walk_git_invocations(toks: list[str]):
    """Yield (i_git, j_after_C, c_path) for each `git [-C path] ...` we find
    where i_git is the index of the `git` token, j_after_C is the index of the
    first non-`-C` argument, and c_path is the value passed via `-C` (or None
    if no `-C` was used). Skips occurrences inside quoted content."""
    i = 0
    while i < len(toks):
        if is_quoted_content(toks[i]):
            i += 1
            continue
        if strip_parens(toks[i]) != "git":
            i += 1
            continue
        # Optional -C <path>
        j = i + 1
        c_path = None
        if j < len(toks) and toks[j] == "-C":
            if j + 1 < len(toks):
                c_path = toks[j + 1]
            j += 2
        yield i, j, c_path
        i = j + 1


# -----------------------------------------------------------------------------
# Rule implementations
# -----------------------------------------------------------------------------


def rule_cargo_build_run(toks, ctx):
    hit = find_cargo_subcommand(toks, {"build", "run"})
    if hit:
        sub = hit[0]
        return (
            f"`cargo {sub}` is banned in this repo (target/ is shared across "
            f"worktrees and contends with the user's other sessions). Use "
            f"`cargo check`; CI handles full build."
        )
    return None


def rule_cargo_test_unscoped(toks, ctx):
    hit = find_cargo_subcommand(toks, {"test"})
    if not hit:
        return None
    _sub, sub_idx = hit
    if has_p_flag(toks[sub_idx + 1 :]):
        return None
    return (
        "Unscoped `cargo test` builds and runs the whole workspace, which is "
        "too slow / target-contending for the AI to invoke. Re-run with "
        "`-p <crate>` (or `--package <crate>`)."
    )


def rule_worktree_remove_main(toks, ctx):
    """git [-C base] worktree (remove|move) <path> where <path> resolves to
    main_root."""
    main_root = ctx.get("main_root", "")
    cwd = ctx.get("cwd", "")
    if not main_root:
        return None
    real_main = os.path.realpath(main_root)

    for i_git, j, c_path in walk_git_invocations(toks):
        if j >= len(toks):
            continue
        if strip_parens(toks[j]) != "worktree":
            continue
        if j + 1 >= len(toks):
            continue
        sub = strip_parens(toks[j + 1])
        if sub not in ("remove", "move"):
            continue
        rest = toks[j + 2 :]
        positionals = [x for x in rest if not x.startswith("-")]
        if not positionals:
            continue
        target = positionals[0]
        base = c_path or cwd
        if not target.startswith("/") and base:
            target = os.path.join(base, target)
        try:
            target = os.path.realpath(target)
        except OSError:
            continue
        if target == real_main:
            return (
                f"`git worktree {sub}` targeting the MAIN worktree itself "
                f"({main_root}). That would destroy the user's main checkout. "
                f"If this is really what you want, ask the user to do it."
            )
    return None


GIT_DIRECT_MUTATIONS = {
    "checkout", "switch", "merge", "rebase", "reset", "commit", "push",
    "pull", "cherry-pick", "revert", "am", "apply", "clean",
}
GIT_BRANCH_FORCE_FLAGS = {"-D", "-d", "-m", "--delete", "--force"}
GIT_STASH_MUTATIONS = {"pop", "apply", "drop", "clear"}
GIT_WORKTREE_MUTATIONS = {"remove", "prune", "move"}
GIT_TAG_DELETE_FLAGS = {"-d", "--delete"}


def rule_git_mutation_main(toks, ctx):
    """When kind=main, refuse any modifying git invocation that would touch
    the main worktree's tree, HEAD, or stash. Worktree-cleanup commands
    (`git worktree remove|prune|move` against linked trees, `git branch
    -D <feature>` for spent branches) are allowed: removing a linked
    worktree doesn't affect main, and git itself refuses to delete a
    checked-out branch. The dangerous case (removing main itself) is
    caught by `rule_worktree_remove_main`."""
    if ctx.get("kind") != "main":
        return None

    for i_git, j, c_path in walk_git_invocations(toks):
        if j >= len(toks):
            continue
        sub = strip_parens(toks[j])
        sub_arg = strip_parens(toks[j + 1]) if j + 1 < len(toks) else None
        if sub in GIT_DIRECT_MUTATIONS:
            return f"`git {sub}` in main worktree."
        if sub == "stash" and sub_arg in GIT_STASH_MUTATIONS:
            return f"`git stash {sub_arg}` in main worktree."
        if sub == "tag" and sub_arg in GIT_TAG_DELETE_FLAGS:
            return f"`git tag {sub_arg}` in main worktree."
    return None


def rule_sed_i_perl_pi_main(toks, ctx):
    if ctx.get("kind") != "main":
        return None
    for i, t in enumerate(toks):
        if is_quoted_content(t):
            continue
        c = strip_parens(t)
        if c == "sed":
            for x in toks[i + 1 :]:
                if is_quoted_content(x):
                    break
                cx = strip_parens(x)
                if cx.startswith("-") and not cx.startswith("--") and "i" in cx[1:]:
                    return "`sed -i` in main worktree."
                if not cx.startswith("-"):
                    break
        if c == "perl":
            for x in toks[i + 1 :]:
                if is_quoted_content(x):
                    break
                cx = strip_parens(x)
                if cx.startswith("-") and "p" in cx and "i" in cx:
                    return "`perl -pi` in main worktree."
                if not cx.startswith("-"):
                    break
    return None


def rule_redirect_into_main(toks, ctx):
    """Detect `>` / `>>` redirects whose target lands in the main worktree.

    shlex tokenization gives us quote-awareness for free: `>` inside a quoted
    string (e.g. inside a commit message) becomes part of the surrounding
    token rather than a standalone redirect operator, so we just scan the
    tokens for real shell-level redirects.
    """
    if ctx.get("kind") != "main":
        return None
    main_root = ctx.get("main_root", "")
    if not main_root:
        return None
    real_repo = os.path.realpath(main_root)

    for i, t in enumerate(toks):
        target = None
        if t in (">", ">>"):
            if i + 1 < len(toks) and not is_quoted_content(toks[i + 1]):
                target = toks[i + 1]
        elif re.match(r"^>>?[^&]", t):
            # Glued form: `>foo`, `>>foo`. Excludes `>&1`, `>&-`, etc.
            target = t[2:] if t.startswith(">>") else t[1:]
        if not target:
            continue
        if target.startswith("&") or target in ("/dev/null", "/dev/stderr", "/dev/stdout"):
            continue
        if target.startswith("/"):
            ap = os.path.realpath(target)
        else:
            ap = os.path.realpath(os.path.join(real_repo, target))
        if ap == real_repo or ap.startswith(real_repo + "/"):
            return f"Shell write redirect into main worktree (target: {ap})."
    return None


def rule_force_push_main(toks, ctx):
    """git [-C ...] push ... (-f|--force|--force-with-lease) ... main|master,
    or `+main`/`+master` refspec."""
    for i_git, j, c_path in walk_git_invocations(toks):
        # Find `push` somewhere in the rest of this git invocation. We treat
        # the rest of the tokens (until next ;|&||| etc) as args; shlex doesn't
        # know about operators, so we walk until end of toks. False matches
        # are unlikely because we only walk after a literal `git` token.
        rest = toks[j:]
        if not any(strip_parens(x) == "push" for x in rest):
            continue
        has_force = any(
            x in ("-f", "--force", "--force-with-lease")
            or x.startswith("--force=")
            for x in rest
        )
        targets_main = False
        for x in rest:
            cx = strip_parens(x)
            if cx in ("main", "master"):
                targets_main = True
                break
            if cx.startswith("+"):
                bare = cx.lstrip("+")
                if bare in ("main", "master", "HEAD"):
                    targets_main = True
                    has_force = True
                    break
            if ":" in cx:
                _, dst = cx.rsplit(":", 1)
                if dst.lstrip("+") in ("main", "master"):
                    targets_main = True
                    if cx.startswith("+") or "+" in cx.split(":")[0]:
                        has_force = True
                    break
        if has_force and targets_main:
            return (
                "Force-push to main / master is near-irreversible. Get "
                "explicit user confirmation and consider a safer alternative."
            )
    return None


def rule_no_verify(toks, ctx):
    """`--no-verify` / `--no-gpg-sign` on commit/push/rebase/merge/am/cherry-pick/pull."""
    for i_git, j, c_path in walk_git_invocations(toks):
        rest = toks[j:]
        if not rest:
            continue
        sub = strip_parens(rest[0])
        if sub not in (
            "commit", "push", "rebase", "merge", "am", "cherry-pick", "pull",
        ):
            continue
        for x in rest[1:]:
            if is_quoted_content(x):
                continue
            if x in ("--no-verify", "--no-gpg-sign"):
                return (
                    f"`{x}` bypasses hooks/signing. Not allowed — fix the "
                    f"underlying failure instead."
                )
    return None


def rule_broad_git_add(toks, ctx):
    for i_git, j, c_path in walk_git_invocations(toks):
        rest = toks[j:]
        if not rest:
            continue
        sub = strip_parens(rest[0])
        if sub != "add":
            continue
        for x in rest[1:]:
            if is_quoted_content(x):
                continue
            cx = strip_parens(x)
            if cx in ("-A", "-a", "--all", "--update", "-u"):
                return (
                    "Broad `git add` (-A / -a / --all / --update). CLAUDE.md "
                    "(global) requires staging specific files by name to "
                    "avoid sweeping in secrets / large binaries."
                )
            if cx == ".":
                return (
                    "Broad `git add .` stages everything in cwd. Stage "
                    "specific files by name."
                )
            if cx == ":/":
                return "Broad `git add :/` stages the whole repo."
    return None


# `credentials` / `secrets` are restricted to credential-STORAGE extensions
# (data formats + common backup / DB suffixes) so that source files that
# *handle* credentials — e.g. `crates/librefang-extensions/src/credentials.rs`
# — don't trip the guard. The earlier `(\.[a-z]+)?` form swept in `.rs` /
# `.py` / `.go` and forced a manual user override on every legitimate
# code-edit commit.
#
# Whitelisted suffixes:
#   - data formats: json toml yaml yml txt csv ini conf env cfg
#   - credential DBs: kdbx (KeePass), dat (generic credential blob)
#   - backups: bak old (`credentials.bak` / `credentials.old` still hold real keys)
_DATA_EXT = r"(json|toml|yaml|yml|txt|csv|ini|conf|env|cfg|kdbx|dat|bak|old)"
_SENSITIVE_RE = re.compile(
    r"^("
    r"\.env(\.[a-z0-9._-]+)?"
    r"|id_(rsa|ed25519|ecdsa|dsa)(\.pub)?"
    rf"|credentials(\.{_DATA_EXT})?"
    rf"|secrets?(\.{_DATA_EXT})?"
    r"|vault[_-][a-z0-9_-]+\.(key|json)"
    r"|.+\.(pem|p12|pfx|jks|keystore)"
    r")$",
    re.IGNORECASE,
)
_EXAMPLE_RE = re.compile(r"\.(example|template|sample)$", re.IGNORECASE)


def _looks_sensitive(token: str) -> bool:
    if is_quoted_content(token):
        return False
    # Inspect just the basename — `path/to/.env` and `~/.ssh/id_rsa` should both match.
    base = os.path.basename(strip_parens(token))
    if not base:
        return False
    if _EXAMPLE_RE.search(base):
        return False
    return bool(_SENSITIVE_RE.match(base))


def rule_sensitive_file_add(toks, ctx):
    for i_git, j, c_path in walk_git_invocations(toks):
        rest = toks[j:]
        if not rest:
            continue
        sub = strip_parens(rest[0])
        if sub not in ("add", "commit"):
            continue
        # For `git commit`, ignore everything after -m / -F / --message / --file.
        i = 1
        while i < len(rest):
            x = rest[i]
            if is_quoted_content(x):
                i += 1
                continue
            if x in SKIP_AFTER_FLAGS:
                i += 2
                continue
            if x.startswith("--message=") or x.startswith("--file="):
                i += 1
                continue
            if _looks_sensitive(x):
                return (
                    f"Command references a likely-sensitive file ({x}). "
                    f"If you really need to track it, ask the user first."
                )
            i += 1
    return None


_ATTRIBUTION_RE = re.compile(
    r"(Co-Authored-By:.*(Claude|Anthropic|noreply@anthropic\.com)"
    r"|Generated with .{0,40}Claude"
    r"|🤖.*Claude\s+Code)",
    re.IGNORECASE,
)


def rule_claude_attribution(toks, ctx):
    """Inspect the value passed to -m / --message of a `git commit`."""
    for i_git, j, c_path in walk_git_invocations(toks):
        rest = toks[j:]
        if not rest or strip_parens(rest[0]) != "commit":
            continue
        i = 1
        while i < len(rest):
            x = rest[i]
            msg = None
            if x in ("-m", "--message"):
                if i + 1 < len(rest):
                    msg = rest[i + 1]
                i += 2
            elif x.startswith("--message="):
                msg = x[len("--message=") :]
                i += 1
            elif x.startswith("-m") and len(x) > 2:
                msg = x[2:]
                i += 1
            else:
                i += 1
                continue
            if msg and _ATTRIBUTION_RE.search(msg):
                return (
                    "Commit message contains Claude / Anthropic attribution "
                    "(Co-Authored-By / Generated with Claude / 🤖 Claude Code). "
                    "CLAUDE.md forbids these. Remove the line and retry."
                )
    return None


_RM_DANGEROUS_PATHS = {
    "/", "/*", "~", "~/", "$HOME", "$HOME/", "${HOME}", "${HOME}/",
    "/Users", "/Users/", "/home", "/home/", "/usr", "/var", "/etc",
    "/opt", "/private", "/System", "/Library",
    "target", "target/", "./target", "./target/",
    ".git", ".git/", "./.git", "./.git/",
}


def rule_rm_rf_dangerous(toks, ctx):
    for i, t in enumerate(toks):
        if is_quoted_content(t):
            continue
        if strip_parens(t) != "rm":
            continue
        # Collect flags + positionals until we leave this rm command.
        flags = ""
        positionals = []
        for x in toks[i + 1 :]:
            if is_quoted_content(x):
                # Quoted arg — could be a path. Treat as positional.
                positionals.append(x)
                continue
            cx = strip_parens(x)
            if cx in SHELL_OPS:
                break
            if cx.startswith("-") and not cx == "-":
                # Long --opt or short -rf cluster
                flags += cx[1:].lstrip("-")
            else:
                positionals.append(cx)
        # rm needs both -r/-R and -f for our criterion
        recursive = "r" in flags or "R" in flags or "--recursive" in flags
        force = "f" in flags or "--force" in flags
        # Long-flag detection above isn't quite right — refine:
        if "--recursive" in toks[i + 1 :]:
            recursive = True
        if "--force" in toks[i + 1 :]:
            force = True
        if not (recursive and force):
            continue
        for p in positionals:
            if p in _RM_DANGEROUS_PATHS:
                return (
                    f"`rm -rf {p}` against a dangerous path. Be specific or "
                    f"ask the user."
                )
    return None


def rule_librefang_daemon_launch(toks, ctx):
    for i, t in enumerate(toks):
        if is_quoted_content(t):
            continue
        c = strip_parens(t)
        # Match: librefang, librefang.exe, target/{debug,release}/librefang[.exe],
        # ./target/{debug,release}/librefang[.exe]
        is_binary = (
            c == "librefang"
            or c == "librefang.exe"
            or re.fullmatch(r"\.?/?target/(debug|release)/librefang(\.exe)?", c)
        )
        if not is_binary:
            continue
        if i + 1 >= len(toks):
            continue
        sub = strip_parens(toks[i + 1])
        if sub in ("start", "daemon"):
            return (
                "Starting the librefang daemon is a HUMAN workflow per "
                "CLAUDE.md (port 4545 typically owned by the user's session). "
                "Ask the user to run it and report back."
            )
    return None


def rule_gh_pr_merge(toks, ctx):
    for i, t in enumerate(toks):
        if is_quoted_content(t):
            continue
        if strip_parens(t) != "gh":
            continue
        if i + 2 >= len(toks):
            continue
        if strip_parens(toks[i + 1]) != "pr":
            continue
        if strip_parens(toks[i + 2]) != "merge":
            continue
        # Found gh pr merge
        rest = [x for x in toks[i + 3 :] if not is_quoted_content(x)]
        if any(x == "--admin" for x in rest):
            return (
                "`gh pr merge --admin` bypasses branch protection — equivalent "
                "to force-pushing to main. Get explicit user confirmation."
            )
        # `--auto` schedules the merge to run only after every required
        # CI check has passed. The user is already in the loop (they
        # said "merge"), and CI is still the gate, so this is a safe AI
        # action — never an immediate ship-without-checks. Allow it.
        # Plain `gh pr merge <pr>` (no --auto) would merge right now,
        # before CI runs; keep blocking that.
        if any(x == "--auto" or x.startswith("--auto=") for x in rest):
            return None
        return (
            "Refusing immediate `gh pr merge`. An immediate merge ships "
            "before CI runs; pass --auto so GitHub waits for required "
            "checks (recommended: `gh pr merge <pr> --auto --squash`)."
        )
    return None


# -----------------------------------------------------------------------------
# Dispatcher
# -----------------------------------------------------------------------------

RULES = {
    "cargo-build-run":           rule_cargo_build_run,
    "cargo-test-unscoped":       rule_cargo_test_unscoped,
    "worktree-remove-main":      rule_worktree_remove_main,
    "git-mutation-main":         rule_git_mutation_main,
    "sed-i-perl-pi-main":        rule_sed_i_perl_pi_main,
    "redirect-into-main":        rule_redirect_into_main,
    "force-push-main":           rule_force_push_main,
    "no-verify":                 rule_no_verify,
    "broad-git-add":             rule_broad_git_add,
    "sensitive-file-add":        rule_sensitive_file_add,
    "claude-attribution":        rule_claude_attribution,
    "rm-rf-dangerous":           rule_rm_rf_dangerous,
    "librefang-daemon-launch":   rule_librefang_daemon_launch,
    "gh-pr-merge":               rule_gh_pr_merge,
}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--rules", required=True,
                   help="Comma-separated list of rule names to evaluate")
    p.add_argument("--cwd", default="")
    p.add_argument("--main-root", default="")
    p.add_argument("--kind", default="", choices=["", "main", "worktree"])
    args = p.parse_args()

    cmd = sys.stdin.read()
    toks = tokenize(cmd)
    ctx = {
        "cwd": args.cwd,
        "main_root": args.main_root,
        "kind": args.kind,
        "cmd": cmd,
    }

    for name in args.rules.split(","):
        name = name.strip()
        if not name:
            continue
        rule = RULES.get(name)
        if not rule:
            print(f"unknown rule: {name}", file=sys.stderr)
            sys.exit(2)
        msg = rule(toks, ctx)
        if msg:
            print(msg)
            return  # first match wins; let the hook shell print prefix


if __name__ == "__main__":
    main()
