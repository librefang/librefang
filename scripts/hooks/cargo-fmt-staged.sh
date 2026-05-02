#!/usr/bin/env bash
# cargo fmt --check, but only on the Rust files passed in (staged).
# Used by .pre-commit-config.yaml's cargo-fmt-staged hook so commits
# don't pay for a full-workspace fmt scan.
#
# rustfmt accepts file paths directly via `--`. We forward only files
# that still exist on disk (deletions are skipped) and let rustfmt
# handle the actual diff/check. Empty arg list = nothing to do.

set -eu

files=()
for f in "$@"; do
  case "$f" in
    *.rs)
      [ -f "$f" ] && files+=("$f")
      ;;
  esac
done

if [ "${#files[@]}" -eq 0 ]; then
  exit 0
fi

exec cargo fmt --check -- "${files[@]}"
