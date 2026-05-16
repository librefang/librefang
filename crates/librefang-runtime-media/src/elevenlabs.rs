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

/// Documented length of an ElevenLabs voice_id. Every voice in the
/// public ElevenLabs OpenAPI spec uses exactly 20 ASCII-alphanumeric
/// characters as the voice_id (e.g. Rachel above); enforcing the shape
/// closes the URL-path-injection vector for the
/// `/v1/text-to-speech/{voice_id}` segment.
///
/// Verified against the spec at
/// <https://elevenlabs.io/docs/api-reference/voices/get-all>
/// (snapshot date 2026-05-16). Re-check on schedule — if ElevenLabs
/// ever lengthens the voice_id format, this constant and the docs in
/// `librefang_runtime::tool_runner::definitions` (text_to_speech
/// `voice` field description) both need to be updated together.
const VOICE_ID_LEN: usize = 20;

/// Cap on how much of a malformed user-supplied voice_id we echo back
/// in `MediaError::InvalidRequest`. A 10kB voice_id should not produce
/// a 10kB error string. Tight cap because a valid voice_id is exactly
/// `VOICE_ID_LEN` (20) chars; 64 bytes is comfortably above that while
/// still bounded.
const VOICE_ID_ERROR_ECHO_MAX_BYTES: usize = 64;

/// Max audio response size (25 MB).
const MAX_AUDIO_RESPONSE_BYTES: usize = 25 * 1024 * 1024;

pub struct ElevenLabsMediaDriver {
    base_url: String,
}

