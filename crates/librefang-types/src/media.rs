//! Media understanding types — shared data structures for media processing.

use serde::{Deserialize, Serialize};

/// Supported media types for understanding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaType {
    Image,
    Audio,
    Video,
}

impl std::fmt::Display for MediaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaType::Image => write!(f, "image"),
            MediaType::Audio => write!(f, "audio"),
            MediaType::Video => write!(f, "video"),
        }
    }
}

/// Source of media content.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
#[non_exhaustive]
pub enum MediaSource {
    /// Path to a local file.
    FilePath { path: String },
    /// URL to fetch the media from (SSRF-checked).
    Url { url: String },
    /// Base64-encoded data.
    Base64 { data: String, mime_type: String },
}

/// A media attachment to be analyzed.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaAttachment {
    /// What kind of media this is.
    pub media_type: MediaType,
    /// MIME type (e.g., "image/png", "audio/mp3").
    pub mime_type: String,
    /// Where to get the media data.
    pub source: MediaSource,
    /// File size in bytes (for validation).
    pub size_bytes: u64,
}

/// Result of media analysis.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaUnderstanding {
    /// What type of media was analyzed.
    pub media_type: MediaType,
    /// Human-readable description or transcription.
    pub description: String,
    /// Which provider produced this result.
    pub provider: String,
    /// Which model was used.
    pub model: String,
}

/// Configuration for media understanding.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MediaConfig {
    /// Enable image description. Default: true.
    pub image_description: bool,
    /// Enable audio transcription. Default: true.
    pub audio_transcription: bool,
    /// Enable video description. Default: false (expensive).
    pub video_description: bool,
    /// Max concurrent media processing tasks. Default: 2.
    pub max_concurrency: usize,
    /// Preferred image description provider (auto-detect if None).
    pub image_provider: Option<String>,
    /// Preferred audio transcription provider (auto-detect if None).
    pub audio_provider: Option<String>,
    /// Preferred audio transcription model (provider default if None).
    pub audio_model: Option<String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            image_description: true,
            audio_transcription: true,
            video_description: false,
            max_concurrency: 2,
            image_provider: None,
            audio_provider: None,
            audio_model: None,
        }
    }
}

/// Configuration for link understanding.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LinkConfig {
    /// Enable automatic link understanding. Default: false.
    pub enabled: bool,
    /// Max links to process per message. Default: 3.
    pub max_links: usize,
    /// Max content size to fetch per link in bytes. Default: 100KB.
    pub max_content_bytes: usize,
    /// Timeout per link fetch in seconds. Default: 10.
    pub timeout_secs: u64,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_links: 3,
            max_content_bytes: 102_400,
            timeout_secs: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation constants (SECURITY)
// ---------------------------------------------------------------------------

/// Maximum image size in bytes (10 MB).
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// Maximum audio size in bytes (20 MB).
pub const MAX_AUDIO_BYTES: u64 = 20 * 1024 * 1024;
/// Maximum video size in bytes (50 MB).
pub const MAX_VIDEO_BYTES: u64 = 50 * 1024 * 1024;
/// Maximum base64 decoded size (70 MB).
pub const MAX_BASE64_DECODED_BYTES: u64 = 70 * 1024 * 1024;

/// Allowed image MIME types.
pub const ALLOWED_IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Allowed audio MIME types.
pub const ALLOWED_AUDIO_TYPES: &[&str] = &[
    "audio/mpeg",
    "audio/wav",
    "audio/ogg",
    "audio/mp4",
    "audio/webm",
    "audio/x-wav",
    "audio/flac",
];

/// Allowed video MIME types.
pub const ALLOWED_VIDEO_TYPES: &[&str] = &["video/mp4", "video/quicktime", "video/webm"];

/// Extract the bare `type/subtype` from a MIME string, discarding parameters
/// and whitespace (RFC 2045). E.g. `"audio/ogg; codecs=opus"` → `"audio/ogg"`.
pub fn mime_base(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or(mime)
        .trim()
        .to_ascii_lowercase()
}

