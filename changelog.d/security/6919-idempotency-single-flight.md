Reserve each `Idempotency-Key` atomically before its handler starts, so concurrent retries can no longer execute the same state-creating side effect twice.
An in-flight duplicate now receives `409 idempotency_key_in_use`; owner tokens prevent stale requests from modifying replacement reservations, cancelled and non-successful attempts release their reservation, and storage, clock, or corrupt-status failures fail closed instead of bypassing deduplication.
Expired-row pruning is limited to once per minute (#6919) (@houko)
