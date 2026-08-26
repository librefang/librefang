The `librefang-memory` concurrency regression test now configures its SQLite pool the way a file-backed store is actually deployed, instead of a rollback-journal configuration production never uses.
Contending 24 threads on one key without `journal_mode=WAL` makes the commit path promote RESERVED to EXCLUSIVE, and SQLite deliberately skips the busy handler on that promotion to avoid deadlocking two waiters against each other — so the pool's busy timeout never applied and Windows CI failed with "database is locked".
A guard test pins the journal mode, because raising the busy timeout had already been tried as a fix and could not address that path.
(#7909) (@houko)