impl MediaAttachment {
    /// Validate the attachment against security constraints.
    pub fn validate(&self) -> Result<(), String> {
        // Check MIME type allowlist — normalize to bare type/subtype so that
        // values like `audio/ogg; codecs=opus` (from WhatsApp) match the allowlist.
        let base = mime_base(&self.mime_type);
        let allowed = match self.media_type {
            MediaType::Image => ALLOWED_IMAGE_TYPES.contains(&base.as_str()),
            MediaType::Audio => ALLOWED_AUDIO_TYPES.contains(&base.as_str()),
            MediaType::Video => ALLOWED_VIDEO_TYPES.contains(&base.as_str()),
        };
        if !allowed {
            return Err(format!(
                "Unsupported MIME type '{}' for {:?} media",
                self.mime_type, self.media_type
            ));
        }

        // Check size limits
        let max_bytes = match self.media_type {
            MediaType::Image => MAX_IMAGE_BYTES,
            MediaType::Audio => MAX_AUDIO_BYTES,
            MediaType::Video => MAX_VIDEO_BYTES,
        };
        if self.size_bytes > max_bytes {
            return Err(format!(
                "{} file too large: {} bytes (max {} bytes)",
                self.media_type, self.size_bytes, max_bytes
            ));
        }

        Ok(())
    }
}

/// Supported image generation models.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ImageGenModel {
    #[default]
    DallE3,
    DallE2,
    #[serde(rename = "gpt-image-1")]
    GptImage1,
}

impl std::fmt::Display for ImageGenModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageGenModel::DallE3 => write!(f, "dall-e-3"),
            ImageGenModel::DallE2 => write!(f, "dall-e-2"),
            ImageGenModel::GptImage1 => write!(f, "gpt-image-1"),
        }
    }
}

/// Image generation request.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ImageGenRequest {
    /// The prompt describing the image to generate.
    pub prompt: String,
    /// Which model to use.
    #[serde(default)]
    pub model: ImageGenModel,
    /// Image size (e.g., "1024x1024").
    #[serde(default = "default_image_size")]
    pub size: String,
    /// Quality level (e.g., "standard", "hd").
    #[serde(default = "default_image_quality")]
    pub quality: String,
    /// Number of images to generate (1-4, DALL-E 3 only supports 1).
    #[serde(default = "default_image_count")]
    pub count: u8,
}

fn default_image_size() -> String {
    "1024x1024".to_string()
}

fn default_image_quality() -> String {
    "standard".to_string()
}

fn default_image_count() -> u8 {
    1
}

/// Allowed sizes per model.
pub const DALLE3_SIZES: &[&str] = &["1024x1024", "1792x1024", "1024x1792"];
pub const DALLE2_SIZES: &[&str] = &["256x256", "512x512", "1024x1024"];
pub const GPT_IMAGE1_SIZES: &[&str] = &["1024x1024", "1536x1024", "1024x1536"];

impl ImageGenRequest {
    /// Max prompt length in characters.
    pub const MAX_PROMPT_LEN: usize = 4000;

