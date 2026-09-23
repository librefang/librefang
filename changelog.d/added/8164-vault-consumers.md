The dashboard and the TUI can now set the daemon's GitHub token, which is what skill proposal and agent-type promotion fall back to when no `GITHUB_TOKEN` is in the environment.
Until now the only way to put one there was to edit the host's environment and restart, because the vault had no write API and then no interface consuming it.
The dashboard control lives on the Settings page beside TOTP and passkeys; the TUI's is sub-tab `7` of the Settings screen.
Both build their list from what the daemon reports as writable, so adding a key to the allowlist surfaces it in both places with no further change.
Neither interface can display a stored value — the API has no read-back endpoint, the input starts empty every time, and the only status either shows is where the daemon resolves the key from.
That status is the effective source and not vault presence, which matters on any host that already exports the credential: the daemon reads its environment first, so both surfaces would otherwise have shown `Not set` while promotion worked, and shown it again after a removal that revoked nothing.
An environment-overridden key is now badged as such in both places, with wording that says storing or clearing a value there changes the stored copy rather than the credential the daemon uses, and the confirmation after each write says the same instead of reporting a plain success.
The TUI additionally holds the secret an operator is typing in a `Zeroizing` buffer, so the allocation is overwritten when it is dropped rather than merely moved out of.
Both controls require an Owner account, and both report a refusal as one: the TUI previously turned any failed listing — a role the daemon rejected, a vault it could not unlock, or a daemon that was not running — into the empty-list message, telling the operator this build has no writable vault keys.
(#8164, #8187) (@DaBlitzStein)
