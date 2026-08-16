Fail startup when migration audit-row healing cannot complete instead of reporting a successful upgrade with an inconsistent audit trail.
The healing pass is now atomic and can be retried safely after the underlying SQLite failure is resolved (#7625) (@houko)