    /// Validate the request against model-specific constraints.
    pub fn validate(&self) -> Result<(), String> {
        // Prompt length
        if self.prompt.is_empty() {
            return Err("Image generation prompt cannot be empty".into());
        }
        if self.prompt.len() > Self::MAX_PROMPT_LEN {
            return Err(format!(
                "Prompt too long: {} chars (max {})",
                self.prompt.len(),
                Self::MAX_PROMPT_LEN
            ));
        }
        // Strip control chars check
        if self
            .prompt
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        {
            return Err("Prompt contains invalid control characters".into());
        }

        // Model-specific size validation
        let allowed_sizes = match self.model {
            ImageGenModel::DallE3 => DALLE3_SIZES,
            ImageGenModel::DallE2 => DALLE2_SIZES,
            ImageGenModel::GptImage1 => GPT_IMAGE1_SIZES,
        };
        if !allowed_sizes.contains(&self.size.as_str()) {
            return Err(format!(
                "Invalid size '{}' for {}. Allowed: {:?}",
                self.size, self.model, allowed_sizes
            ));
        }

        // Count validation
        match self.model {
            ImageGenModel::DallE3 => {
                if self.count != 1 {
                    return Err("DALL-E 3 only supports count=1".into());
                }
            }
            ImageGenModel::DallE2 | ImageGenModel::GptImage1 => {
                if self.count == 0 || self.count > 4 {
                    return Err(format!(
                        "Invalid count {} for {}. Must be 1-4",
                        self.count, self.model
                    ));
                }
            }
        }

        // Quality validation
        match self.model {
            ImageGenModel::DallE3 => {
                if self.quality != "standard" && self.quality != "hd" {
                    return Err(format!(
                        "Invalid quality '{}' for DALL-E 3. Must be 'standard' or 'hd'",
                        self.quality
                    ));
                }
            }
            _ => {
                if self.quality != "standard"
                    && self.quality != "auto"
                    && self.quality != "high"
                    && self.quality != "medium"
                    && self.quality != "low"
                {
                    return Err(format!(
                        "Invalid quality '{}'. Must be 'standard', 'auto', 'high', 'medium', or 'low'",
                        self.quality
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Result of image generation.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ImageGenResult {
    /// Generated images.
    pub images: Vec<GeneratedImage>,
    /// Which model was used.
    pub model: String,
    /// Revised prompt (DALL-E 3 rewrites prompts for quality).
    pub revised_prompt: Option<String>,
}

/// A single generated image.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GeneratedImage {
    /// Base64-encoded image data.
    pub data_base64: String,
    /// Temporary URL (may expire).
    pub url: Option<String>,
}

// ===========================================================================
// Media Generation — provider-agnostic request/result types
// ===========================================================================

/// What media capabilities a driver supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaCapability {
    ImageGeneration,
    TextToSpeech,
    VideoGeneration,
    MusicGeneration,
}

impl std::fmt::Display for MediaCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaCapability::ImageGeneration => write!(f, "image_generation"),
            MediaCapability::TextToSpeech => write!(f, "text_to_speech"),
            MediaCapability::VideoGeneration => write!(f, "video_generation"),
            MediaCapability::MusicGeneration => write!(f, "music_generation"),
        }
    }
}

/// Status of an async media generation task (e.g. video).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", tag = "state")]
#[non_exhaustive]
pub enum MediaTaskStatus {
    Pending,
    Queued,
    Processing,
    Completed,
    Failed { error: String },
}

impl std::fmt::Display for MediaTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaTaskStatus::Pending => write!(f, "pending"),
            MediaTaskStatus::Queued => write!(f, "queued"),
            MediaTaskStatus::Processing => write!(f, "processing"),
            MediaTaskStatus::Completed => write!(f, "completed"),
            MediaTaskStatus::Failed { error } => write!(f, "failed: {error}"),
        }
    }
}

// ── Image Generation (generic) ─────────────────────────────────────────

/// Provider-agnostic image generation request.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaImageRequest {
    /// Text prompt describing the image.
    pub prompt: String,
    /// Provider name (e.g. "openai", "minimax"). Auto-detected if None.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model ID (e.g. "dall-e-3", "image-01"). Uses provider default if None.
    #[serde(default)]
    pub model: Option<String>,
    /// Image width in pixels (provider-specific).
    #[serde(default)]
    pub width: Option<u32>,
    /// Image height in pixels (provider-specific).
    #[serde(default)]
    pub height: Option<u32>,
    /// Aspect ratio (e.g. "16:9", "1:1"). Takes priority over width/height.
    #[serde(default)]
    pub aspect_ratio: Option<String>,
    /// Quality level (e.g. "standard", "hd").
    #[serde(default)]
    pub quality: Option<String>,
    /// Number of images to generate.
    #[serde(default = "default_media_count")]
    pub count: u8,
    /// Seed for reproducibility (if supported).
    #[serde(default)]
    pub seed: Option<u64>,
}

fn default_media_count() -> u8 {
    1
}

impl MediaImageRequest {
    pub const MAX_PROMPT_LEN: usize = 4000;

    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.is_empty() {
            return Err("Image generation prompt cannot be empty".into());
        }
        if self.prompt.len() > Self::MAX_PROMPT_LEN {
            return Err(format!(
                "Prompt too long: {} chars (max {})",
                self.prompt.len(),
                Self::MAX_PROMPT_LEN
            ));
        }
        if self
            .prompt
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        {
            return Err("Prompt contains invalid control characters".into());
        }
        if self.count == 0 || self.count > 9 {
            return Err(format!("Invalid count {}. Must be 1-9", self.count));
        }
        Ok(())
    }
}

