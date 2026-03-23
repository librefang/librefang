//! OpenAI media generation driver.
//!
//! Supports:
//! - Image generation via `POST /v1/images/generations` (DALL-E 3, DALL-E 2, gpt-image-1)
//! - TTS via `POST /v1/audio/speech` (tts-1, tts-1-hd)
//!
//! Video and music generation are not supported by OpenAI.

use async_trait::async_trait;
use librefang_types::media::{
    GeneratedImage, MediaCapability, MediaImageRequest, MediaImageResult, MediaTtsRequest,
    MediaTtsResult,
};
use tracing::warn;

use super::{MediaDriver, MediaError};

/// Default OpenAI API base URL.
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Max image base64 size (10 MB).
const MAX_IMAGE_B64_BYTES: usize = 10 * 1024 * 1024;

/// Max audio response size (10 MB).
const MAX_AUDIO_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

pub struct OpenAIMediaDriver {
    base_url: String,
}

impl OpenAIMediaDriver {
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
        }
    }

    fn api_key() -> Result<String, MediaError> {
        std::env::var("OPENAI_API_KEY").map_err(|_| {
            MediaError::MissingKey(
                "OPENAI_API_KEY not set. Image and TTS generation require an OpenAI API key."
                    .into(),
            )
        })
    }
}

#[async_trait]
impl MediaDriver for OpenAIMediaDriver {
    fn capabilities(&self) -> Vec<MediaCapability> {
        vec![
            MediaCapability::ImageGeneration,
            MediaCapability::TextToSpeech,
        ]
    }

    fn is_configured(&self) -> bool {
        Self::api_key().is_ok()
    }

    fn provider_name(&self) -> &str {
        "openai"
    }

    // ── Image generation ───────────────────────────────────────────

    async fn generate_image(
        &self,
        request: &MediaImageRequest,
    ) -> Result<MediaImageResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        let model = request.model.as_deref().unwrap_or("dall-e-3");

        // Build size from width/height or aspect_ratio, defaulting to 1024x1024
        let size = if let (Some(w), Some(h)) = (request.width, request.height) {
            format!("{w}x{h}")
        } else {
            "1024x1024".to_string()
        };

        let mut body = serde_json::json!({
            "model": model,
            "prompt": request.prompt,
            "n": request.count,
            "size": size,
            "response_format": "b64_json",
        });

        if let Some(ref q) = request.quality {
            body["quality"] = serde_json::json!(q);
        }

        let url = format!("{}/images/generations", self.base_url);
        let client = crate::http_client::proxied_client();
        let response = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
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

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| MediaError::Http(format!("Failed to parse response: {e}")))?;

        let mut images = Vec::new();
        let mut revised_prompt = None;

        if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            for item in data {
                let b64 = item
                    .get("b64_json")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let url = item
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                if b64.len() > MAX_IMAGE_B64_BYTES {
                    warn!("OpenAI generated image base64 exceeds 10MB, skipping");
                    continue;
                }

                images.push(GeneratedImage {
                    data_base64: b64,
                    url,
                });

                if revised_prompt.is_none() {
                    revised_prompt = item
                        .get("revised_prompt")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
        }

        if images.is_empty() {
            return Err(MediaError::Other("No images returned by OpenAI".into()));
        }

        Ok(MediaImageResult {
            images,
            model: model.to_string(),
            provider: "openai".to_string(),
            revised_prompt,
        })
    }

    // ── Text-to-speech ─────────────────────────────────────────────

    async fn synthesize_speech(
        &self,
        request: &MediaTtsRequest,
    ) -> Result<MediaTtsResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        let model = request.model.as_deref().unwrap_or("tts-1");
        let voice = request.voice.as_deref().unwrap_or("alloy");
        let format = request.format.as_deref().unwrap_or("mp3");

        let mut body = serde_json::json!({
            "model": model,
            "input": request.text,
            "voice": voice,
            "response_format": format,
        });

        if let Some(speed) = request.speed {
            body["speed"] = serde_json::json!(speed);
        }

        let url = format!("{}/audio/speech", self.base_url);
        let client = crate::http_client::proxied_client();
        let response = client
            .post(&url)
            .bearer_auth(&api_key)
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

        // Rough duration estimate: ~150 words/min → ~400ms per word
        let word_count = request.text.split_whitespace().count();
        let duration_ms = (word_count as u64 * 400).max(500);

        Ok(MediaTtsResult {
            audio_data,
            format: format.to_string(),
            provider: "openai".to_string(),
            model: model.to_string(),
            duration_ms: Some(duration_ms),
            sample_rate: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_capabilities() {
        let driver = OpenAIMediaDriver::new(None);
        let caps = driver.capabilities();
        assert_eq!(caps.len(), 2);
        assert!(caps.contains(&MediaCapability::ImageGeneration));
        assert!(caps.contains(&MediaCapability::TextToSpeech));
        // Video and music NOT supported
        assert!(!caps.contains(&MediaCapability::VideoGeneration));
        assert!(!caps.contains(&MediaCapability::MusicGeneration));
    }

    #[test]
    fn test_driver_provider_name() {
        let driver = OpenAIMediaDriver::new(None);
        assert_eq!(driver.provider_name(), "openai");
    }

    #[test]
    fn test_driver_custom_base_url() {
        let driver = OpenAIMediaDriver::new(Some("https://custom.api.com/v1/"));
        assert_eq!(driver.base_url, "https://custom.api.com/v1");
    }

    #[tokio::test]
    async fn test_video_not_supported() {
        let driver = OpenAIMediaDriver::new(None);
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
        assert!(matches!(result, Err(super::MediaError::NotSupported(_))));
    }

    #[tokio::test]
    async fn test_music_not_supported() {
        let driver = OpenAIMediaDriver::new(None);
        let req = librefang_types::media::MediaMusicRequest {
            prompt: Some("test".into()),
            lyrics: None,
            provider: None,
            model: None,
            instrumental: false,
            format: None,
        };
        let result = driver.generate_music(&req).await;
        assert!(matches!(result, Err(super::MediaError::NotSupported(_))));
    }
}
