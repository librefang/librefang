//! CLI command handlers, split out of `main.rs` by domain.
//!
//! `main.rs` keeps `main()`, process/tracing setup, and the top-level
//! dispatch match; each submodule here owns one command group. Shared
//! helpers and the imports every handler needs are re-exported from
//! [`prelude`], which each module pulls in with `use crate::commands::prelude::*;`.

pub(crate) mod prelude;

pub(crate) mod common;
pub(crate) mod daemon;
pub(crate) mod status;
pub(crate) mod init;
