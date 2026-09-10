# Recovering after a binary downgrade

This is the ops-facing answer to: *I ran a newer LibreFang binary, its first boot migrated the SQLite database forward, and now I want to go back to the older binary — what happens, and what do I do?*

Short version: the older binary refuses to boot by design, nothing is corrupt, and the way back is an *offline* restore of a backup taken before the upgrade — not the `POST /api/restore` endpoint, for the reason in [Why not `POST /api/restore` for this](#why-not-post-apirestore-for-this).

## What happens when an older binary meets a newer database

Every schema change ships in the PR that consumes it and takes the next free migration number at merge time.
At boot the memory substrate runs the migration ladder (`crates/librefang-memory/src/migration.rs`), and `SCHEMA_VERSION` at `migration.rs:8` is the highest step that binary knows.
When the database's `user_version` is higher than that, the guard at `migration.rs:16-28` refuses to run anything and the daemon does not start.
The guard's own text is wrapped twice on the way out: `crates/librefang-memory/src/substrate.rs:217` propagates the `rusqlite` error out of the substrate constructor through `LibreFangError::memory`, which renders as `Memory error: {message}` (`crates/librefang-types/src/error.rs:61`), and `crates/librefang-kernel/src/kernel/boot.rs:397` wraps that again into `BootFailed(format!("Memory init failed: {e}"))`.
So the line in the journal carries both prefixes:

```
Memory init failed: Memory error: Database schema version 58 is newer than this binary supports (54). Downgrade is not supported. Use the correct binary version or restore from backup.
```

Search for `Downgrade is not supported` rather than for the start of the line — the two prefixes are exactly what a grep built from the guard's format string in `migration.rs` will miss.

The refusal is deliberate.
Before #3962 (April 2026), an older daemon on a newer database *silently rewrote `user_version` down* and kept booting.
That was issue #3656: the operator saw NULL and missing-column errors on the newer tables, and the next forward upgrade re-ran migrations against a schema they had partly applied.
The guard added by #3962 (together with per-step migration transactions and the daemon file lock) turns that silent corruption into a refusal to start with an instruction.

There is no downgrade migration ladder on purpose.
Migrations are additive — new tables, columns and indexes that the newer binary's code consumes — and the older binary has no DDL to remove them and no code that understands the schema without them.
Pre-declaring the next versions' schema before those PRs land was reviewed and rejected (#8068): a stub migration claims a version number, `run_step!` is gated on `current_version < version` (`migration.rs:95-104`), and the real migration behind that number then never runs on such a database — a silently diverged schema with no error anywhere.

## What a backup covers

The daemon ships a backup feature: `POST /api/backup`, or Admin → Runtime → Backups in the dashboard.
It walks the home directory and writes `librefang_backup_<timestamp>.zip` into `<home_dir>/backups/` (`crates/librefang-api/src/routes/backup.rs:243-249`).
The archive holds `config.toml`, agent workspaces, skills, workflows and the whole `data/` tree (`BACKUP_LAYOUT`, `backup.rs:68-80`) — the SQLite databases themselves included, since they live under `data/` (`backup.rs:265-269`): the memory database defaults to `data/librefang.db` (`crates/librefang-kernel/src/kernel/boot.rs:378-383`) and the A2A task store to `data/a2a_tasks.db` (`boot.rs:1811`).
Only the SQLite `-shm` shared-memory index sidecar is left out of the archive, on both backup and restore: it is the WAL index for the connections currently mapping the database, not state, and SQLite recreates it on demand (`backup.rs:185-192`, skipped at `backup.rs:314` and `backup.rs:949`).

A backup is a live file-level copy taken while the daemon runs.
It is not a transactionally consistent SQLite snapshot, so for the cleanest artifact create it while the daemon is quiet.

### Check first: the databases may not be under `<home_dir>/data/` at all

Everything below assumes the SQLite databases sit inside the `data/` tree the archive carries, and two `config.toml` keys can move them out of it.
`data_dir` relocates the whole directory (default `~/.librefang/data`, `crates/librefang-types/src/config/types.rs:3470`, documented at `docs/src/app/configuration/page.mdx:523`), and `[memory] sqlite_path` (`types.rs:7401`) overrides the memory database's path outright — `boot.rs:378-383` resolves it as `sqlite_path` if set, else `data_dir/librefang.db`.
The backup side follows neither: `BACKUP_LAYOUT`'s `data` tree is resolved by `backup_source` as `home_dir.join("data")` (`backup.rs:152`), a hard-coded `<home_dir>/data`.

On a deployment that sets either key the archive therefore contains no database at all, and the rollback below silently does nothing to `user_version` — the older binary prints the same refusal after every step has apparently succeeded.
Read those two keys out of `config.toml` before starting, and if either is set, move the databases back by hand from wherever they actually live; the archive can only restore the rest.

## The sanctioned rollback procedure

The recovery needs a backup taken before the upgrade — while the old binary still runs.
Do it offline, with every daemon process stopped: the restore replaces the SQLite files, and no process may be holding them open while that happens.

1. Do not force the older binary onto the migrated database; the guard will keep refusing, and that is correct.
2. Stop every daemon process. Nothing starts again until step 4.
3. Unzip the pre-upgrade archive over the home directory.
   The archive is home-relative, so this writes back `config.toml`, `skills/`, `workflows/` and the whole `data/` tree in place — which returns `user_version` to the pre-upgrade value, provided the databases really are under `<home_dir>/data/` (see the precondition above).
   Two things a plain `unzip` gets wrong, the first of which the restore endpoint corrects for you and the second of which it does not:
   - The archive's `agents/` tree has to end up in the agent workspaces directory — `<home_dir>/workspaces/agents/`, or `<workspaces_dir>/agents/` when `workspaces_dir` is set — because that is where `create_backup` read it from and where `restore_root` (`backup.rs:172-181`) writes it back.
     Left at `<home_dir>/agents/` it sits in the pre-unification legacy layout, which the kernel only relocates when the canonical destination does not already exist; on a rollback it does, so the archived workspaces are silently stranded.
   - Delete both the `-wal` and the `-shm` next to every SQLite database being restored *before* extracting — `data/librefang.db` and `data/a2a_tasks.db` under a default layout — and let the archive's own pair land if it carries one.
     The `-wal` is the one that can silently undo the whole rollback.
     `unzip -o` overwrites only the entries the archive actually holds, and a backup taken while the old daemon was stopped, or after a checkpoint, carries no `-wal` entry at all; the newer binary's `-wal` then survives next to the restored older database.
     SQLite validates WAL frames by their own checksums and salts, and nothing binds a WAL to a particular copy of the database file, so those frames are replayed on the next open — including page 1, which is where `user_version` lives.
     The operator completes every step and then either sees the identical version refusal or boots onto the migrated schema the guard exists to keep them away from, with nothing pointing at the leftover file.
     Deleting the `-shm` alone is not enough here: the restore endpoint skips `-shm` and nothing else (`is_sqlite_shared_memory_index`, `backup.rs:191`, applied at `backup.rs:949`), because it restores into the same daemon rather than across a version boundary.

   To keep the target's own `config.toml` instead of the archive's — the manual equivalent of the endpoint's `keep_config: true` — restore everything else and leave that one file alone.
4. Start the **older** binary.
   The guard now passes, because the database is back at the version that binary shipped with.

Anything the newer binary wrote between the backup and the downgrade is gone; that is the price of the rollback.

### Why not `POST /api/restore` for this

The endpoint (`backup.rs:1023-1041`, or the Backups tab's Restore control) is the obvious first reach: boot the newer binary, which accepts the database at its own version, restore, then swap binaries.
It is the wrong tool here, and the reason is the same one this page exists for.

The endpoint runs inside a live daemon holding an open connection pool on `data/librefang.db`, and the archive carries that database's `-wal` alongside it — `is_sqlite_shared_memory_index` (`backup.rs:191`) excludes the `-shm` and nothing else.
So the restore replaces the database file *and* its write-ahead log underneath connections that are still mapped to them, and until the daemon is stopped it can checkpoint its own stale WAL over what was just restored.
The endpoint's own contract frames the restart as a matter of "all changes to take effect", which reads as a freshness caveat; in this procedure it is a data-integrity one.

If there is no shell on the host and the endpoint is the only way in, stop the daemon the instant the call returns and issue no other API call in between.

## If there is no backup

There is no in-product downgrade: the guard has no reverse ladder, and the error message already says the two supported exits are the correct binary version or a backup restore.
If neither exists, the options are:

- Stay on the newer binary — always safe, and usually the right call.
- Start fresh: stop the daemon, move the data directory aside — `<home_dir>/data/`, or wherever `data_dir` points — and let the old binary build an empty database at its own version.
  `config.toml`, agent workspaces, skills and workflows survive, because the backup layout holds them as separate trees outside `data/` (`backup.rs:68-80`).
  Three components that same layout names separately do *not*: `data/cron_jobs.json`, `data/hand_state.json` and `data/custom_models.json` live inside the tree being moved aside (`backup.rs:69-79`).
  All three are plain JSON rather than schema-versioned SQLite, so copy them back out of the moved-aside directory before starting the old binary — otherwise every cron schedule, hand state record and custom model definition goes with it.
  Memory, sessions, the audit trail and every other `data/` artefact are lost either way.

## See also

- [`config-reload.md`](./config-reload.md) — which config changes take effect without a restart; `config.toml` is not migrated, so a downgrade does not touch it.
- [`audit-chain-recovery.md`](./audit-chain-recovery.md) — the comparable recovery procedure for a broken audit chain, which documents when *not* to repair.