/// Reject voice_id values that would either break the API contract or
/// allow path traversal / query-string injection at the URL boundary.
fn validate_voice_id(voice_id: &str) -> Result<(), MediaError> {
    if voice_id.len() != VOICE_ID_LEN || !voice_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        // Cap the echo so a malicious 10kB voice_id doesn't echo 10kB
        // back through the error chain.
        let echoed = crate::safe_truncate_str(voice_id, VOICE_ID_ERROR_ECHO_MAX_BYTES);
        return Err(MediaError::InvalidRequest(format!(
            "Invalid ElevenLabs voice_id {echoed:?}: expected {VOICE_ID_LEN} ASCII alphanumeric characters. \
             Find valid IDs at https://elevenlabs.io/app/voice-library."
        )));
    }
    Ok(())
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

        // Validate voice_id BEFORE reading the API key so that, when
        // both are wrong, the LLM agent sees the actionable
        // `InvalidRequest` rather than a generic `MissingKey`. Both
        // errors remain pre-network — no HTTP request has been issued.
        let model = request.model.as_deref().unwrap_or(DEFAULT_MODEL);
        let voice_id = request.voice.as_deref().unwrap_or(DEFAULT_VOICE_ID);
        validate_voice_id(voice_id)?;
        let api_key = Self::api_key()?;
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

        let client = librefang_http::proxied_client();
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
            let truncated = crate::safe_truncate_str(&err, 500);
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

    #[test]
    fn default_voice_id_matches_shape_invariant() {
        // Compile-time-ish guarantee: the hardcoded default must pass the
        // same validation we apply to user input, otherwise a request
        // with `voice: None` would 400 before it ever hit the wire.
        assert!(validate_voice_id(DEFAULT_VOICE_ID).is_ok());
        assert_eq!(DEFAULT_VOICE_ID.len(), VOICE_ID_LEN);
    }

    #[test]
    fn validate_voice_id_accepts_known_good_id() {
        // Rachel — present as an example in the ElevenLabs OpenAPI spec.
        assert!(validate_voice_id("21m00Tcm4TlvDq8ikWAM").is_ok());
        // Second documented sample shape.
        assert!(validate_voice_id("VW7YKqPnjY4h39yTbx2L").is_ok());
    }

    #[test]
    fn validate_voice_id_rejects_wrong_length() {
        // 19 characters — the exact failure mode in the closed PR #5039.
        assert!(matches!(
            validate_voice_id("21m00Tcm4TlvDq8ikWA"),
            Err(MediaError::InvalidRequest(_))
        ));
        // 21 characters.
        assert!(matches!(
            validate_voice_id("21m00Tcm4TlvDq8ikWAMx"),
            Err(MediaError::InvalidRequest(_))
        ));
        // Empty.
        assert!(matches!(
            validate_voice_id(""),
            Err(MediaError::InvalidRequest(_))
        ));
    }

    #[test]
    fn validate_voice_id_rejects_url_injection() {
        // Path traversal — would resolve to a different ElevenLabs route.
        assert!(matches!(
            validate_voice_id("../../voices/aaaaa"),
            Err(MediaError::InvalidRequest(_))
        ));
        // Trailing slash — would append a path segment to the URL.
        assert!(matches!(
            validate_voice_id("21m00Tcm4TlvDq8ikWA/"),
            Err(MediaError::InvalidRequest(_))
        ));
        // Query-string smuggling.
        assert!(matches!(
            validate_voice_id("21m00Tcm4TlvDq8?x=y"),
            Err(MediaError::InvalidRequest(_))
        ));
        // Whitespace.
        assert!(matches!(
            validate_voice_id("21m00Tcm4TlvDq8ikWA "),
            Err(MediaError::InvalidRequest(_))
        ));
        // Non-ASCII.
        assert!(matches!(
            validate_voice_id("21m00Tcm4TlvDq8ikWAé"),
            Err(MediaError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    #[serial_test::serial(elevenlabs_api_key)]
    async fn synthesize_speech_rejects_malformed_voice_id_before_network() {
        // Critical: set ELEVENLABS_API_KEY so the validator (NOT the
        // api_key() check) is the gate this test actually drives.
        // Without this, `api_key()?` would short-circuit and we'd be
        // observing `MissingKey` while pretending to test the
        // validator — see #5078 review.
        //
        // The #[serial(elevenlabs_api_key)] guard prevents racing with
        // any other test that mutates this env var concurrently.
        let prior = std::env::var("ELEVENLABS_API_KEY").ok();
        // SAFETY: serialised via #[serial_test::serial]; no concurrent env mutation.
        unsafe {
            std::env::set_var("ELEVENLABS_API_KEY", "test-elevenlabs-key");
        }

        let driver = ElevenLabsMediaDriver::new(None);
        let req = MediaTtsRequest {
            text: "hello".into(),
            provider: None,
            model: None,
            voice: Some("not-a-voice".into()),
            format: None,
            speed: None,
            language: None,
            pitch: None,
        };

        let result = driver.synthesize_speech(&req).await;

        // Restore env BEFORE asserting so a failed assert doesn't
        // poison subsequent tests.
        // SAFETY: same as the set_var above.
        unsafe {
            match prior {
                Some(v) => std::env::set_var("ELEVENLABS_API_KEY", v),
                None => std::env::remove_var("ELEVENLABS_API_KEY"),
            }
        }

        let err = result.expect_err("expected synthesize_speech to reject malformed voice_id");
        match err {
            MediaError::InvalidRequest(msg) => {
                // Sanity-check that the error came from the voice_id
                // validator, not e.g. `request.validate()`.
                assert!(
                    msg.contains("voice_id"),
                    "InvalidRequest should mention voice_id, got: {msg}"
                );
            }
            other => {
                panic!("expected MediaError::InvalidRequest from voice_id validator, got {other:?}")
            }
        }
    }

    #[tokio::test]
    #[serial_test::serial(elevenlabs_api_key)]
    async fn synthesize_speech_truncates_oversized_voice_id_in_error() {
        // A multi-kilobyte voice_id must not echo verbatim through the
        // error chain. Cap is `VOICE_ID_ERROR_ECHO_MAX_BYTES` (64).
        let prior = std::env::var("ELEVENLABS_API_KEY").ok();
        // SAFETY: serialised via #[serial_test::serial]; no concurrent env mutation.
        unsafe {
            std::env::set_var("ELEVENLABS_API_KEY", "test-elevenlabs-key");
        }

        let oversized = "A".repeat(10_000);
        let driver = ElevenLabsMediaDriver::new(None);
        let req = MediaTtsRequest {
            text: "hello".into(),
            provider: None,
            model: None,
            voice: Some(oversized),
            format: None,
            speed: None,
            language: None,
            pitch: None,
        };

        let result = driver.synthesize_speech(&req).await;

        // SAFETY: same as the set_var above.
        unsafe {
            match prior {
                Some(v) => std::env::set_var("ELEVENLABS_API_KEY", v),
                None => std::env::remove_var("ELEVENLABS_API_KEY"),
            }
        }

        let err = result.expect_err("oversized voice_id must be rejected");
        match err {
            MediaError::InvalidRequest(msg) => {
                // Whole error message stays small even though input
                // was 10kB; the echoed voice_id slice is capped to
                // VOICE_ID_ERROR_ECHO_MAX_BYTES, plus quoting +
                // surrounding template text comfortably under 512.
                assert!(
                    msg.len() < 512,
                    "InvalidRequest message should be bounded, got {} bytes",
                    msg.len()
                );
                // Must not echo the entire 10kB input verbatim.
                assert!(
                    !msg.contains(&"A".repeat(VOICE_ID_ERROR_ECHO_MAX_BYTES + 1)),
                    "echo should be truncated at VOICE_ID_ERROR_ECHO_MAX_BYTES"
                );
            }
            other => panic!("expected MediaError::InvalidRequest, got {other:?}"),
        }
    }
}
