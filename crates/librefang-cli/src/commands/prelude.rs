//! Shared imports for the CLI command modules and `main.rs` dispatch.
//!
//! Every command module starts with `use crate::commands::prelude::*;`. This
//! re-exports the clap definitions ([`crate::cli`]), the cross-cutting helpers
//! in [`super::common`], the handful of `main.rs`-resident items handlers call,
//! and the std/external symbols handlers reference by short name. As more
//! command groups are split out of `main.rs`, their modules are re-exported
//! here too so cross-group calls resolve without per-call-site imports.
//!
//! `allow(unused_imports)` is deliberate and scoped to this prelude: it exists
//! to re-export for consumer convenience, and not every consumer uses every
//! item. Consumers glob-import it (glob imports are already unused-exempt).
#![allow(unused_imports)]

pub(crate) use crate::cli::*;
pub(crate) use crate::install_ctrlc_handler;
pub(crate) use super::common::*;

pub(crate) use colored::Colorize;
pub(crate) use librefang_api::server::read_daemon_info;
pub(crate) use librefang_extensions::dotenv;
pub(crate) use librefang_kernel::{
    config::load_config, AgentSubsystemApi, LibreFangKernel, LlmSubsystemApi,
};
pub(crate) use librefang_types::agent::{AgentId, AgentManifest};
pub(crate) use std::ffi::OsString;
pub(crate) use std::io::{self, BufRead, Write};
pub(crate) use std::path::PathBuf;
pub(crate) use std::process::Stdio;
pub(crate) use std::sync::atomic::AtomicBool;
pub(crate) use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
