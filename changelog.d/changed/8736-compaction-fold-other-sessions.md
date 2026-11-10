A session no longer loses compacted history because a different session on the same agent compacted.
The threshold compaction trims the oldest messages out of the agent's shared canonical window, and until now only the compacting session's own messages reached a summary while every other session's trimmed messages were dropped.
Each other session's trimmed messages are now folded into that session's own compaction summary in the same transaction, without changing which session owns the canonical summary or letting any session's content into another's. (#8736) (@houko)
