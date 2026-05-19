//! Stub `media` module for `--no-default-features` builds (#3710 Phase 1).
//!
//! Exposes the bare minimum type / constructor surface that consumer code
//! (kernel boot, agent loop context, integration tests) holds by reference
//! or constructs unconditionally. All methods are no-ops; the dispatch
//! layer for media tools (`media_describe`, `image_generate`, …) is
//! `#[cfg(feature = "media")]`-gated and never reaches these stubs at
//! runtime when the feature is off.

#![allow(unused_variables, dead_code)]

/// Empty stand-in for the real `MediaDriverCache`. Held by reference in
/// `ToolExecutionContext` and constructed at kernel boot; with the
/// `media` feature off the cache holds no drivers and tool dispatch
/// never reaches the methods that would touch it.
#[derive(Default)]
pub struct MediaDriverCache;

impl MediaDriverCache {
    pub fn new() -> Self {
        Self
    }

    pub fn new_with_urls<I>(_urls: I) -> Self {
        Self
    }

    pub fn load_providers_from_registry(&self, _registry: &impl std::any::Any) {}

    pub fn clear(&self) {}

    pub fn update_provider_urls<I>(&self, _urls: I) {}
}
