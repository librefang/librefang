Proactive-memory lock recovery from a poisoned state is now logged for the runtime config lock and the decay/cleanup/counter-prune maintenance locks, instead of recovering silently.
Config reads and writes, and background maintenance scheduling, remain usable after recovery (#7003) (@houko)
