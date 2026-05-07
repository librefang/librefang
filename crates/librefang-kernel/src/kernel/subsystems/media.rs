//! Media subsystem — web search, browser automation, audio/video media.
//!
//! Bundles five engines that all sit behind the agent's media-related
//! tool calls: web search + SSRF-protected fetch (`web_ctx`), Playwright
//! sessions (`browser_ctx`), image/audio understanding (`media_engine`),
//! text-to-speech (`tts_engine`), and the media-generation driver cache
//! (`media_drivers`).

use librefang_runtime::browser::BrowserManager;
use librefang_runtime::media::MediaDriverCache;
use librefang_runtime::media_understanding::MediaEngine;
use librefang_runtime::tts::TtsEngine;
use librefang_runtime::web_search::WebToolsContext;

/// Web + browser + media + TTS cluster — see module docs.
pub struct MediaSubsystem {
    /// Web tools context (multi-provider search + SSRF-protected fetch + caching).
    pub(crate) web_ctx: WebToolsContext,
    /// Browser automation manager (Playwright bridge sessions).
    pub(crate) browser_ctx: BrowserManager,
    /// Media understanding engine (image description, audio transcription).
    pub(crate) media_engine: MediaEngine,
    /// Text-to-speech engine.
    pub(crate) tts_engine: TtsEngine,
    /// Media generation driver cache (video, music, etc.).
    pub(crate) media_drivers: MediaDriverCache,
}

impl MediaSubsystem {
    pub(crate) fn new(
        web_ctx: WebToolsContext,
        browser_ctx: BrowserManager,
        media_engine: MediaEngine,
        tts_engine: TtsEngine,
        media_drivers: MediaDriverCache,
    ) -> Self {
        Self {
            web_ctx,
            browser_ctx,
            media_engine,
            tts_engine,
            media_drivers,
        }
    }
}
