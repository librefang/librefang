The Docker image builds again.
`librefang-channels` reads the Python SDK's version out of `sdk/python/pyproject.toml` with `include_str!`, but neither the `.dockerignore` allowlist nor the `Dockerfile` carried that file into the build context, so every image build failed at `couldn't read crates/librefang-channels/src/../../../sdk/python/pyproject.toml`.
(#7967) (@houko)