/// Provider-agnostic image generation result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaImageResult {
    /// Generated images.
    pub images: Vec<GeneratedImage>,
    /// Which model was used.
    pub model: String,
    /// Which provider was used.
    pub provider: String,
    /// Revised prompt (some providers rewrite prompts).
    pub revised_prompt: Option<String>,
}

// ── Text-to-Speech (generic) ───────────────────────────────────────────

/// Provider-agnostic TTS request.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaTtsRequest {
    /// Text to synthesize.
    pub text: String,
    /// Provider name. Auto-detected if None.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model ID (e.g. "tts-1-hd", "speech-2.8-hd").
    #[serde(default)]
    pub model: Option<String>,
    /// Voice ID or name (provider-specific).
    #[serde(default)]
    pub voice: Option<String>,
    /// Speech speed multiplier (e.g. 0.5–2.0).
    #[serde(default)]
    pub speed: Option<f32>,
    /// Output audio format (e.g. "mp3", "wav", "flac").
    #[serde(default)]
    pub format: Option<String>,
    /// Language hint (e.g. "en", "zh", "ja").
    #[serde(default)]
    pub language: Option<String>,
    /// Pitch adjustment (e.g. -20.0 to 20.0 for Google TTS). Provider-specific.
    #[serde(default)]
    pub pitch: Option<f32>,
}

impl MediaTtsRequest {
    pub const MAX_TEXT_LEN: usize = 10_000;

    pub fn validate(&self) -> Result<(), String> {
        if self.text.is_empty() {
            return Err("TTS text cannot be empty".into());
        }
        if self.text.len() > Self::MAX_TEXT_LEN {
            return Err(format!(
                "Text too long: {} chars (max {})",
                self.text.len(),
                Self::MAX_TEXT_LEN
            ));
        }
        if let Some(speed) = self.speed {
            if !(0.25..=4.0).contains(&speed) {
                return Err(format!("Invalid speed {speed}. Must be 0.25-4.0"));
            }
        }
        if let Some(pitch) = self.pitch {
            if !(-20.0..=20.0).contains(&pitch) {
                return Err(format!("Invalid pitch {pitch}. Must be -20.0 to 20.0"));
            }
        }
        Ok(())
    }
}

/// Provider-agnostic TTS result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaTtsResult {
    /// Raw audio bytes.
    #[serde(skip)]
    pub audio_data: Vec<u8>,
    /// Audio format (e.g. "mp3").
    pub format: String,
    /// Provider that produced this.
    pub provider: String,
    /// Model used.
    pub model: String,
    /// Estimated duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Sample rate in Hz.
    pub sample_rate: Option<u32>,
}

// ── Video Generation (generic, async) ──────────────────────────────────

/// Provider-agnostic video generation request.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaVideoRequest {
    /// Text prompt describing the video.
    pub prompt: String,
    /// Provider name. Auto-detected if None.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model ID (e.g. "MiniMax-Hailuo-2.3").
    #[serde(default)]
    pub model: Option<String>,
    /// Video duration in seconds.
    #[serde(default)]
    pub duration_secs: Option<u32>,
    /// Resolution (e.g. "720P", "1080P").
    #[serde(default)]
    pub resolution: Option<String>,
    /// Reference image URL for image-to-video.
    #[serde(default)]
    pub image_url: Option<String>,
    /// Whether to optimize the prompt automatically.
    #[serde(default)]
    pub optimize_prompt: Option<bool>,
}

impl MediaVideoRequest {
    pub const MAX_PROMPT_LEN: usize = 2000;

    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.is_empty() && self.image_url.is_none() {
            return Err("Video generation requires a prompt or reference image".into());
        }
        if self.prompt.len() > Self::MAX_PROMPT_LEN {
            return Err(format!(
                "Prompt too long: {} chars (max {})",
                self.prompt.len(),
                Self::MAX_PROMPT_LEN
            ));
        }
        if let Some(d) = self.duration_secs {
            if d == 0 || d > 60 {
                return Err(format!("Invalid duration {d}s. Must be 1-60"));
            }
        }
        Ok(())
    }
}

