The test that guards the cron prune lock against being held across the summarize await now proves concurrency with a barrier instead of a wall-clock budget.
It asserted that two 200ms fires finish in under 350ms, which reads as the same claim but is not: it also fails when a loaded runner merely delays a sleep, and on CI it did exactly that at 432ms with nothing wrong with the lock, blocking an unrelated dependency PR.
A timing bound cannot separate "serialized" from "descheduled", and on a shared two-core runner executing the whole suite in parallel the two are indistinguishable.
The barrier releases only once both fires stand inside the await, so real serialization now deadlocks and is reported as such rather than being inferred from a stopwatch.
The test also stops sleeping, so it runs in 0.06s instead of 0.4s (#8383) (@houko)
