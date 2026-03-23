//! MiniMax media generation driver.
//!
//! Supports all four modalities:
//! - Image generation via `POST /v1/image_generation` (model: image-01)
//! - TTS via `POST /v1/t2a_v2` (models: speech-2.8-hd, speech-2.8-turbo, etc.)
//! - Video generation via `POST /v1/video_generation` + polling (Hailuo models)
//! - Music generation via `POST /v1/music_generation` (models: music-2.5, music-2.5+)
//!
//! API docs: <https://platform.minimax.io/docs/api-reference/api-overview>

use async_trait::async_trait;
use librefang_types::media::{
    GeneratedImage, MediaCapability, MediaImageRequest, MediaImageResult, MediaMusicRequest,
    MediaMusicResult, MediaTaskStatus, MediaTtsRequest, MediaTtsResult, MediaVideoRequest,
    MediaVideoResult, MediaVideoSubmitResult,
};
use tracing::warn;

use super::{MediaDriver, MediaError};

/// Default base URL for MiniMax international API.
const DEFAULT_BASE_URL: &str = "https://api.minimax.io/v1";

/// Maximum response body size for sync requests (50 MB).
const MAX_RESPONSE_BYTES: usize = 50 * 1024 * 1024;

/// Default timeout for sync media requests (120s — music can take 60s+).
const DEFAULT_TIMEOUT_SECS: u64 = 180;

/// Default timeout for video submit/poll requests.
const POLL_TIMEOUT_SECS: u64 = 30;

pub struct MiniMaxMediaDriver {
    base_url: String,
}

impl MiniMaxMediaDriver {
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
        }
    }

    fn api_key() -> Result<String, MediaError> {
        std::env::var("MINIMAX_API_KEY")
            .or_else(|_| std::env::var("MINIMAX_CN_API_KEY"))
            .map_err(|_| {
                MediaError::MissingKey(
                    "MINIMAX_API_KEY not set. Get one at https://platform.minimax.io".into(),
                )
            })
    }

    /// Parse the `base_resp` from MiniMax API responses.
    fn check_base_resp(json: &serde_json::Value) -> Result<(), MediaError> {
        if let Some(base_resp) = json.get("base_resp") {
            let code = base_resp
                .get("status_code")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if code != 0 {
                let msg = base_resp
                    .get("status_msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                return match code {
                    1002 => Err(MediaError::RateLimit(msg.to_string())),
                    1004 => Err(MediaError::MissingKey(msg.to_string())),
                    1026 | 1027 => Err(MediaError::ContentFiltered(msg.to_string())),
                    2013 => Err(MediaError::InvalidRequest(msg.to_string())),
                    _ => Err(MediaError::Api {
                        status: code as u16,
                        message: msg.to_string(),
                    }),
                };
            }
        }
        Ok(())
    }
}

#[async_trait]
impl MediaDriver for MiniMaxMediaDriver {
    fn capabilities(&self) -> Vec<MediaCapability> {
        vec![
            MediaCapability::ImageGeneration,
            MediaCapability::TextToSpeech,
            MediaCapability::VideoGeneration,
            MediaCapability::MusicGeneration,
        ]
    }

    fn is_configured(&self) -> bool {
        Self::api_key().is_ok()
    }

    fn provider_name(&self) -> &str {
        "minimax"
    }

    // ── Image generation ───────────────────────────────────────────

    async fn generate_image(
        &self,
        request: &MediaImageRequest,
    ) -> Result<MediaImageResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        let model = request.model.as_deref().unwrap_or("image-01");

        let mut body = serde_json::json!({
            "model": model,
            "prompt": request.prompt,
            "n": request.count,
            "response_format": "url",
        });

        if let Some(ref ar) = request.aspect_ratio {
            body["aspect_ratio"] = serde_json::json!(ar);
        }
        if let Some(w) = request.width {
            body["width"] = serde_json::json!(w);
        }
        if let Some(h) = request.height {
            body["height"] = serde_json::json!(h);
        }
        if let Some(seed) = request.seed {
            body["seed"] = serde_json::json!(seed);
        }

        let url = format!("{}/image_generation", self.base_url);
        let client = crate::http_client::proxied_client();
        let response = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
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

        Self::check_base_resp(&json)?;

        let mut images = Vec::new();
        if let Some(data) = json.get("data") {
            // URL format
            if let Some(urls) = data.get("image_urls").and_then(|v| v.as_array()) {
                for url in urls {
                    if let Some(u) = url.as_str() {
                        images.push(GeneratedImage {
                            data_base64: String::new(),
                            url: Some(u.to_string()),
                        });
                    }
                }
            }
            // Base64 format
            if let Some(b64s) = data.get("image_base64").and_then(|v| v.as_array()) {
                for b64 in b64s {
                    if let Some(d) = b64.as_str() {
                        if d.len() > 10 * 1024 * 1024 {
                            warn!("MiniMax image base64 exceeds 10MB, skipping");
                            continue;
                        }
                        images.push(GeneratedImage {
                            data_base64: d.to_string(),
                            url: None,
                        });
                    }
                }
            }
        }

