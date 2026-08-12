`atomic_write` fsynced the staged temp file before the rename but never synced the containing directory afterward, so the rename itself was not guaranteed durable.
A crash between the rename syscall and the next unrelated fsync of that directory could still lose the update on some filesystems, even though the write looked atomic from the caller's side.
On Unix, the parent directory is now fsynced after the rename so the new directory entry survives a crash (#6942) (@houko)
