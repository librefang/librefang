The dashboard and the TUI can now set the daemon's GitHub token, which is what skill proposal and agent-type promotion fall back to when no `GITHUB_TOKEN` is in the environment.
Until now the only way to put one there was to edit the host's environment and restart, because the vault had no write API and then no interface consuming it.
The dashboard control lives on the Settings page beside TOTP and passkeys; the TUI's is sub-tab `7` of the Settings screen.
Both build their list from what the daemon reports as writable, so adding a key to the allowlist surfaces it in both places with no further change.
Neither interface can display a stored value — the API has no read-back endpoint, the input starts empty every time, and the only status either shows is whether a key holds something.
(#8164) (@DaBlitzStein)
