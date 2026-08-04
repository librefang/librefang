Add a managed configuration mode so a deployment can own `config.toml` instead of treating it as application state.
`LIBREFANG_CONFIG_PATH` relocates the file — useful on its own, for a Compose bind mount or a ConfigMap mounted outside `LIBREFANG_HOME` — and `LIBREFANG_CONFIG_MODE=managed` locks it.
The two are deliberately independent: relocating a file is not a statement about who owns it, and inferring the lock from the path would hand a read-only dashboard to an operator who only wanted the file somewhere else.
The mode is read from the process environment and never from the config file, so a write through the API cannot unlock the very file it is being refused access to.
When managed mode is active, every API surface that persists deployment configuration answers `423 Locked` with `{"code": "config_managed", "source": "<path>"}` and leaves the file untouched — enforcement lives in the handlers rather than relying on a read-only mount, because a filesystem `EACCES` surfaces as a 500 with an errno and tells an operator nothing about why.
`GET /api/config/status` reports the mode, the source path, writability, a SHA-256 over the file's bytes, and its last-modified time, so the dashboard can present managed settings as read-only from server-supplied metadata rather than by attempting a save and reading the refusal back.
Boot-time schema migration no longer tries to write the migrated config back when the file is managed; it logs a single targeted warning instead.
That write previously failed against a read-only mount with nothing but a `warn!`, so the migration re-ran silently on every boot forever.
Mutable mode remains the default and is unchanged (#6695, #6717) (@houko)
