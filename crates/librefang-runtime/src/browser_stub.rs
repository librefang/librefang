//! Stub `browser` module for `--no-default-features` builds (#3710 Phase 1).
//!
//! Exposes `BrowserManager` as a no-op shell so consumer code (kernel
//! boot, `ToolExecutionContext.browser_ctx`, channel bridges that hold
//! the manager by reference) keeps compiling. The dispatch layer for
//! `browser_*` tools is `#[cfg(feature = "browser")]`-gated and never
//! reaches these stubs at runtime when the feature is off.

#![allow(unused_variables, dead_code)]

pub struct BrowserManager;

impl BrowserManager {
    pub fn new(_config: librefang_types::config::BrowserConfig) -> Self {
        Self
    }

    pub fn has_session(&self, _agent_id: &str) -> bool {
        false
    }

    pub async fn close_session(&self, _agent_id: &str) {}

    pub async fn cleanup_agent(&self, _agent_id: &str) {}
}

/// Stand-in for the real command enum; only the variant names are needed
/// by consumers that pass them by value into `send_command`. With the
/// `browser` feature off, no command ever reaches the manager.
#[derive(Debug, Clone, Copy)]
pub enum BrowserCommand {
    ReadPage,
    Screenshot,
}
