Add `GET /api/ready`, a public readiness probe that returns 503 when a dependency required to accept work is unavailable.
`GET /api/health` could not serve this purpose: it returns 200 even while its body reports `status: degraded`, so a Kubernetes probe — which sees only the status code — could never remove a degraded pod from Service endpoints.
Changing `/api/health` itself would have conflated liveness with readiness and restart-looped pods through recoverable storage incidents, so the two contracts are now separate endpoints (#6633) (#6638) (@houko)
