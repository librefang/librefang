Proactive-memory extraction now keeps its kernel-handle slot usable when a thread panics while holding the lock, instead of silently ignoring all later handle reads and updates.
Conversation prompt assembly also uses a preallocated buffer and writes each message directly, avoiding a temporary allocation per turn.
(#6911) (@houko)
