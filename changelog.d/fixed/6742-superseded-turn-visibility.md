Make a superseded agent turn visible in the log instead of discarding it at `debug!` level, below the default filter.
When a newer message arrives for the same `(agent, session)` the in-flight turn is aborted and produces no reply at all, and in a group channel the error text is suppressed too, so the only symptom an operator ever saw was a bot that silently ignored them.
The abort key matters more than it looks: for a group the session spans the whole **channel**, not the thread, so a message from any user in any thread preempts whatever turn is running for that agent there.
The warning now names that mechanism and reports how long the discarded turn had been running, and the channel bridge separately records that the aborted turn emits nothing.
Logging only — the supersede policy itself is unchanged.
(#6742) (@houko)