/// Result of a video generation submit (returns task ID for polling).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaVideoSubmitResult {
    /// Task ID for polling status.
    pub task_id: String,
    /// Provider that accepted the task.
    pub provider: String,
}

/// Result of a completed video generation.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaVideoResult {
    /// URL to download the video (may expire).
    pub file_url: String,
    /// Video width in pixels.
    pub width: Option<u32>,
    /// Video height in pixels.
    pub height: Option<u32>,
    /// Video duration in seconds.
    pub duration_secs: Option<u32>,
    /// Provider that produced this.
    pub provider: String,
    /// Model used.
    pub model: String,
}

// ── Music Generation (generic) ─────────────────────────────────────────

/// Provider-agnostic music generation request.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaMusicRequest {
    /// Style/mood description (e.g. "upbeat pop song").
    #[serde(default)]
    pub prompt: Option<String>,
    /// Song lyrics with optional structure tags ([Verse], [Chorus], etc.).
    #[serde(default)]
    pub lyrics: Option<String>,
    /// Provider name. Auto-detected if None.
    #[serde(default)]
    pub provider: Option<String>,
    /// Model ID (e.g. "music-2.5").
    #[serde(default)]
    pub model: Option<String>,
    /// Generate instrumental only (no vocals).
    #[serde(default)]
    pub instrumental: bool,
    /// Output audio format (e.g. "mp3", "wav").
    #[serde(default)]
    pub format: Option<String>,
}

impl MediaMusicRequest {
    pub const MAX_PROMPT_LEN: usize = 2000;
    pub const MAX_LYRICS_LEN: usize = 3500;

    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.is_none() && self.lyrics.is_none() {
            return Err("Music generation requires a prompt or lyrics".into());
        }
        if let Some(ref p) = self.prompt {
            if p.len() > Self::MAX_PROMPT_LEN {
                return Err(format!(
                    "Prompt too long: {} chars (max {})",
                    p.len(),
                    Self::MAX_PROMPT_LEN
                ));
            }
        }
        if let Some(ref l) = self.lyrics {
            if l.len() > Self::MAX_LYRICS_LEN {
                return Err(format!(
                    "Lyrics too long: {} chars (max {})",
                    l.len(),
                    Self::MAX_LYRICS_LEN
                ));
            }
        }
        Ok(())
    }
}

