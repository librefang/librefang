# Change C-005d.3 — Persist channel (sidecar) settings to the DB store

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-d3 · **Effort:** M · **Depends on:** C-005d.1 · **Agent:** claude

## Goal
`POST /api/channels/sidecar/{name}/configure` persists the non-secret
`[[sidecar_channels]]` structure to the DB config store instead of `config.toml`,
so channel config works under the read-only ConfigMap. Secrets stay in
`secrets.env` (unchanged).

## Design
`configure_sidecar_channel` (`crates/librefang-api/src/routes/channels.rs:809`)
ALREADY splits the payload: secret values → `~/.librefang/secrets.env`
(`channels.rs:862`), non-secrets → a `[[sidecar_channels]]` block in
`config.toml` via `sidecar_toml::upsert_sidecar_block`. Keep the `secrets.env`
write EXACTLY as-is. Replace only the `config.toml` write (surreal branch):

1. Read the current `Vec<SidecarChannelConfig>` from `kernel.config_ref()`.
2. Upsert the named block in memory (mirror `upsert_sidecar_block` semantics —
   schema-managed env keys only; non-schema keys preserved).
3. Store the resulting array as one `config_overrides` entry `sidecar_channels`.
4. `resolve_config_with_overrides` (trusted section) → `replace_config`.
5. Keep the existing `reload_channels_from_disk` bridge-manager restart.

sqlite-only build keeps the `config.toml` path.

## Security (the crux)
- **NO secret VALUE may enter the `sidecar_channels` override.** Only the
  schema-managed non-secret keys + `*_env` pointers. Secrets stay in
  `secrets.env`. A test must assert no secret-valued field is present in the
  stored override.
- Preserve the **include-file shadow guard** (`channels.rs:805,880`): refuse if an
  included file already declares a `[[sidecar_channels]]` entry, so the DB write
  cannot silently shadow it. (R-d1.)

## Files
- `crates/librefang-api/src/routes/channels.rs` (`configure_sidecar_channel`)
- possibly `crates/librefang-api/src/routes/sidecar_toml.rs` (extract the
  in-memory upsert if needed)
- `crates/librefang-api/tests/config_store_overlay_test.rs` (tests)

## Tasks
- [ ] cfg-branch the config.toml write only; secrets.env write unchanged.
- [ ] In-memory `sidecar_channels` upsert-by-name → store override + resolve +
  `replace_config`.
- [ ] Preserve the include-shadow guard before the store write.
- [ ] Tests: configure → `sidecar_channels` override applied + live config has the
  channel; `config.toml` untouched; bridge reload still fires; **security** — no
  secret-valued field in the stored override.

## Done when
`cargo test -p librefang-api` green; under surreal `configure_sidecar_channel`
writes only `secrets.env` (secrets) + the DB store (structure), never
`config.toml`.
