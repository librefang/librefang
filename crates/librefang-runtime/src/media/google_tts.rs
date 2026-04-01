//! Google Cloud Text-to-Speech media driver.
//!
//! Supports:
//! - TTS via `POST /v1/text:synthesize` (Standard, WaveNet, Neural2, Studio voices)
//!
//! Image, video, and music generation are not supported.
//!
//! API docs: <https://cloud.google.com/text-to-speech/docs/reference/rest/v1/text/synthesize>

use super::{MediaDriver, MediaError};
use async_trait::async_trait;
use base64::Engine;
use librefang_types::media::{MediaCapability, MediaTtsRequest, MediaTtsResult};

/// Default Google Cloud TTS API base URL.
const DEFAULT_BASE_URL: &str = "https://texttospeech.googleapis.com/v1";

/// Default voice name.
const DEFAULT_VOICE: &str = "en-US-Standard-F";

/// Default language code.
const DEFAULT_LANGUAGE: &str = "en-US";

/// Max audio response size (25 MB).
const MAX_AUDIO_RESPONSE_BYTES: usize = 25 * 1024 * 1024;

pub struct GoogleTtsMediaDriver {
    base_url: String,
}

impl GoogleTtsMediaDriver {
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
        }
    }

    fn api_key() -> Result<String, MediaError> {
        std::env::var("GOOGLE_API_KEY")
            .or_else(|_| std::env::var("GOOGLE_CLOUD_API_KEY"))
            .map_err(|_| {
                MediaError::MissingKey(
                    "Neither GOOGLE_API_KEY nor GOOGLE_CLOUD_API_KEY is set. \
                     Get one at https://console.cloud.google.com"
                        .into(),
                )
            })
    }
}

#[async_trait]
impl MediaDriver for GoogleTtsMediaDriver {
    fn capabilities(&self) -> Vec<MediaCapability> {
        vec![MediaCapability::TextToSpeech]
    }

    fn is_configured(&self) -> bool {
        Self::api_key().is_ok()
    }

    fn provider_name(&self) -> &str {
        "google_tts"
    }

    // ── Text-to-speech ────────────────────────────────────────────────

    async fn synthesize_speech(
        &self,
        request: &MediaTtsRequest,
    ) -> Result<MediaTtsResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        let voice_name = request.voice.as_deref().unwrap_or(DEFAULT_VOICE);
        let language_code = request.language.as_deref().unwrap_or(DEFAULT_LANGUAGE);
        let speaking_rate = request.speed.unwrap_or(1.0);
        let audio_encoding = map_audio_encoding(request.format.as_deref());

        // Detect SSML vs plain text
        let input = build_input(&request.text);

        let body = serde_json::json!({
            "input": input,
            "voice": {
                "languageCode": language_code,
                "name": voice_name,
            },
            "audioConfig": {
                "audioEncoding": audio_encoding,
                "speakingRate": speaking_rate,
                "pitch": request.pitch.unwrap_or(0.0),
            },
        });

        let url = format!("{}/text:synthesize?key={}", self.base_url, api_key);

        let client = crate::http_client::proxied_client();
        let response = client
            .post(&url)
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

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| MediaError::Http(format!("Failed to parse JSON response: {e}")))?;

        let audio_b64 = json["audioContent"]
            .as_str()
            .ok_or_else(|| MediaError::Other("Missing audioContent in response".into()))?;

        let audio_data = base64::engine::general_purpose::STANDARD
            .decode(audio_b64)
            .map_err(|e| MediaError::Other(format!("Failed to decode base64 audio: {e}")))?;

        if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
            return Err(MediaError::Other(format!(
                "Audio data exceeds {}MB limit",
                MAX_AUDIO_RESPONSE_BYTES / 1024 / 1024
            )));
        }

        // Rough duration estimate: ~150 words/min, adjusted for speaking rate
        let word_count = request.text.split_whitespace().count();
        let rate = speaking_rate.max(0.25);
        let duration_ms = ((word_count as f64 * 400.0) / rate as f64).max(500.0) as u64;

        let model = request
            .model
            .as_deref()
            .unwrap_or("google-tts-standard")
            .to_string();

        let format = request
            .format
            .as_deref()
            .unwrap_or("mp3")
            .split('_')
            .next()
            .unwrap_or("mp3")
            .to_lowercase();

        Ok(MediaTtsResult {
            audio_data,
            format,
            provider: "google_tts".to_string(),
            model,
            duration_ms: Some(duration_ms),
            sample_rate: None,
        })
    }
}

