# [Low] Repo hygiene — naming confusion, stale CLAUDE.md, xtask vs justfile overlap

**Severity:** Low · **Domain:** Architecture
**Status:** Merges 2 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `librefang-migrate` (framework-import tool) and `librefang-memory/src/migration.rs` (SQLite schema) share the same word; searching for "migration" returns the wrong file | `crates/librefang-migrate/`, `crates/librefang-memory/src/migration.rs` |
| stale CLAUDE.md | `extensions/CLAUDE.md` + `channels/CLAUDE.md` are out of sync with the current code | the two CLAUDE.md files |
| xtask vs justfile | `xtask/` and `justfile` overlap (setup / dev, etc.); newcomers don't know which to use | `xtask/`, `justfile` |

## Combined fix plan

1. (this) Rename `librefang-migrate` → `librefang-import` (update `Cargo.toml`, CLI bin name, all docs).
2. (stale CLAUDE.md) Reconcile the two CLAUDE.md files with the code: delete descriptions of interfaces that no longer exist, add the new ones; or merge them into a top-level CLAUDE.md "crate index."
3. (xtask vs justfile) Pick one entry point: either `justfile` is a thin wrapper calling `cargo xtask ...`, or vice versa. Document which commands live where.
