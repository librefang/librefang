//! ElevenLabs media generation driver.
//!
//! Supports:
//! - TTS via `POST /v1/text-to-speech/{voice_id}` (multilingual_v2, turbo_v2_5, etc.)
//!
//! Image, video, and music generation are not supported.
//!
//! API docs: <https://elevenlabs.io/docs/api-reference/text-to-speech>

use super::{MediaDriver, MediaError};
use async_trait::async_trait;
use librefang_types::media::{MediaCapability, MediaTtsRequest, MediaTtsResult};

/// Default ElevenLabs API base URL.
const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io/v1";

/// Default TTS model.
const DEFAULT_MODEL: &str = "eleven_multilingual_v2";

/// Default voice ID (Rachel).
const DEFAULT_VOICE_ID: &str = "21m00Tcm4TlvDq8ikWAM";

/// Max audio response size (25 MB).
const MAX_AUDIO_RESPONSE_BYTES: usize = 25 * 1024 * 1024;

pub struct ElevenLabsMediaDriver {
    base_url: String,
}

impl ElevenLabsMediaDriver {
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
        }
    }

    fn api_key() -> Result<String, MediaError> {
        std::env::var("ELEVENLABS_API_KEY").map_err(|_| {
            MediaError::MissingKey(
                "ELEVENLABS_API_KEY not set. Get one at https://elevenlabs.io".into(),
            )
        })
    }
}

#[async_trait]
impl MediaDriver for ElevenLabsMediaDriver {
    fn capabilities(&self) -> Vec<MediaCapability> {
        vec![MediaCapability::TextToSpeech]
    }

    fn is_configured(&self) -> bool {
        Self::api_key().is_ok()
    }

    fn provider_name(&self) -> &str {
        "elevenlabs"
    }

    // ── Text-to-speech ────────────────────────────────────────────────

    async fn synthesize_speech(
        &self,
        request: &MediaTtsRequest,
    ) -> Result<MediaTtsResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        let model = request.model.as_deref().unwrap_or(DEFAULT_MODEL);
        let voice_id = request.voice.as_deref().unwrap_or(DEFAULT_VOICE_ID);
        let format = request.format.as_deref().unwrap_or("mp3_44100_128");

        let mut body = serde_json::json!({
            "text": request.text,
            "model_id": model,
        });

        // Voice settings
        let mut voice_settings = serde_json::json!({
            "stability": 0.5,
            "similarity_boost": 0.75,
        });
        if let Some(speed) = request.speed {
            // ElevenLabs doesn't have a direct speed param, but we can
            // influence it via stability (lower = more expressive/varied)
            voice_settings["stability"] = serde_json::json!(if speed > 1.0 { 0.3 } else { 0.7 });
        }
        body["voice_settings"] = voice_settings;

        if let Some(ref lang) = request.language {
            body["language_code"] = serde_json::json!(lang);
        }

        let url = format!(
            "{}/text-to-speech/{}?output_format={}",
            self.base_url, voice_id, format
        );

        let client = crate::http_client::proxied_client();
        let response = client
            .post(&url)
            .header("xi-api-key", &api_key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| MediaError::Http(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let err = response.text().await.unwrap_or_default();
            let truncated = crate::str_utils::safe_truncate_str(&err, 500);
            return Err(MediaError::Api {
                status,
                message: truncated.to_string(),
            });
        }

        if let Some(len) = response.content_length() {
            if len as usize > MAX_AUDIO_RESPONSE_BYTES {
                return Err(MediaError::Other(format!(
                    "Audio response too large: {len} bytes (max {MAX_AUDIO_RESPONSE_BYTES})"
                )));
            }
        }

        let audio_data = response
            .bytes()
            .await
            .map_err(|e| MediaError::Http(format!("Failed to read audio response: {e}")))?
            .to_vec();

        if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
            return Err(MediaError::Other(format!(
                "Audio data exceeds {}MB limit",
                MAX_AUDIO_RESPONSE_BYTES / 1024 / 1024
            )));
        }

        // Rough duration estimate: ~150 words/min
        let word_count = request.text.split_whitespace().count();
        let duration_ms = (word_count as u64 * 400).max(500);

        // Parse sample rate from output format string (e.g. "mp3_44100_128")
        let (audio_format, sample_rate) = parse_output_format(format);

        Ok(MediaTtsResult {
            audio_data,
            format: audio_format,
            provider: "elevenlabs".to_string(),
            model: model.to_string(),
            duration_ms: Some(duration_ms),
            sample_rate,
        })
    }
}

/// Parse ElevenLabs output format string (e.g. "mp3_44100_128") into
/// (format, sample_rate).
fn parse_output_format(fmt: &str) -> (String, Option<u32>) {
    let parts: Vec<&str> = fmt.split('_').collect();
    let format = parts.first().unwrap_or(&"mp3").to_string();
    let sample_rate = parts.get(1).and_then(|s| s.parse::<u32>().ok());
    (format, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_capabilities() {
        let driver = ElevenLabsMediaDriver::new(None);
        let caps = driver.capabilities();
        assert_eq!(caps.len(), 1);
        assert!(caps.contains(&MediaCapability::TextToSpeech));
        assert!(!caps.contains(&MediaCapability::ImageGeneration));
    }

    #[test]
    fn test_driver_provider_name() {
        let driver = ElevenLabsMediaDriver::new(None);
        assert_eq!(driver.provider_name(), "elevenlabs");
    }

    #[test]
    fn test_driver_custom_base_url() {
        let driver = ElevenLabsMediaDriver::new(Some("https://custom.api/v1/"));
        assert_eq!(driver.base_url, "https://custom.api/v1");
    }

    #[test]
    fn test_parse_output_format() {
        let (fmt, sr) = parse_output_format("mp3_44100_128");
        assert_eq!(fmt, "mp3");
        assert_eq!(sr, Some(44100));

        let (fmt, sr) = parse_output_format("pcm_16000");
        assert_eq!(fmt, "pcm");
        assert_eq!(sr, Some(16000));

        let (fmt, sr) = parse_output_format("mp3");
        assert_eq!(fmt, "mp3");
        assert_eq!(sr, None);
    }

    #[tokio::test]
    async fn test_image_not_supported() {
        let driver = ElevenLabsMediaDriver::new(None);
        let req = librefang_types::media::MediaImageRequest {
            prompt: "test".into(),
            provider: None,
            model: None,
            width: None,
            height: None,
            aspect_ratio: None,
            quality: None,
            count: 1,
            seed: None,
        };
        let result = driver.generate_image(&req).await;
        assert!(matches!(result, Err(MediaError::NotSupported(_))));
    }

    #[tokio::test]
    async fn test_video_not_supported() {
        let driver = ElevenLabsMediaDriver::new(None);
        let req = librefang_types::media::MediaVideoRequest {
            prompt: "test".into(),
            provider: None,
            model: None,
            duration_secs: None,
            resolution: None,
            image_url: None,
            optimize_prompt: None,
        };
        let result = driver.submit_video(&req).await;
        assert!(matches!(result, Err(MediaError::NotSupported(_))));
    }
}
