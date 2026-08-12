Hand activation now logs when the activation mutex recovers from a poisoned lock, instead of recovering silently.
The mutex only serializes the check-and-insert critical section and guards no data of its own, so recovering via `into_inner()` was already safe — the gap was visibility into a prior panic, not correctness.
This brings `activate_with_id` in line with the existing `persist_lock` poison-recovery logging (#7028) (@houko)
