//! Configuration version tracking, providing the foundation for future config migrations.

/// Configuration file version number.
///
/// Version `2` migrates legacy configs that still store `api_key`,
/// `api_listen`, or `log_level` under `[api]` into the top-level schema.
pub const CONFIG_VERSION: u32 = 2;