/// Provider-agnostic music generation result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MediaMusicResult {
    /// Raw audio bytes.
    #[serde(skip)]
    pub audio_data: Vec<u8>,
    /// Audio format (e.g. "mp3").
    pub format: String,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Provider that produced this.
    pub provider: String,
    /// Model used.
    pub model: String,
    /// Sample rate in Hz.
    pub sample_rate: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_type_display() {
        assert_eq!(MediaType::Image.to_string(), "image");
        assert_eq!(MediaType::Audio.to_string(), "audio");
        assert_eq!(MediaType::Video.to_string(), "video");
    }

    #[test]
    fn test_media_config_default() {
        let config = MediaConfig::default();
        assert!(config.image_description);
        assert!(config.audio_transcription);
        assert!(!config.video_description);
        assert_eq!(config.max_concurrency, 2);
        assert!(config.image_provider.is_none());
    }

    #[test]
    fn test_link_config_default() {
        let config = LinkConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.max_links, 3);
        assert_eq!(config.max_content_bytes, 102_400);
        assert_eq!(config.timeout_secs, 10);
    }

    #[test]
    fn test_attachment_validate_valid_image() {
        let a = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".to_string(),
            source: MediaSource::FilePath {
                path: "test.png".to_string(),
            },
            size_bytes: 1024,
        };
        assert!(a.validate().is_ok());
    }

    #[test]
    fn test_attachment_validate_mime_with_parameters() {
        // WhatsApp voice notes arrive as `audio/ogg; codecs=opus`.
        // The allowlist stores the bare `audio/ogg`, so validation must
        // normalize parameters away before comparing.
        let a = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/ogg; codecs=opus".to_string(),
            source: MediaSource::FilePath {
                path: "voice.ogg".to_string(),
            },
            size_bytes: 1024,
        };
        assert!(a.validate().is_ok());

        // Uppercase type/subtype and spacing must also work.
        let a = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "Audio/OGG ;codecs=opus".to_string(),
            source: MediaSource::FilePath {
                path: "voice.ogg".to_string(),
            },
            size_bytes: 1024,
        };
        assert!(a.validate().is_ok());
    }

    #[test]
    fn test_attachment_validate_bad_mime() {
        let a = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "application/pdf".to_string(),
            source: MediaSource::FilePath {
                path: "test.pdf".to_string(),
            },
            size_bytes: 1024,
        };
        assert!(a.validate().is_err());
    }

    #[test]
    fn test_attachment_validate_too_large() {
        let a = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".to_string(),
            source: MediaSource::FilePath {
                path: "big.png".to_string(),
            },
            size_bytes: MAX_IMAGE_BYTES + 1,
        };
        assert!(a.validate().is_err());
    }

    #[test]
    fn test_attachment_validate_audio() {
        let a = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/mpeg".to_string(),
            source: MediaSource::Url {
                url: "https://example.com/a.mp3".to_string(),
            },
            size_bytes: 5_000_000,
        };
        assert!(a.validate().is_ok());
    }

    #[test]
    fn test_attachment_validate_video_too_large() {
        let a = MediaAttachment {
            media_type: MediaType::Video,
            mime_type: "video/mp4".to_string(),
            source: MediaSource::FilePath {
                path: "big.mp4".to_string(),
            },
            size_bytes: MAX_VIDEO_BYTES + 1,
        };
        assert!(a.validate().is_err());
    }

    #[test]
    fn test_image_gen_model_display() {
        assert_eq!(ImageGenModel::DallE3.to_string(), "dall-e-3");
        assert_eq!(ImageGenModel::DallE2.to_string(), "dall-e-2");
        assert_eq!(ImageGenModel::GptImage1.to_string(), "gpt-image-1");
    }

    #[test]
    fn test_image_gen_request_validate_valid() {
        let req = ImageGenRequest {
            prompt: "A sunset over mountains".to_string(),
            model: ImageGenModel::DallE3,
            size: "1024x1024".to_string(),
            quality: "hd".to_string(),
            count: 1,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_image_gen_request_validate_empty_prompt() {
        let req = ImageGenRequest {
            prompt: String::new(),
            model: ImageGenModel::DallE3,
            size: "1024x1024".to_string(),
            quality: "standard".to_string(),
            count: 1,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_image_gen_request_validate_bad_size() {
        let req = ImageGenRequest {
            prompt: "test".to_string(),
            model: ImageGenModel::DallE3,
            size: "512x512".to_string(),
            quality: "standard".to_string(),
            count: 1,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_image_gen_request_validate_dalle3_count() {
        let req = ImageGenRequest {
            prompt: "test".to_string(),
            model: ImageGenModel::DallE3,
            size: "1024x1024".to_string(),
            quality: "standard".to_string(),
            count: 2,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_image_gen_request_validate_dalle2_multi() {
        let req = ImageGenRequest {
            prompt: "test".to_string(),
            model: ImageGenModel::DallE2,
            size: "512x512".to_string(),
            quality: "standard".to_string(),
            count: 4,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_image_gen_request_validate_control_chars() {
        let req = ImageGenRequest {
            prompt: "test\x00prompt".to_string(),
            model: ImageGenModel::DallE3,
            size: "1024x1024".to_string(),
            quality: "standard".to_string(),
            count: 1,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_type_serde_roundtrip() {
        let mt = MediaType::Audio;
        let json = serde_json::to_string(&mt).unwrap();
        assert_eq!(json, "\"audio\"");
        let parsed: MediaType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, mt);
    }

    #[test]
    fn test_image_gen_model_serde_roundtrip() {
        let m = ImageGenModel::GptImage1;
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "\"gpt-image-1\"");
        let parsed: ImageGenModel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn test_media_config_serde_roundtrip() {
        let config = MediaConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: MediaConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.max_concurrency, 2);
        assert!(parsed.image_description);
    }

    // ── Media generation types tests ───────────────────────────────────

    #[test]
    fn test_media_capability_display() {
        assert_eq!(
            MediaCapability::ImageGeneration.to_string(),
            "image_generation"
        );
        assert_eq!(MediaCapability::TextToSpeech.to_string(), "text_to_speech");
        assert_eq!(
            MediaCapability::VideoGeneration.to_string(),
            "video_generation"
        );
        assert_eq!(
            MediaCapability::MusicGeneration.to_string(),
            "music_generation"
        );
    }

    #[test]
    fn test_media_capability_serde_roundtrip() {
        let cap = MediaCapability::VideoGeneration;
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, "\"video_generation\"");
        let parsed: MediaCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cap);
    }

    #[test]
    fn test_media_task_status_display() {
        assert_eq!(MediaTaskStatus::Pending.to_string(), "pending");
        assert_eq!(MediaTaskStatus::Queued.to_string(), "queued");
        assert_eq!(MediaTaskStatus::Processing.to_string(), "processing");
        assert_eq!(MediaTaskStatus::Completed.to_string(), "completed");
        assert_eq!(
            MediaTaskStatus::Failed {
                error: "timeout".into()
            }
            .to_string(),
            "failed: timeout"
        );
    }

    #[test]
    fn test_media_image_request_validate_valid() {
        let req = MediaImageRequest {
            prompt: "A cat in space".into(),
            provider: None,
            model: None,
            width: None,
            height: None,
            aspect_ratio: Some("16:9".into()),
            quality: None,
            count: 1,
            seed: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_media_image_request_validate_empty() {
        let req = MediaImageRequest {
            prompt: String::new(),
            provider: None,
            model: None,
            width: None,
            height: None,
            aspect_ratio: None,
            quality: None,
            count: 1,
            seed: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_image_request_validate_bad_count() {
        let req = MediaImageRequest {
            prompt: "test".into(),
            provider: None,
            model: None,
            width: None,
            height: None,
            aspect_ratio: None,
            quality: None,
            count: 0,
            seed: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_tts_request_validate_valid() {
        let req = MediaTtsRequest {
            text: "Hello world".into(),
            provider: None,
            model: None,
            voice: Some("alloy".into()),
            speed: Some(1.0),
            format: None,
            language: None,
            pitch: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_media_tts_request_validate_empty() {
        let req = MediaTtsRequest {
            text: String::new(),
            provider: None,
            model: None,
            voice: None,
            speed: None,
            format: None,
            language: None,
            pitch: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_tts_request_validate_bad_speed() {
        let req = MediaTtsRequest {
            text: "Hello".into(),
            provider: None,
            model: None,
            voice: None,
            speed: Some(10.0),
            format: None,
            language: None,
            pitch: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_video_request_validate_valid() {
        let req = MediaVideoRequest {
            prompt: "A sunset timelapse".into(),
            provider: None,
            model: None,
            duration_secs: Some(6),
            resolution: Some("1080P".into()),
            image_url: None,
            optimize_prompt: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_media_video_request_validate_no_input() {
        let req = MediaVideoRequest {
            prompt: String::new(),
            provider: None,
            model: None,
            duration_secs: None,
            resolution: None,
            image_url: None,
            optimize_prompt: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_video_request_validate_bad_duration() {
        let req = MediaVideoRequest {
            prompt: "test".into(),
            provider: None,
            model: None,
            duration_secs: Some(120),
            resolution: None,
            image_url: None,
            optimize_prompt: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_music_request_validate_valid() {
        let req = MediaMusicRequest {
            prompt: Some("upbeat pop".into()),
            lyrics: Some("[Verse]\nHello world".into()),
            provider: None,
            model: None,
            instrumental: false,
            format: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_media_music_request_validate_no_input() {
        let req = MediaMusicRequest {
            prompt: None,
            lyrics: None,
            provider: None,
            model: None,
            instrumental: false,
            format: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_media_music_request_validate_instrumental() {
        let req = MediaMusicRequest {
            prompt: Some("jazz piano".into()),
            lyrics: None,
            provider: None,
            model: None,
            instrumental: true,
            format: Some("mp3".into()),
        };
        assert!(req.validate().is_ok());
    }
}
