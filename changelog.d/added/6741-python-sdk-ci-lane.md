Run the Python SDK test suite in CI.
`sdk/python/tests/` held roughly 1900 pytest cases covering the HTTP client and every stdlib-only sidecar channel adapter — slack, discord, telegram, mastodon and the rest — and no workflow ran a single one of them, so the production code path for every sidecar channel shipped with CI fully green regardless of what broke.
The `sdk/` prefix was already routed to the Rust lane, but only as an openapi codegen drift guard, which runs cargo and never pytest.
The new lane installs the package with its `dev` extra and runs the suite on any `sdk/python/**` change, in under a minute (#6741) (@houko)
