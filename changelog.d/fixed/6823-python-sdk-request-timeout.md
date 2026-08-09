Apply a bounded timeout to every generated Python SDK HTTP request. (@houko)
Both ordinary API calls and SSE stream setup previously called `urlopen` without a timeout, leaving connection establishment and stalled socket reads without an inactivity bound.
Clients now use a 30-second default for both paths and accept a constructor-level timeout override for deployments that need a different network budget.
