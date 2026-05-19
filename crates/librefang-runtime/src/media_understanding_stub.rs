//! Stub `media_understanding` module for `--no-default-features` builds
//! (#3710 Phase 1).
//!
//! Exposes `MediaEngine` as a no-op shell with constructors that match
//! the real API. Methods return errors so any accidental hit is loud.
//! The dispatch arms for `media_describe` / `speech_to_text` /
//! `media_transcribe` are `#[cfg(feature = "media")]`-gated and never
//! reach these stubs when the feature is off.

#![allow(unused_variables, dead_code)]

use librefang_types::media::MediaConfig;

/// Mirror of `librefang_runtime_media::media_understanding::NOT_IMPLEMENTED_SENTINEL`.
/// Kept in sync so callers (channel-bridge enricher, agents.rs upload
/// enricher) can match on the same string when the `media` Cargo feature
/// is off. Stub builds never *produce* this sentinel (the stub returns a
/// different Err), but the constant must exist so the consumer compiles.
pub const NOT_IMPLEMENTED_SENTINEL: &str = "describe_image: not yet implemented (stub)";

pub struct MediaEngine;

impl MediaEngine {
    pub fn new(_config: MediaConfig) -> Self {
        Self
    }

    pub async fn describe_image(
        &self,
        _attachment: &impl std::any::Any,
    ) -> Result<MediaUnderstandingResult, String> {
        Err("media feature is disabled in this build".to_string())
    }

    pub async fn transcribe_audio(
        &self,
        _attachment: &impl std::any::Any,
    ) -> Result<MediaUnderstandingResult, String> {
        Err("media feature is disabled in this build".to_string())
    }

    pub async fn describe_video(
        &self,
        _attachment: &impl std::any::Any,
    ) -> Result<MediaUnderstandingResult, String> {
        Err("media feature is disabled in this build".to_string())
    }

    pub async fn process_attachments<I>(&self, _attachments: I) -> Vec<MediaUnderstandingResult> {
        Vec::new()
    }
}

pub struct MediaUnderstandingResult {
    pub text: String,
}
