Adds the `librefang-storage` crate, an opt-in SurrealDB 4.0 storage foundation: a serialisable storage configuration, a connection pool that hands out per-tenant sessions over one shared transport, and a versioned migration runner with the operational schema.
  SurrealDB 4.0 removed RocksDB, so the embedded engine is SurrealKV.
  Nothing consumes the crate yet and the default SQLite storage is unchanged. (#8707) (@GQAdonis)
