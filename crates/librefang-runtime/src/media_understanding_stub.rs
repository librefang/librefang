//! Stub `media_understanding` module for `--no-default-features` builds
//! (#3710 Phase 1).
//!
//! Exposes `MediaEngine` as a no-op shell with constructors that match
//! the real API. Methods return errors so any accidental hit is loud.
//! The dispatch arms for `media_describe` / `speech_to_text` /
//! `media_transcribe` are `#[cfg(feature = "media")]`-gated and never
//! reach these stubs when the feature is off.

#![allow(unused_variables, dead_code)]

use librefang_types::media::{CapabilityRouting, MediaConfig};

pub struct MediaEngine;

impl MediaEngine {
    pub fn new(_config: MediaConfig) -> Self {
        Self
    }

    pub fn with_capability_routing(&self, _routing: &CapabilityRouting) -> Self {
        Self
    }

    /// Always `false` in this build: with the `media` feature off there is no
    /// engine to describe anything, so callers must take their no-media path
    /// rather than attempting a call that would only ever error.
    pub fn image_description_enabled(&self) -> bool {
        false
    }

    pub fn audio_transcription_enabled(&self) -> bool {
        false
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
