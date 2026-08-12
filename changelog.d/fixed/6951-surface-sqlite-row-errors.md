Memory consolidation and the per-user spend ranking used `.filter_map(|r| r.ok()).collect()` over their SQLite row iterators, which silently dropped any row that failed to decode instead of surfacing the failure.
A corrupted `agent_id` could make consolidation skip a tenant's memories with no error, and a corrupted usage row could make a user vanish from the spend ranking rather than showing up as a failed query.
Both call sites now collect into `rusqlite::Result<Vec<_>>` and propagate the decode error (#6951) (@houko)
