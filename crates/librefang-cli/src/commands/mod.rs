//! CLI command handlers, split out of `main.rs` by domain.
//!
//! `main.rs` keeps `main()`, process/tracing setup, and the top-level
//! dispatch match; each submodule here owns one command group. Shared
//! helpers and the imports every handler needs are re-exported from
//! [`prelude`], which each module pulls in with `use crate::commands::prelude::*;`.

pub(crate) mod agent;
pub(crate) mod auth;
pub(crate) mod automation;
pub(crate) mod channel;
pub(crate) mod common;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod doctor_cmd;
pub(crate) mod hand;
pub(crate) mod init;
pub(crate) mod maintenance;
pub(crate) mod mcp_cmds;
pub(crate) mod models;
pub(crate) mod monitoring;
pub(crate) mod prelude;
pub(crate) mod skill;
pub(crate) mod status;
pub(crate) mod system;
