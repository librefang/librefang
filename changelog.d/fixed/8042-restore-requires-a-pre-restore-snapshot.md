Restoring an agent type from the registry now fails with a 500 and writes nothing when the pre-restore snapshot cannot be recorded, instead of overwriting the file anyway and answering 200.
Every other version snapshot in this route is deliberately best-effort, and correctly so: those run after the write, so losing one costs a history row while the content is still on disk.
The pre-restore snapshot is the opposite case, because it is the only copy of content the very next line destroys.
An operator whose `~/.librefang/agent-types/{name}.toml` was last written by hand has no history row for what is on disk, so a snapshot that failed — a `SQLITE_BUSY` from a concurrent writer is the realistic way — took that configuration with it and reported success. (#8042) (@DaBlitzStein)
