//! Configuration for LibreFang telemetry.
//!
//! The canonical `TelemetryConfig` lives in `librefang-types::config::types`
//! alongside all other kernel configuration structs. This module re-exports it
//! for convenience so that code importing from `librefang_telemetry::config`
//! continues to work.

pub use librefang_types::config::TelemetryConfig;
