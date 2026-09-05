`librefang skill publish` now resolves its GitHub token the same way the HTTP routes do — the environment first, then the credential vault.
It previously read `GITHUB_TOKEN` / `GH_TOKEN` from the environment only and exited with status 1, so the same daemon could promote a skill through the API and fail to publish one from the CLI on the same machine with the same token in the vault.
The "no token" message now names both places a token can live instead of only the environment variables. (#8163) (@DaBlitzStein)
