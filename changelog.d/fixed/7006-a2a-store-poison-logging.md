A2A task-store lock recovery from a poisoned state is now logged for both the in-memory task map and the backing SQLite connection, instead of recovering silently.
Task loading, persistence, lookup, and mutation all remain usable after recovery (#7006) (@houko)
