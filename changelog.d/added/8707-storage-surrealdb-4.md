Adds the `librefang-storage` crate, an opt-in SurrealDB 4.0 storage foundation: a serialisable storage configuration, a connection pool that hands out per-tenant sessions over one shared transport, and a versioned migration runner with the operational schema.
  SurrealDB 4.0 removed RocksDB, so the embedded engine is SurrealKV.
  The schema tracks the SQLite ladder through v65, including the per-task claim TTL, the truncated flag on ephemeral runs, the per-session reasoning-effort pin and the columns and indexes earlier SQLite steps added, so a SurrealDB backend built on it does not silently drop those fields.
  The SurrealDB 4 crates are BUSL-1.1, so `deny.toml` gains per-crate license exceptions for them, plus a BSL-1.0 exception for `xxhash-rust`, which the embedded engine pulls in.
  Nothing consumes the crate yet and the default SQLite storage is unchanged. (#8707) (@GQAdonis)