        if images.is_empty() {
            return Err(MediaError::Other("No images returned by MiniMax".into()));
        }

        Ok(MediaImageResult {
            images,
            model: model.to_string(),
            provider: "minimax".to_string(),
            revised_prompt: None,
        })
    }

    // ── Text-to-speech ─────────────────────────────────────────────

    async fn synthesize_speech(
        &self,
        request: &MediaTtsRequest,
    ) -> Result<MediaTtsResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        let model = request.model.as_deref().unwrap_or("speech-2.8-hd");
        let voice_id = request.voice.as_deref().unwrap_or("English_Graceful_Lady");

        let mut body = serde_json::json!({
            "model": model,
            "text": request.text,
            "voice_setting": {
                "voice_id": voice_id,
            },
            "output_format": "url",
        });

        if let Some(speed) = request.speed {
            body["voice_setting"]["speed"] = serde_json::json!(speed);
        }
        if let Some(ref fmt) = request.format {
            body["audio_setting"] = serde_json::json!({ "format": fmt });
        }
        if let Some(ref lang) = request.language {
            body["language_boost"] = serde_json::json!(lang);
        }

        let url = format!("{}/t2a_v2", self.base_url);
        let client = crate::http_client::proxied_client();
        let response = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
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

        Self::check_base_resp(&json)?;

        // MiniMax returns audio as hex-encoded bytes or a URL
        let audio_data = if let Some(hex_str) = json.pointer("/data/audio").and_then(|v| v.as_str())
        {
            hex::decode(hex_str)
                .map_err(|e| MediaError::Other(format!("Failed to decode hex audio: {e}")))?
        } else {
            return Err(MediaError::Other(
                "No audio data in MiniMax response".into(),
            ));
        };

        if audio_data.len() > MAX_RESPONSE_BYTES {
            return Err(MediaError::Other(format!(
                "Audio data too large: {} bytes",
                audio_data.len()
            )));
        }

        let duration_ms = json
            .pointer("/extra_info/audio_length")
            .and_then(|v| v.as_u64());
        let sample_rate = json
            .pointer("/extra_info/audio_sample_rate")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let format = json
            .pointer("/extra_info/audio_format")
            .and_then(|v| v.as_str())
            .unwrap_or("mp3")
            .to_string();

        Ok(MediaTtsResult {
            audio_data,
            format,
            provider: "minimax".to_string(),
            model: model.to_string(),
            duration_ms,
            sample_rate,
        })
    }

    // ── Video generation (async) ───────────────────────────────────

    async fn submit_video(
        &self,
        request: &MediaVideoRequest,
    ) -> Result<MediaVideoSubmitResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        let model = request.model.as_deref().unwrap_or("MiniMax-Hailuo-2.3");

        let mut body = serde_json::json!({
            "model": model,
            "prompt": request.prompt,
        });

        if let Some(d) = request.duration_secs {
            body["duration"] = serde_json::json!(d);
        }
        if let Some(ref res) = request.resolution {
            body["resolution"] = serde_json::json!(res);
        }
        if let Some(opt) = request.optimize_prompt {
            body["prompt_optimizer"] = serde_json::json!(opt);
        }

        let url = format!("{}/video_generation", self.base_url);
        let client = crate::http_client::proxied_client();
        let response = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(POLL_TIMEOUT_SECS))
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

        Self::check_base_resp(&json)?;

        let task_id = json
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MediaError::Other("No task_id in video generation response".into()))?
            .to_string();

        Ok(MediaVideoSubmitResult {
            task_id,
            provider: "minimax".to_string(),
        })
    }

    async fn poll_video(&self, task_id: &str) -> Result<MediaTaskStatus, MediaError> {
        let api_key = Self::api_key()?;

        let url = format!(
            "{}/query/video_generation?task_id={}",
            self.base_url, task_id
        );
        let client = crate::http_client::proxied_client();
        let response = client
            .get(&url)
            .bearer_auth(&api_key)
            .timeout(std::time::Duration::from_secs(POLL_TIMEOUT_SECS))
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

        Self::check_base_resp(&json)?;

        let status_str = json
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        match status_str {
            "Preparing" => Ok(MediaTaskStatus::Pending),
            "Queueing" => Ok(MediaTaskStatus::Queued),
            "Processing" => Ok(MediaTaskStatus::Processing),
            "Success" => Ok(MediaTaskStatus::Completed),
            "Fail" => Ok(MediaTaskStatus::Failed {
                error: "Video generation failed".into(),
            }),
            _ => Ok(MediaTaskStatus::Processing), // treat unknown as in-progress
        }
    }

    async fn get_video_result(&self, task_id: &str) -> Result<MediaVideoResult, MediaError> {
        let api_key = Self::api_key()?;

        // First poll to get the file_id
        let url = format!(
            "{}/query/video_generation?task_id={}",
            self.base_url, task_id
        );
        let client = crate::http_client::proxied_client();
        let response = client
            .get(&url)
            .bearer_auth(&api_key)
            .timeout(std::time::Duration::from_secs(POLL_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| MediaError::Http(e.to_string()))?;

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| MediaError::Http(format!("Failed to parse response: {e}")))?;

        Self::check_base_resp(&json)?;

        let status = json
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if status != "Success" {
            return Err(MediaError::Other(format!(
                "Video task not completed, status: {status}"
            )));
        }

        let file_id = json
            .get("file_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MediaError::Other("No file_id in completed video task".into()))?;

        // Fetch the file URL via the file retrieval API
        let file_url = format!("{}/files/retrieve?file_id={}", self.base_url, file_id);
        let file_resp = client
            .get(&file_url)
            .bearer_auth(&api_key)
            .timeout(std::time::Duration::from_secs(POLL_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| MediaError::Http(e.to_string()))?;

        let file_json: serde_json::Value = file_resp
            .json()
            .await
            .map_err(|e| MediaError::Http(format!("Failed to parse file response: {e}")))?;

        let download_url = file_json
            .pointer("/file/download_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MediaError::Other("No download_url in file response".into()))?
            .to_string();

        let width = json
            .get("video_width")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let height = json
            .get("video_height")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        Ok(MediaVideoResult {
            file_url: download_url,
            width,
            height,
            duration_secs: None,
            provider: "minimax".to_string(),
            model: "hailuo".to_string(),
        })
    }

    // ── Music generation (sync) ────────────────────────────────────

    async fn generate_music(
        &self,
        request: &MediaMusicRequest,
    ) -> Result<MediaMusicResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        let model = request.model.as_deref().unwrap_or("music-2.5");

        let mut body = serde_json::json!({
            "model": model,
            "output_format": "hex",
        });

        if let Some(ref p) = request.prompt {
            body["prompt"] = serde_json::json!(p);
        }
        if let Some(ref l) = request.lyrics {
            body["lyrics"] = serde_json::json!(l);
        }
        if request.instrumental {
            body["is_instrumental"] = serde_json::json!(true);
        }
        if let Some(ref fmt) = request.format {
            body["audio_setting"] = serde_json::json!({ "format": fmt });
        }

        let url = format!("{}/music_generation", self.base_url);
        let client = crate::http_client::proxied_client();
        let response = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
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

        Self::check_base_resp(&json)?;

        let audio_data = if let Some(hex_str) = json.pointer("/data/audio").and_then(|v| v.as_str())
        {
            hex::decode(hex_str)
                .map_err(|e| MediaError::Other(format!("Failed to decode hex audio: {e}")))?
        } else {
            return Err(MediaError::Other(
                "No audio data in MiniMax music response".into(),
            ));
        };

        if audio_data.len() > MAX_RESPONSE_BYTES {
            return Err(MediaError::Other(format!(
                "Music data too large: {} bytes",
                audio_data.len()
            )));
        }

        let duration_ms = json
            .pointer("/extra_info/music_duration")
            .and_then(|v| v.as_u64());
        let sample_rate = json
            .pointer("/extra_info/music_sample_rate")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let format = request.format.as_deref().unwrap_or("mp3").to_string();

        Ok(MediaMusicResult {
            audio_data,
            format,
            duration_ms,
            provider: "minimax".to_string(),
            model: model.to_string(),
            sample_rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_capabilities() {
        let driver = MiniMaxMediaDriver::new(None);
        let caps = driver.capabilities();
        assert_eq!(caps.len(), 4);
        assert!(caps.contains(&MediaCapability::ImageGeneration));
        assert!(caps.contains(&MediaCapability::TextToSpeech));
        assert!(caps.contains(&MediaCapability::VideoGeneration));
        assert!(caps.contains(&MediaCapability::MusicGeneration));
    }

    #[test]
    fn test_driver_provider_name() {
        let driver = MiniMaxMediaDriver::new(None);
        assert_eq!(driver.provider_name(), "minimax");
    }

    #[test]
    fn test_driver_custom_base_url() {
        let driver = MiniMaxMediaDriver::new(Some("https://api.minimaxi.com/v1/"));
        assert_eq!(driver.base_url, "https://api.minimaxi.com/v1");
    }

    #[test]
    fn test_check_base_resp_success() {
        let json = serde_json::json!({
            "base_resp": { "status_code": 0, "status_msg": "success" }
        });
        assert!(MiniMaxMediaDriver::check_base_resp(&json).is_ok());
    }

    #[test]
    fn test_check_base_resp_rate_limit() {
        let json = serde_json::json!({
            "base_resp": { "status_code": 1002, "status_msg": "rate limited" }
        });
        let err = MiniMaxMediaDriver::check_base_resp(&json).unwrap_err();
        assert!(matches!(err, MediaError::RateLimit(_)));
    }

    #[test]
    fn test_check_base_resp_content_filtered() {
        let json = serde_json::json!({
            "base_resp": { "status_code": 1026, "status_msg": "sensitive content" }
        });
        let err = MiniMaxMediaDriver::check_base_resp(&json).unwrap_err();
        assert!(matches!(err, MediaError::ContentFiltered(_)));
    }
}
