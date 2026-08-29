Backup no longer archives SQLite's `-shm` shared-memory index, and restore skips one carried by an older archive.
The file is the WAL index for whichever connections are currently mapping the database, so a snapshot of it means nothing to another process and writing one back over a live database was never right; on Windows it also failed outright, because truncating a file with an active mapped section returns `ERROR_USER_MAPPED_FILE` and left every restore reporting a partial failure.
(#7966) (@houko)
