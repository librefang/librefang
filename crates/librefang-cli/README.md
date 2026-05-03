# librefang-cli

Command-line interface for the [LibreFang](https://github.com/librefang/librefang) Agent OS.

Ships the `librefang` binary. When a daemon is running
(`librefang start`), the CLI talks to it over HTTP at
`http://127.0.0.1:4545` by default. Otherwise, commands boot an
in-process kernel for single-shot operation.

## Common commands

- `librefang start` — start the daemon (HTTP API + dashboard).
- `librefang init` — write a starter `~/.librefang/config.toml`.
- `librefang agent <subcommand>` — spawn / list / message agents.
- `librefang doctor` — diagnose the local environment.

Run `librefang help` (or any subcommand with `--help`) for the full
catalog.

## Key dependencies

`librefang-types`, `librefang-http`, `clap`, `reqwest`, `tokio`,
`tikv-jemallocator` (non-MSVC global allocator). Channel adapters are
feature-gated; the default set covers `telegram`, `discord`, `slack`,
`webhook`, `ntfy`. Build with `--features all-channels` for the full
~25-channel set.

See the [workspace README](../../README.md).
