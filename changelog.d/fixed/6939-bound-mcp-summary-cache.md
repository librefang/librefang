Bound the rendered MCP summary cache, which grew one entry per distinct allowlist combination for the lifetime of the daemon.
Agent manifests control the allowlist, so a caller cycling through one-off combinations (or stale generations left behind by config reloads) could grow the cache without limit.
The cache now caps at 256 distinct entries and clears wholesale before admitting a new key past that cap, while preserving current-generation cache hits and rendered summary content (#6939) (@houko)
