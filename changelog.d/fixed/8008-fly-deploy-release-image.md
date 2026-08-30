The official Fly demo deploys the release being cut instead of a hardcoded July build.
`deploy/fly/fly.toml` pinned `ghcr.io/librefang/librefang:v2026.7.31`, which had since been removed from the registry, so the deploy failed with "Could not find image" — and while it existed, the demo served that stale build after every release.
(#8008) (@houko)
