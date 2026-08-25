Confidence decay now charges each interval of idle time exactly once, instead of re-applying a memory's entire idle span on every hourly tick.
The `UPDATE` wrote `confidence` and nothing else, so each run recomputed the exponent from an `accessed_at` that never moved and applied it to an already-decayed value: a row idle for `D` days accrued `24 x rate x D` of decay per day rather than `rate`, growing quadratically in idle time.
On the instance that reported this, configured for a roughly 70-day half-life, that drove the median raw-dialogue memory to 0.001 across a corpus whose oldest row is 102 days old — where the configured rate can produce no less than 0.36.
A new `memories.last_decayed_at` column (schema v48) records when a row was last charged, and the pass measures from the later of that stamp and `accessed_at` so an actively recalled memory is not handed one large retroactive decay the first hour it goes idle.
Rows written before the migration read back NULL and fall back to `accessed_at`, taking exactly the one-time decay the documented formula always intended rather than jumping at the migration boundary.
(#7864) (@houko)
