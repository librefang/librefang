Make channel-journal transition failures observable to callers instead of logging and swallowing failed writes.
Journal queries now filter stale entries without mutating recovery state, while the hourly maintenance path performs explicit cleanup before compaction.
Retry claims clear obsolete deadlines consistently, and malformed retry windows become hard failures instead of overflowing into immediate redispatch or panicking (#7664) (@houko)
