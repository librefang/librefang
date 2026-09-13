Asking to export a session that does not exist now answers 404 instead of 500.
So does asking for one that exists under a different agent.
The kernel returned the miss as a string inside `LibreFangError::Internal`, and the route helper typed only the two agent-shaped errors, so every other kernel error — including a plain bad id — became a server fault whose reason was then scrubbed out of the body.
The scrub is right and stays, because the memory layer wraps every rusqlite error in that same variant and echoing one would leak SQL schema; what was wrong was calling a missing session an internal error in the first place.
A caller could not distinguish a typo from an outage, and a scripted client saw a retryable 5xx where the answer will never change.
The fix is typed rather than a match on the message text: `SessionNotFound` and `ResourceNotFound` already existed and the sibling helper for `KernelOpError` already mapped both to 404, so this brings the outlier into line for the fifteen handlers that share it.
That also fixes tool-level misses, which reach the same helper as `ResourceNotFound` and were 500 for the same reason.
A session belonging to another agent is reported as not found rather than as a distinct wrong-owner error, so the answer does not confirm the session exists to someone who cannot read it. (#8263) (@DaBlitzStein)
