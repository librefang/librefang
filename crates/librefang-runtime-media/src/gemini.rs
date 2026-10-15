//! Google Gemini media generation driver.
//!
//! Supports:
//! - Image generation via Imagen 3 (`POST /v1beta/models/{model}:predict`)
//!
//! TTS, video, and music generation are not supported.
//!
//! Auth: `GEMINI_API_KEY` or `GOOGLE_API_KEY` env var, sent in the `x-goog-api-key` header.
//! The key is never placed in the URL, because reqwest error messages include the full request URL and those messages flow into tool results and logs.

use async_trait::async_trait;
use librefang_types::media::{
    GeneratedImage, MediaCapability, MediaImageRequest, MediaImageResult,
};
use tracing::warn;

use super::{MediaDriver, MediaError};

/// Default Gemini API base URL.
const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";

/// Default Imagen model.
const DEFAULT_IMAGE_MODEL: &str = "imagen-3.0-generate-002";

/// Max response body size (10 MB).
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

pub struct GeminiMediaDriver {
    base_url: String,
}

impl GeminiMediaDriver {
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
        }
    }

    fn api_key() -> Result<String, MediaError> {
        std::env::var("GEMINI_API_KEY")
            .or_else(|_| std::env::var("GOOGLE_API_KEY"))
            .map_err(|_| {
                MediaError::MissingKey(
                    "GEMINI_API_KEY or GOOGLE_API_KEY not set. \
                     Get one at https://aistudio.google.com/apikey"
                        .into(),
                )
            })
    }
}

#[async_trait]
impl MediaDriver for GeminiMediaDriver {
    fn capabilities(&self) -> Vec<MediaCapability> {
        vec![MediaCapability::ImageGeneration]
    }

    fn is_configured(&self) -> bool {
        Self::api_key().is_ok()
    }

    fn provider_name(&self) -> &str {
        "gemini"
    }

    // ── Image generation via Imagen 3 ─────────────────────────────────

    async fn generate_image(
        &self,
        request: &MediaImageRequest,
    ) -> Result<MediaImageResult, MediaError> {
        request.validate().map_err(MediaError::InvalidRequest)?;

        let api_key = Self::api_key()?;
        self.generate_image_with_key(request, &api_key).await
    }
}

