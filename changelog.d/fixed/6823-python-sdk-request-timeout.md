Apply a bounded timeout to every generated Python SDK HTTP request.
Both ordinary API calls and SSE stream setup previously called `urlopen` without a timeout, leaving connection establishment and stalled socket reads without an inactivity bound.
Clients now use a 30-second default for both paths and accept a constructor-level timeout override for deployments that need a different network budget.
A slow-to-respond server now raises the SDK's own `LibreFangError` instead of a bare `TimeoutError`, keeping the new failure mode inside the same error contract callers already rely on for connection and HTTP errors (#6823) (@houko)
