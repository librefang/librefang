# Recovering a broken audit chain

`librefang security verify` reporting `chain break at seq N` means the persisted Merkle chain in `audit_entries` no longer verifies from row N onward, and it will keep reporting that on every boot until something repairs it.
This page covers what to do about it.

## First, decide whether you need to repair anything

Two conditions look alike in the logs and are not the same.

`audit anchor mismatch: anchor says seq=… but DB has rows=…` is not a chain break.
The rows are fine and the external anchor file has fallen out of step with them — usually a database restored from backup without its anchor, or the reverse.
Nothing below applies.

`chain break at seq N` is the real thing: row N's `prev_hash` does not name the hash of the row before it, or row N's own stored hash does not match its content.
`librefang security audit-reanchor` (no flags) diagnoses which, without touching anything.

## What produced it

The mechanism behind the reports on record is two independently derived chains merged into one table.
A second writer holds a pre-transaction snapshot of `(seq, prev_hash)`; its `INSERT` used to be blocked only by the `seq INTEGER PRIMARY KEY` collision, and that interlock disappears the moment the row occupying its stale `seq` is deleted while higher rows survive — which the default 90-day retention prune does unattended.
The stale writer's row then lands successfully carrying a `prev_hash` that names a row which is no longer its predecessor, and the chain forks at exactly one sequence number: everything below it verifying, one break, everything above it verifying.

That path is closed as of #7847 — `record_with_context` now reads the tail inside the same `BEGIN IMMEDIATE` transaction as the `INSERT`, so `seq` and `prev_hash` are true at write time rather than true by coincidence.
A log broken before that fix stays broken, which is what the tool below is for.

Two things worth confirming on the affected host, because they change what you conclude:

- `SELECT seq, timestamp, prev_hash, hash FROM audit_entries WHERE seq BETWEEN N-10 AND N+10 ORDER BY seq;` — contiguous `seq` across the break means two chains merged; a gap means rows were removed first.
- Whether `timestamp` is monotonic across the break. Non-monotonic is the stale-writer fingerprint.

## Repairing it

```
librefang stop
librefang security audit-reanchor            # diagnose and print the plan
librefang security audit-reanchor --confirm  # perform it
librefang start
librefang security verify
```

The command is offline by design and refuses to run while a daemon holds the database — including for the dry run, because a diagnosis taken against a live writer describes a state that may already have changed by the time you act on it.

A Merkle chain admits exactly one predecessor per row, so a repair necessarily severs one side of the break.
The only real question is whether the severed rows are destroyed or preserved, and `audit-reanchor` preserves them:

1. The rows at and after the break are written to `<data_dir>/audit-archive/audit-severed-<seq>-<timestamp>.jsonl`, one JSON object per row, verbatim — `seq`, `timestamp`, `agent_id`, `action`, `detail`, `outcome`, `user_id`, `channel`, `prev_hash`, `hash`. This happens **before** anything is deleted.
2. Those rows are deleted.
3. A `ChainReanchored` entry is appended, linked to the last row that still verified. Its `detail` is a JSON document naming the break, the number of rows severed, the archive's file name and the archive's SHA-256.
4. The anchor file is rewritten with the post-repair row count and tip.

Steps 2 through 4 happen inside one `BEGIN IMMEDIATE` transaction that re-diagnoses the break first and aborts without mutating anything if it no longer matches what the dry run reported.

Committing the archive's digest into the chain is what keeps the preserved rows tamper-evident: altering the archive after the repair no longer matches the hash the (now verifying) chain vouches for.
Keep the archive next to the database — it is the forensic record of everything the repair removed.

## Reading an archive

```
jq -s 'sort_by(.seq) | .[] | {seq, timestamp, agent_id, action, detail}' \
  ~/.librefang/data/audit-archive/audit-severed-204-*.jsonl
```

To confirm an archive is the one the chain vouches for, compare its digest against the marker entry:

```
shasum -a 256 ~/.librefang/data/audit-archive/audit-severed-204-*.jsonl
librefang security audit --limit 200 --json \
  | jq -r '.entries[] | select(.action == "ChainReanchored") | .detail | fromjson | .archive_sha256'
```

## When to use `audit-reset` instead

`librefang security audit-reset` empties `audit_entries` and removes the anchor.
It restores verification by destroying the evidence, so it is appropriate only on a development machine whose audit history has no value.
In a compliance or production environment, use `audit-reanchor`.

The one case `audit-reset` still covers that `audit-reanchor` does not is a database whose `audit_entries` rows cannot be decoded at all — an unknown `action` string from a newer daemon, or a row with a negative `seq`.
`audit-reanchor` fails closed there rather than guessing, and reports the row it could not read.