impl GeminiMediaDriver {
    /// Run the Imagen request with an explicit key, so tests can drive the request path without mutating process env.
    async fn generate_image_with_key(
        &self,
        request: &MediaImageRequest,
        api_key: &str,
    ) -> Result<MediaImageResult, MediaError> {
        let model = request.model.as_deref().unwrap_or(DEFAULT_IMAGE_MODEL);

        let mut params = serde_json::json!({
            "prompt": request.prompt,
            "sampleCount": request.count.min(4),
        });

        // Imagen supports aspect ratios: "1:1", "3:4", "4:3", "9:16", "16:9"
        if let Some(ref ar) = request.aspect_ratio {
            params["aspectRatio"] = serde_json::json!(ar);
        }

        let body = serde_json::json!({
            "instances": [params],
        });

        let url = format!("{}/v1beta/models/{}:predict", self.base_url, model);

        let client = librefang_http::proxied_client();
        let response = client
            .post(&url)
            .header("x-goog-api-key", api_key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| MediaError::Http(e.without_url().to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let err = response.text().await.unwrap_or_default();
            let truncated = crate::safe_truncate_str(&err, 500);
            return Err(MediaError::Api {
                status,
                message: truncated.to_string(),
            });
        }

        let json: serde_json::Value = response.json().await.map_err(|e| {
            MediaError::Http(format!("Failed to parse response: {}", e.without_url()))
        })?;

        let mut images = Vec::new();

        if let Some(predictions) = json.get("predictions").and_then(|p| p.as_array()) {
            for pred in predictions {
                let b64 = pred
                    .get("bytesBase64Encoded")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                if b64.is_empty() {
                    continue;
                }
                if b64.len() > MAX_RESPONSE_BYTES {
                    warn!("Gemini generated image base64 exceeds 10MB, skipping");
                    continue;
                }

                images.push(GeneratedImage {
                    data_base64: b64,
                    url: None,
                });
            }
        }

        if images.is_empty() {
            // Check for content filter
            if let Some(filtered) = json
                .pointer("/predictions/0/raiFilteredReason")
                .and_then(|v| v.as_str())
            {
                return Err(MediaError::ContentFiltered(filtered.to_string()));
            }
            return Err(MediaError::Other(
                "No images returned by Gemini Imagen".into(),
            ));
        }

        Ok(MediaImageResult {
            images,
            model: model.to_string(),
            provider: "gemini".to_string(),
            revised_prompt: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL_KEY: &str = "AIzaTESTSENTINEL123";

    /// Bind then drop a listener so the port is closed and a request to it fails at connect time.
    fn closed_port_base_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    /// Accept one connection, capture the raw request head, and answer 200 with a non-JSON body.
    async fn one_shot_non_json_server() -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot json")
                .await
                .unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });
        (base, handle)
    }

    fn image_req() -> MediaImageRequest {
        MediaImageRequest {
            prompt: "a cat".into(),
            provider: None,
            model: None,
            width: None,
            height: None,
            aspect_ratio: None,
            quality: None,
            count: 1,
            seed: None,
        }
    }

    #[tokio::test]
    async fn transport_error_does_not_leak_api_key() {
        let driver = GeminiMediaDriver::new(Some(&closed_port_base_url()));
        let err = driver
            .generate_image_with_key(&image_req(), SENTINEL_KEY)
            .await
            .expect_err("closed port must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains(SENTINEL_KEY),
            "API key leaked into error: {msg}"
        );
    }

    #[tokio::test]
    async fn key_is_sent_as_header_and_parse_error_does_not_leak_it() {
        let (base, server) = one_shot_non_json_server().await;
        let driver = GeminiMediaDriver::new(Some(&base));
        let err = driver
            .generate_image_with_key(&image_req(), SENTINEL_KEY)
            .await
            .expect_err("non-JSON body must fail to parse");
        let head = server.await.unwrap();
        let request_line = head.lines().next().unwrap_or_default();
        assert!(
            !request_line.contains(SENTINEL_KEY),
            "API key in URL: {request_line}"
        );
        assert!(
            head.to_ascii_lowercase().contains(&format!(
                "x-goog-api-key: {}",
                SENTINEL_KEY.to_ascii_lowercase()
            )),
            "API key header missing: {head}"
        );
        let msg = err.to_string();
        assert!(
            !msg.contains(SENTINEL_KEY),
            "API key leaked into error: {msg}"
        );
    }

    #[test]
    fn test_driver_capabilities() {
        let driver = GeminiMediaDriver::new(None);
        let caps = driver.capabilities();
        assert_eq!(caps.len(), 1);
        assert!(caps.contains(&MediaCapability::ImageGeneration));
        assert!(!caps.contains(&MediaCapability::TextToSpeech));
        assert!(!caps.contains(&MediaCapability::VideoGeneration));
        assert!(!caps.contains(&MediaCapability::MusicGeneration));
    }

    #[test]
    fn test_driver_provider_name() {
        let driver = GeminiMediaDriver::new(None);
        assert_eq!(driver.provider_name(), "gemini");
    }

    #[test]
    fn test_driver_custom_base_url() {
        let driver = GeminiMediaDriver::new(Some("https://custom.gemini.api/v1beta/"));
        assert_eq!(driver.base_url, "https://custom.gemini.api/v1beta");
    }

    #[tokio::test]
    async fn test_tts_not_supported() {
        let driver = GeminiMediaDriver::new(None);
        let req = librefang_types::media::MediaTtsRequest {
            text: "test".into(),
            provider: None,
            model: None,
            voice: None,
            speed: None,
            format: None,
            language: None,
            pitch: None,
        };
        let result = driver.synthesize_speech(&req).await;
        assert!(matches!(result, Err(MediaError::NotSupported(_))));
    }

    #[tokio::test]
    async fn test_video_not_supported() {
        let driver = GeminiMediaDriver::new(None);
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

    #[tokio::test]
    async fn test_music_not_supported() {
        let driver = GeminiMediaDriver::new(None);
        let req = librefang_types::media::MediaMusicRequest {
            prompt: Some("test".into()),
            lyrics: None,
            provider: None,
            model: None,
            instrumental: false,
            format: None,
        };
        let result = driver.generate_music(&req).await;
        assert!(matches!(result, Err(MediaError::NotSupported(_))));
    }
}
