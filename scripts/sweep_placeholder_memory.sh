#!/usr/bin/env bash
# Sweep placeholder-leak rows from the LibreFang episodic memory bank.
#
# Background: the kernel formerly stored interactions as
#   "User asked: <user_message>\nI responded: <agent_response>"
# When the model occasionally emitted a placeholder ("<empty>", "<response>",
# "<silent>", "<no_reply>") instead of a real reply, the kernel persisted
# rows like
#   "User asked: ...\nI responded: <empty>"
# These rows are then retrieved by `proactive_memory::auto_retrieve` and
# rendered as bullet items in the system prompt's Memory section, which
# (a) wastes context budget and (b) trains the model to imitate the
# placeholder pattern on subsequent turns.
#
# The behavioural fix lives in the runtime (gate empty/silent responses out
# of `remember_interaction_best_effort`, mandate `NO_REPLY` only as the
# silent sentinel, defensive output guard for system-prompt regurgitation).
# Existing rows still need a one-shot cleanup — that's what this script does.
#
# Usage:
#   sweep_placeholder_memory.sh [--apply] [--db <path>]
#
# Default mode is dry-run: prints how many rows MATCH but does not modify the
# database. Pass --apply to soft-delete (memories.deleted = 1) the matching
# rows. The matching predicate is intentionally narrow:
#
#   * scope = 'episodic'
#   * deleted = 0
#   * content matches one of the known placeholder leak shapes
#
# The default --db comes from $LIBREFANG_DB if set, falling back to
# `./data/librefang.db` (a relative path that works on a host clone).
# On a host filesystem you can pass --db explicitly. Inside a container
# without sqlite3 installed, run from the host using the /proc/<PID>/root/
# pivot:
#
#   PID=$(lzc-docker inspect cloudlazycatapplibrefang-librefang-1 \
#         --format '{{.State.Pid}}')
#   sqlite3 /proc/$PID/root/data/librefang.db < <( ... )
#
# Idempotent: re-running after --apply finds zero matches.

set -euo pipefail

DB="${LIBREFANG_DB:-./data/librefang.db}"
APPLY=0

usage() {
    cat <<'USAGE'
sweep_placeholder_memory.sh — soft-delete placeholder-leak rows from the
LibreFang episodic memory bank.

Usage:
  sweep_placeholder_memory.sh [--apply] [--db <path>]

  --apply       Commit the soft-delete (memories.deleted = 1). Default is
                dry-run: report counts only.
  --db <path>   Database path. Defaults to /data/librefang.db (Lazycat NAS).
  -h, --help    Show this help.

The default mode is dry-run; --apply takes a timestamped backup of the
database file before mutating it, then runs the UPDATE inside a
transaction so a partial failure never leaves the bank inconsistent.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --apply)  APPLY=1; shift ;;
        --db)     DB="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *)
            echo "unknown arg: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if ! command -v sqlite3 >/dev/null 2>&1; then
    echo "sqlite3 not found in PATH" >&2
    exit 3
fi
if [[ ! -f "$DB" ]]; then
    echo "database not found: $DB" >&2
    exit 4
fi

# No user input is ever interpolated into the SQL so the only risk
# is operator typo in this file — keep the predicate one place.
PREDICATE="
    scope = 'episodic'
    AND deleted = 0
    AND (
        content LIKE '%I responded: <empty>%'
        OR content LIKE '%I responded: <response>%'
        OR content LIKE '%I responded: <silent>%'
        OR content LIKE '%I responded: <no_reply>%'
        OR content LIKE '%I responded: <answer>%'
        OR content LIKE '%</answer>%'
        OR content LIKE '%</response>%'
    )
"

# Initialise BEFORE the read so a sqlite3 error that produces empty
# stdout doesn't leave the variables unset under `set -u`.
TOTAL_EPISODIC=0
MATCH_COUNT=0
COUNTS=$(sqlite3 -separator ' ' "$DB" "
    SELECT
      (SELECT COUNT(*) FROM memories WHERE scope = 'episodic' AND deleted = 0),
      (SELECT COUNT(*) FROM memories WHERE ${PREDICATE});
")
# Pin IFS instead of inheriting; `|| :` because read exits non-zero on
# missing trailing newline, which would otherwise trip `set -e`.
IFS=' ' read -r TOTAL_EPISODIC MATCH_COUNT <<<"$COUNTS" || :

echo "Database:           $DB"
echo "Episodic memories:  $TOTAL_EPISODIC"
echo "Placeholder leaks:  $MATCH_COUNT"

if [[ "$APPLY" -eq 0 ]]; then
    echo
    echo "(dry-run — no changes written. Re-run with --apply to soft-delete.)"
    exit 0
fi

if [[ "$MATCH_COUNT" -eq 0 ]]; then
    echo "Nothing to do."
    exit 0
fi

BACKUP="${DB}.bak.$(date +%Y%m%d-%H%M%S)"
echo
echo "Backing up database to $BACKUP via online .backup ..."
# `.backup` is WAL-aware: a `cp` of just the main file would lose any
# committed pages still in the -wal sidecar. Single-quote escape the
# backup path against the surrounding SQL string in case --db ever
# resolves to a path containing apostrophes.
BACKUP_SQL="${BACKUP//\'/\'\'}"
sqlite3 "$DB" ".backup '${BACKUP_SQL}'"

echo "Applying soft-delete to $MATCH_COUNT rows (transactional)..."
# `-bail` aborts the script (and the transaction) on the first sqlite3
# error so a failing UPDATE never reaches `COMMIT`. Without it the
# heredoc would commit a partial mutation.
sqlite3 -bail "$DB" <<SQL
BEGIN IMMEDIATE;
UPDATE memories SET deleted = 1 WHERE ${PREDICATE};
COMMIT;
SQL

REMAINING=$(sqlite3 "$DB" "SELECT COUNT(*) FROM memories WHERE ${PREDICATE};")
echo "Remaining matching rows: $REMAINING (expected: 0)"
if [[ "$REMAINING" -ne 0 ]]; then
    echo "WARN: some rows still match — re-inspect predicate. Backup: $BACKUP" >&2
    exit 5
fi
echo "Done. Backup retained at $BACKUP — delete it once the agent has run cleanly."
