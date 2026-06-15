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
pub(crate) use crate::INIT_DEFAULT_CONFIG_TEMPLATE;

// Sibling crate-root modules, re-exported so handlers can reference them by
// bare name (`i18n::t`, `ui::success`, `table::…`, `doctor::…`, …) exactly as
// they did inside the old single-file main.rs.
pub(crate) use crate::{
    acp, desktop_install, doctor, http_client, i18n, launcher, log_filter, mcp, progress, table,
    templates, tui, ui,
};

// Command groups — re-exported so the dispatch match in `main.rs` and any
// cross-group handler call resolves without per-call-site imports.
pub(crate) use super::agent::*;
pub(crate) use super::auth::*;
pub(crate) use super::automation::*;
pub(crate) use super::channel::*;
pub(crate) use super::common::*;
pub(crate) use super::config::*;
pub(crate) use super::daemon::*;
pub(crate) use super::doctor_cmd::*;
pub(crate) use super::hand::*;
pub(crate) use super::init::*;
pub(crate) use super::maintenance::*;
pub(crate) use super::mcp_cmds::*;
pub(crate) use super::models::*;
pub(crate) use super::monitoring::*;
pub(crate) use super::skill::*;
pub(crate) use super::status::*;
pub(crate) use super::system::*;

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
