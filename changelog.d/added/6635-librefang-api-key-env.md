Add `LIBREFANG_API_KEY` as an environment override for the API bearer token.
`config.toml` lives inside the daemon's own writable data dir and is rewritten at boot, so it cannot be mounted from a Kubernetes Secret — leaving no way to supply `api_key` without baking the literal into an image.
An empty value is ignored with a warning rather than treated as "clear the key", because a Secret key that exists but is unset would otherwise disarm bearer authentication on a non-loopback bind (#6635) (#6638) (@houko)
