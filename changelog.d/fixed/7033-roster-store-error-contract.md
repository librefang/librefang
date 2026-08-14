Group roster storage (`RosterStore::upsert`, `members`, `remove_member`, `member_count`) swallowed every SQLite pool-exhaustion and row-decode error, returning empty results or fixed defaults instead of failing.
A corrupted `group_roster` row was silently dropped from `members()` rather than surfacing as a query failure, and a pool outage during `upsert` or `remove_member` looked identical to success to every caller.
All four methods now return `LibreFangResult`, and the channel bridge and kernel handle boundaries propagate the error instead of discarding it (#7033) (@houko)