/// Detect SSML in the input text and build the appropriate JSON input object.
///
/// - If text contains `<speak>`, it is treated as a complete SSML document.
/// - If text contains SSML tags (e.g. `<break`) but no `<speak>` wrapper,
///   it is wrapped in `<speak>...</speak>`.
/// - Otherwise plain text is used.
fn build_input(text: &str) -> serde_json::Value {
    if text.contains("<speak>") {
        serde_json::json!({ "ssml": text })
    } else if text.contains("<break") || is_ssml(text) {
        serde_json::json!({ "ssml": format!("<speak>{text}</speak>") })
    } else {
        serde_json::json!({ "text": text })
    }
}

/// Returns true if the text looks like it contains SSML markup tags.
fn is_ssml(text: &str) -> bool {
    // Common SSML tags aside from <speak> and <break>.
    // For <p> and <s>, require closing tags to avoid false positives on plain HTML.
    text.contains("<prosody")
        || text.contains("<emphasis")
        || text.contains("<say-as")
        || text.contains("<phoneme")
        || text.contains("<audio")
        || (text.contains("<p>") && text.contains("</p>"))
        || (text.contains("<s>") && text.contains("</s>"))
        || text.contains("<sub")
        || text.contains("<mark")
        || text.contains("<par")
        || text.contains("<seq")
        || text.contains("<media")
}

/// Map a requested audio format to a Google Cloud TTS audioEncoding value.
fn map_audio_encoding(format: Option<&str>) -> &'static str {
    match format.unwrap_or("mp3").split('_').next().unwrap_or("mp3") {
        "opus" | "ogg" => "OGG_OPUS",
        "wav" | "pcm" | "linear16" => "LINEAR16",
        _ => "MP3",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_capabilities() {
        let driver = GoogleTtsMediaDriver::new(None);
        let caps = driver.capabilities();
        assert_eq!(caps.len(), 1);
        assert!(caps.contains(&MediaCapability::TextToSpeech));
        assert!(!caps.contains(&MediaCapability::ImageGeneration));
    }

    #[test]
    fn test_driver_provider_name() {
        let driver = GoogleTtsMediaDriver::new(None);
        assert_eq!(driver.provider_name(), "google_tts");
    }

    #[test]
    fn test_driver_custom_base_url() {
        let driver = GoogleTtsMediaDriver::new(Some("https://custom.api/v1/"));
        assert_eq!(driver.base_url, "https://custom.api/v1");
    }

    #[test]
    fn test_ssml_detection() {
        // Plain text → text input
        let input = build_input("Hello world");
        assert_eq!(input["text"], "Hello world");
        assert!(input["ssml"].is_null());

        // Already has <speak> wrapper
        let ssml = "<speak>Hello <break time=\"500ms\"/> world</speak>";
        let input = build_input(ssml);
        assert_eq!(input["ssml"], ssml);
        assert!(input["text"].is_null());

        // Has SSML tag but no <speak> wrapper → auto-wrapped
        let partial = "Hello <break time=\"500ms\"/> world";
        let input = build_input(partial);
        let ssml_val = input["ssml"].as_str().unwrap();
        assert!(ssml_val.starts_with("<speak>"));
        assert!(ssml_val.ends_with("</speak>"));
        assert!(ssml_val.contains(partial));
        assert!(input["text"].is_null());
    }

    #[test]
    fn test_audio_encoding_mapping() {
        assert_eq!(map_audio_encoding(None), "MP3");
        assert_eq!(map_audio_encoding(Some("mp3")), "MP3");
        assert_eq!(map_audio_encoding(Some("mp3_44100_128")), "MP3");
        assert_eq!(map_audio_encoding(Some("opus")), "OGG_OPUS");
        assert_eq!(map_audio_encoding(Some("ogg")), "OGG_OPUS");
        assert_eq!(map_audio_encoding(Some("wav")), "LINEAR16");
        assert_eq!(map_audio_encoding(Some("pcm")), "LINEAR16");
        assert_eq!(map_audio_encoding(Some("linear16")), "LINEAR16");
        assert_eq!(map_audio_encoding(Some("flac")), "MP3"); // unknown → MP3
    }

    #[tokio::test]
    async fn test_image_not_supported() {
        let driver = GoogleTtsMediaDriver::new(None);
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
        let driver = GoogleTtsMediaDriver::new(None);
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
