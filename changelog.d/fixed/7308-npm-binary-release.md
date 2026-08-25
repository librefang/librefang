Make the split npm binary release path explicitly request OIDC provenance for every package instead of relying on unverified environment inheritance.
The job now bounds network and build time, caches Rust dependencies, and keeps repository context out of shell source while the existing PAT release path remains unchanged (#7308) (@xiaomo)
