A fallback model now resolves its own reasoning policy from the catalog instead of inheriting the primary model's, so a failover cannot send a level the fallback does not support.
The policy lookup is scoped to the provider: a provider the catalog never heard of is undeclared rather than borrowing a same-id entry from another vendor, and provider aliases such as `codex` resolve to their canonical entry.
`thinking: false` in a request frame now beats a session effort pin, matching the documented precedence.
Undeclared models coerce to the four classic levels (`medium`/`minimal` to `low`, `xhigh` to `high`); this is a change from the released behaviour, which passed the token through verbatim.
Effort declarations are validated at API and runtime ingress, request tokens are trimmed and ASCII-lowercased, and a declaration whose tokens are all invalid is treated as inherit while an explicit empty set still means budget-controlled.
The dashboard keeps an operator's effort overrides when saving a model and stops sending `thinking` until the toggle is touched (#8616) (@DaBlitzStein).
