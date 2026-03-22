//! Configuration version tracking, providing the foundation for future config migrations.

/// Configuration file version number, used for future config migration support.
///
/// TODO: When an incompatible change is made to the config format, increment this version
/// and implement automatic migration logic in the loader (old version -> new version).
pub const CONFIG_VERSION: u32 = 1;
