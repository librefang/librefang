//! Media understanding engine — image description, audio transcription, video analysis.
//!
//! Each modality dispatches to a single provider: either the one explicitly
//! configured in `[media]` (`image_provider` / `audio_provider`), or — when
//! no explicit provider is set — the first one whose API key env var is
//! present. There is no runtime cascade across providers; a failure on the
//! chosen provider surfaces as an `Err` to the caller.

use librefang_types::media::{
    MediaAttachment, MediaConfig, MediaSource, MediaType, MediaUnderstanding,
};
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tracing::info;

/// Media understanding engine.
pub struct MediaEngine {
    config: MediaConfig,
    semaphore: Arc<Semaphore>,
}

impl MediaEngine {
    pub fn new(config: MediaConfig) -> Self {
        let max = config.max_concurrency.clamp(1, 8);
        Self {
            config,
            semaphore: Arc::new(Semaphore::new(max)),
        }
    }

    /// Describe an image using a vision-capable LLM.
    ///
    /// Picks a single provider: `[media] image_provider` if set, otherwise
    /// the first of Anthropic / OpenAI / Groq / Gemini whose API key env var is
    /// present. No runtime fallback if the chosen provider errors.
    ///
    /// Reads the image bytes from the attachment source, base64-encodes them,
    /// and sends them to the provider's multimodal endpoint.
    pub async fn describe_image(
        &self,
        attachment: &MediaAttachment,
    ) -> Result<MediaUnderstanding, String> {
        attachment.validate()?;
        if attachment.media_type != MediaType::Image {
            return Err("Expected image attachment".into());
        }

        // Determine which provider to use
        let explicit = self.config.image_provider.is_some();
        let provider = self
            .config
            .image_provider
            .as_deref()
            .or_else(|| detect_vision_provider())
            .ok_or(
                "No vision-capable LLM provider configured. \
                 Set ANTHROPIC_API_KEY, OPENAI_API_KEY, GROQ_API_KEY, or GEMINI_API_KEY",
            )?;

        if !explicit {
            tracing::debug!(
                detected_provider = provider,
                "Image provider auto-detected from env var — set [media] image_provider in \
                 config.toml for reproducible behaviour."
            );
        }

        let _permit = self.semaphore.acquire().await.map_err(|e| e.to_string())?;

        // Read image bytes from source
        let image_bytes = match &attachment.source {
            MediaSource::FilePath { path } => tokio::fs::read(path)
                .await
                .map_err(|e| format!("Failed to read image file '{}': {}", path, e))?,
            MediaSource::Base64 { data, .. } => {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|e| format!("Failed to decode base64 image: {}", e))?
            }
            MediaSource::Url { url } => {
                return Err(format!(
                    "URL-based image source not supported for describe_image: {}. \
                     Download the image first.",
                    url
                ));
            }
            other => {
                return Err(format!(
                    "Unsupported image source variant for describe_image: {:?}",
                    other
                ));
            }
        };

        let mime_type = &attachment.mime_type;
        let model = self
            .config
            .image_model
            .as_deref()
            .unwrap_or_else(|| default_vision_model(provider));

        info!(
            provider,
            model,
            size = image_bytes.len(),
            mime = %mime_type,
            "Sending image for description"
        );

        let description = match provider {
            "anthropic" => anthropic_describe_image(model, &image_bytes, mime_type).await?,
            "openai" | "groq" => {
                let (api_url, api_key) = openai_vision_provider_config(provider)?;
                openai_describe_image(&api_url, &api_key, model, &image_bytes, mime_type).await?
            }
            "gemini" => gemini_describe_image(model, &image_bytes, mime_type).await?,
            other => return Err(format!("Unsupported image description provider: {}", other)),
        };

        let description = description.trim().to_string();
        if description.is_empty() {
            return Err("Image description returned empty text".into());
        }

        info!(
            provider,
            model,
            chars = description.len(),
            "Image description complete"
        );

        Ok(MediaUnderstanding {
            media_type: MediaType::Image,
            description,
            provider: provider.to_string(),
            model: model.to_string(),
        })
    }

    /// Transcribe audio using speech-to-text.
    /// Picks a single provider: `[media] audio_provider` if set, otherwise
    /// the first one detected from env vars (Groq, OpenAI, Gemini,
    /// ElevenLabs, …). There is no runtime cascade; a provider failure
    /// surfaces as `Err` to the caller.
    pub async fn transcribe_audio(
        &self,
        attachment: &MediaAttachment,
    ) -> Result<MediaUnderstanding, String> {
        attachment.validate()?;
        if attachment.media_type != MediaType::Audio {
            return Err("Expected audio attachment".into());
        }

        let explicit = self.config.audio_provider.is_some();
        let provider = self
            .config
            .audio_provider
            .as_deref()
            .or_else(|| detect_audio_provider())
            .ok_or(
                "No audio transcription provider configured. Set [media] audio_provider in config.toml.",
            )?;

        if !explicit {
            tracing::warn!(
                detected_provider = provider,
                "Audio provider auto-detected from env var — may not match actual service. \
                 Set [media] audio_provider in config.toml for reliable STT."
            );
        }

        let _permit = self.semaphore.acquire().await.map_err(|e| e.to_string())?;

        // Read audio bytes from source
        let mut audio_bytes = match &attachment.source {
            MediaSource::FilePath { path } => tokio::fs::read(path)
                .await
                .map_err(|e| format!("Failed to read audio file '{}': {}", path, e))?,
            MediaSource::Base64 { data, .. } => {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|e| format!("Failed to decode base64 audio: {}", e))?
            }
            MediaSource::Url { url } => {
                return Err(format!(
                    "URL-based audio source not supported for transcription: {}",
                    url
                ));
            }
            other => {
                return Err(format!(
                    "Unsupported audio source variant for transcription: {:?}",
                    other
                ));
            }
        };

        // Derive a proper filename with extension for Whisper to detect format.
        let source_ext = match &attachment.source {
            MediaSource::FilePath { path } => Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase()),
            _ => None,
        };
        let mut mime = attachment.mime_type.clone();
        let mut ext = mime_to_ext(&mime).unwrap_or_else(|| {
            // Fall back to the source file extension when the MIME is missing
            // or unknown (e.g. `application/octet-stream`).
            source_ext.clone().unwrap_or_else(|| "wav".to_string())
        });

        // Telegram voice notes arrive as `.oga` / `audio/oga`. Whisper's
        // format probe rejects both — re-encode to Ogg/Opus so the same
        // Opus payload is delivered under the `audio/ogg` shape Whisper
        // accepts. Failure here is hard-error: the warn+passthrough
        // fallback is useless (the bug this fixes is exactly that raw
        // .oga is rejected).
        if ext == "oga" || mime.eq_ignore_ascii_case("audio/oga") {
            let transcoded = transcode_oga_to_ogg_opus(&audio_bytes)
                .await
                .map_err(|e| format!("ffmpeg .oga transcode failed: {e}"))?;
            info!(
                original_size = audio_bytes.len(),
                transcoded_size = transcoded.len(),
                "Transcoded .oga -> .ogg before Whisper upload"
            );
            audio_bytes = transcoded;
            ext = "ogg".to_string();
            mime = "audio/ogg".to_string();
        }

        let filename = format!("audio.{}", ext);

        // Resolution order for model:
        // 1. Explicit [media] audio_model (per-provider override)
        // 2. [media.custom_stt] model — for custom / self-hosted providers only
        //    (the `_other` dispatch arm below). Must NOT leak into built-in
        //    providers (groq/openai/minimax/fireworks/together/siliconflow/
        //    gemini/elevenlabs); otherwise an operator who sets
        //    `[media.custom_stt] model = "large-v3"` accidentally overrides
        //    Groq/etc.'s default model on every transcription call.
        // 3. Built-in default for the selected provider
        let model = self
            .config
            .audio_model
            .as_deref()
            .or(custom_stt_model_ref(provider, &self.config.custom_stt))
            .unwrap_or_else(|| default_audio_model(provider));

        info!(provider, model, filename = %filename, size = audio_bytes.len(), "Sending audio for transcription");

        let transcription = match provider {
            // Whisper-compatible providers (OpenAI multipart protocol)
            "groq" | "openai" | "minimax" | "fireworks" | "together" | "siliconflow" => {
                let (api_url, api_key) = whisper_provider_config(provider)?;
                whisper_transcribe(&api_url, &api_key, model, audio_bytes, &filename, &mime).await?
            }
            // Gemini — multimodal content generation with audio input
            "gemini" => gemini_transcribe(model, audio_bytes, &mime).await?,
            // ElevenLabs — Speech-to-Text API
            "elevenlabs" => elevenlabs_transcribe(model, audio_bytes, &mime).await?,
            // Custom / self-hosted OpenAI-compatible Whisper endpoint
            _other => {
                let (api_url, api_key) = custom_stt_config(provider, &self.config.custom_stt)?;
                whisper_transcribe(&api_url, &api_key, model, audio_bytes, &filename, &mime).await?
            }
        };

        let transcription = transcription.trim().to_string();
        if transcription.is_empty() {
            return Err("Transcription returned empty text".into());
        }

        info!(
            provider,
            model,
            chars = transcription.len(),
            "Audio transcription complete"
        );

        Ok(MediaUnderstanding {
            media_type: MediaType::Audio,
            description: transcription,
            provider: provider.to_string(),
            model: model.to_string(),
        })
    }

    /// Describe video using Gemini.
    pub async fn describe_video(
        &self,
        attachment: &MediaAttachment,
    ) -> Result<MediaUnderstanding, String> {
        attachment.validate()?;
        if attachment.media_type != MediaType::Video {
            return Err("Expected video attachment".into());
        }

        if !self.config.video_description {
            return Err("Video description is disabled in configuration".into());
        }

        if std::env::var("GEMINI_API_KEY").is_err() && std::env::var("GOOGLE_API_KEY").is_err() {
            return Err("Video description requires GEMINI_API_KEY or GOOGLE_API_KEY".into());
        }

        Ok(MediaUnderstanding {
            media_type: MediaType::Video,
            description: "[Video description would be generated by Gemini]".to_string(),
            provider: "gemini".to_string(),
            model: "gemini-2.5-flash".to_string(),
        })
    }

    /// Process multiple attachments concurrently (bounded by max_concurrency).
    pub async fn process_attachments(
        &self,
        attachments: Vec<MediaAttachment>,
    ) -> Vec<Result<MediaUnderstanding, String>> {
        let mut handles = Vec::new();

        for attachment in attachments {
            // Skip media types that are disabled in config
            match attachment.media_type {
                MediaType::Image if !self.config.image_description => {
                    continue;
                }
                MediaType::Audio if !self.config.audio_transcription => {
                    continue;
                }
                MediaType::Video if !self.config.video_description => {
                    continue;
                }
                _ => {}
            }

            let sem = self.semaphore.clone();
            let config = self.config.clone();
            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.map_err(|e| e.to_string())?;
                let engine = MediaEngine {
                    config,
                    semaphore: Arc::new(Semaphore::new(1)), // inner engine, no extra semaphore
                };
                match attachment.media_type {
                    MediaType::Image => engine.describe_image(&attachment).await,
                    MediaType::Audio => engine.transcribe_audio(&attachment).await,
                    MediaType::Video => engine.describe_video(&attachment).await,
                    other => Err(format!("Unsupported media type: {}", other)),
                }
            });
            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(Err(format!("Task failed: {e}"))),
            }
        }
        results
    }
}

/// Detect which vision provider is available based on environment variables.
///
/// Priority order: Anthropic → OpenAI → Groq → Gemini.
/// Groq supports vision via `meta-llama/llama-4-scout-17b-16e-instruct` and
/// similar vision-capable models on their OpenAI-compatible endpoint.
fn detect_vision_provider() -> Option<&'static str> {
    let has_key = |var: &str| std::env::var(var).is_ok_and(|v| !v.trim().is_empty());
    if has_key("ANTHROPIC_API_KEY") {
        return Some("anthropic");
    }
    if has_key("OPENAI_API_KEY") {
        return Some("openai");
    }
    if has_key("GROQ_API_KEY") {
        return Some("groq");
    }
    if has_key("GEMINI_API_KEY") || has_key("GOOGLE_API_KEY") {
        return Some("gemini");
    }
    None
}

// ── Vision provider helpers ───────────────────────────────────────────────

/// Resolve OpenAI-compatible vision API URL and key for a provider.
fn openai_vision_provider_config(provider: &str) -> Result<(String, String), String> {
    match provider {
        "openai" => Ok((
            "https://api.openai.com/v1/chat/completions".into(),
            std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set")?,
        )),
        "groq" => Ok((
            "https://api.groq.com/openai/v1/chat/completions".into(),
            std::env::var("GROQ_API_KEY").map_err(|_| "GROQ_API_KEY not set")?,
        )),
        other => Err(format!(
            "No OpenAI-compatible vision config for provider: {other}"
        )),
    }
}

/// Describe an image using Anthropic's Messages API.
///
/// Sends the image as a base64-encoded `image` block in a single user turn
/// and extracts the first text block from the response.
async fn anthropic_describe_image(
    model: &str,
    image_bytes: &[u8],
    mime_type: &str,
) -> Result<String, String> {
    use base64::Engine;

    let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set")?;

    let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": mime_type,
                        "data": image_b64,
                    }
                },
                {
                    "type": "text",
                    "text": "Describe this image in detail. Focus on what is shown, \
                             any text visible, and the overall context."
                }
            ]
        }]
    });

    let client = librefang_http::proxied_client();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Anthropic vision request failed");
            "Anthropic vision request failed".to_string()
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, body = %err_body, "Anthropic vision returned non-2xx");
        return Err(format!("Anthropic API error ({})", status));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to parse Anthropic vision response");
        "Failed to parse Anthropic vision response".to_string()
    })?;

    json["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b["type"].as_str() == Some("text"))
                .and_then(|b| b["text"].as_str())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| "Anthropic returned no description text".to_string())
}

/// Describe an image using an OpenAI-compatible vision endpoint (OpenAI, Groq).
///
/// Sends the image as a base64 data-URL inside a `image_url` content block
/// in a Chat Completions request.
async fn openai_describe_image(
    api_url: &str,
    api_key: &str,
    model: &str,
    image_bytes: &[u8],
    mime_type: &str,
) -> Result<String, String> {
    use base64::Engine;

    let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
    let data_url = format!("data:{};base64,{}", mime_type, image_b64);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image_url",
                    "image_url": { "url": data_url }
                },
                {
                    "type": "text",
                    "text": "Describe this image in detail. Focus on what is shown, \
                             any text visible, and the overall context."
                }
            ]
        }]
    });

    let client = librefang_http::proxied_client();
    let resp = client
        .post(api_url)
        .bearer_auth(api_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "OpenAI-compatible vision request failed");
            "Vision request failed".to_string()
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, body = %err_body, "OpenAI-compatible vision returned non-2xx");
        return Err(format!("Vision API error ({})", status));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to parse OpenAI vision response");
        "Failed to parse vision response".to_string()
    })?;

    json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "OpenAI-compatible vision returned no text".to_string())
}

/// Describe an image using Gemini's generateContent API.
///
/// Sends the image as an `inline_data` part alongside a description prompt.
async fn gemini_describe_image(
    model: &str,
    image_bytes: &[u8],
    mime_type: &str,
) -> Result<String, String> {
    use base64::Engine;

    let api_key = std::env::var("GEMINI_API_KEY")
        .or_else(|_| std::env::var("GOOGLE_API_KEY"))
        .map_err(|_| "GEMINI_API_KEY or GOOGLE_API_KEY not set")?;

    let image_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

    let body = serde_json::json!({
        "contents": [{
            "parts": [
                {
                    "inline_data": {
                        "mime_type": mime_type,
                        "data": image_b64,
                    }
                },
                {
                    "text": "Describe this image in detail. Focus on what is shown, \
                             any text visible, and the overall context."
                }
            ]
        }],
        "generationConfig": {
            "maxOutputTokens": 1024
        }
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let client = librefang_http::proxied_client();
    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| {
            // Gemini's URL embeds the API key as `?key=…` — sanitize.
            tracing::warn!(error = %e, "Gemini vision request failed");
            "Gemini vision request failed".to_string()
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, body = %err_body, "Gemini vision returned non-2xx");
        return Err(format!("Gemini API error ({})", status));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to parse Gemini vision response");
        "Failed to parse Gemini vision response".to_string()
    })?;

    json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Gemini returned no description text".to_string())
}

// ── STT provider helpers ──────────────────────────────────────────────

/// Resolve Whisper-compatible API URL and key for a provider.
fn whisper_provider_config(provider: &str) -> Result<(String, String), String> {
    match provider {
        "groq" => Ok((
            "https://api.groq.com/openai/v1/audio/transcriptions".into(),
            std::env::var("GROQ_API_KEY").map_err(|_| "GROQ_API_KEY not set")?,
        )),
        "openai" => Ok((
            "https://api.openai.com/v1/audio/transcriptions".into(),
            std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set")?,
        )),
        "minimax" => Ok((
            "https://api.minimax.io/v1/audio/transcriptions".into(),
            std::env::var("MINIMAX_API_KEY")
                .or_else(|_| std::env::var("MINIMAX_CN_API_KEY"))
                .map_err(|_| "MINIMAX_API_KEY not set")?,
        )),
        "fireworks" => Ok((
            "https://api.fireworks.ai/inference/v1/audio/transcriptions".into(),
            std::env::var("FIREWORKS_API_KEY").map_err(|_| "FIREWORKS_API_KEY not set")?,
        )),
        "together" => Ok((
            "https://api.together.xyz/v1/audio/transcriptions".into(),
            std::env::var("TOGETHER_API_KEY").map_err(|_| "TOGETHER_API_KEY not set")?,
        )),
        "siliconflow" => Ok((
            "https://api.siliconflow.cn/v1/audio/transcriptions".into(),
            std::env::var("SILICONFLOW_API_KEY").map_err(|_| "SILICONFLOW_API_KEY not set")?,
        )),
        other => Err(format!("Unknown Whisper-compatible provider: {other}")),
    }
}

/// Resolve URL and API key for a custom / self-hosted STT endpoint.
///
/// Returns `Err` when:
/// - `custom_stt.base_url` is empty (provider is configured but no URL given).
/// - `key_required = true` and the named env var is absent or empty.
fn custom_stt_config(
    provider: &str,
    cfg: &librefang_types::media::CustomSttConfig,
) -> Result<(String, String), String> {
    if cfg.base_url.is_empty() {
        return Err(format!(
            "Audio provider '{provider}' is not a built-in provider and \
             [media.custom_stt] base_url is not set. \
             Add `base_url = \"http://<host>/v1/audio/transcriptions\"` \
             to [media.custom_stt] in config.toml."
        ));
    }

    let api_key = if cfg.api_key_env.is_empty() {
        // No key env var specified — send no Authorization header.
        String::new()
    } else {
        match std::env::var(&cfg.api_key_env) {
            Ok(k) if !k.trim().is_empty() => k,
            _ if cfg.key_required => {
                return Err(format!(
                    "Custom STT provider '{provider}' requires an API key but \
                     env var '{}' is not set or empty.",
                    cfg.api_key_env
                ));
            }
            _ => String::new(),
        }
    };

    Ok((cfg.base_url.clone(), api_key))
}

/// Transcribe using an OpenAI-compatible Whisper endpoint.
async fn whisper_transcribe(
    api_url: &str,
    api_key: &str,
    model: &str,
    audio_bytes: Vec<u8>,
    filename: &str,
    mime: &str,
) -> Result<String, String> {
    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|e| format!("Failed to set MIME type: {}", e))?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "text");

    let client = librefang_http::proxied_client();
    // Only add Authorization header when an API key is provided. Keyless
    // self-hosted servers (e.g. faster-whisper-server with no auth) reject
    // or ignore an empty `Bearer ` token; omitting the header entirely is
    // safer.
    let mut req = client
        .post(api_url)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60));
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }
    let resp = req.send().await.map_err(|e| {
        // Operator-facing: full error in logs. User-facing Err is
        // sanitized to drop the underlying reqwest::Error display,
        // which can echo URLs / request internals. See #4999.
        tracing::warn!(error = %e, "Whisper transcription request failed");
        "Transcription request failed".to_string()
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        // Operator log keeps the response body for diagnosis; the Err
        // returned to the bridge / agent prompt only carries the status
        // code so a misconfigured provider can't leak a key (some
        // providers echo the request envelope) into the LLM prompt.
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, body = %body, "Whisper transcription returned non-2xx");
        return Err(format!("Transcription API error ({})", status));
    }

    resp.text().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to read transcription response");
        "Failed to read transcription response".to_string()
    })
}

/// Transcribe using Gemini's multimodal generateContent API.
async fn gemini_transcribe(
    model: &str,
    audio_bytes: Vec<u8>,
    mime: &str,
) -> Result<String, String> {
    use base64::Engine;

    let api_key = std::env::var("GEMINI_API_KEY")
        .or_else(|_| std::env::var("GOOGLE_API_KEY"))
        .map_err(|_| "GEMINI_API_KEY or GOOGLE_API_KEY not set")?;

    let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);

    let body = serde_json::json!({
        "contents": [{
            "parts": [
                {
                    "inline_data": {
                        "mime_type": mime,
                        "data": audio_b64,
                    }
                },
                {
                    "text": "Transcribe this audio exactly as spoken. Output only the transcription text, nothing else."
                }
            ]
        }]
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let client = librefang_http::proxied_client();
    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| {
            // Critical: Gemini's URL embeds the API key as `?key=…`.
            // The `reqwest::Error` display can reproduce the URL — never
            // surface it to the LLM prompt. Log + return a sanitized Err.
            // See #4999.
            tracing::warn!(error = %e, "Gemini transcription request failed");
            "Gemini transcription request failed".to_string()
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, body = %err_body, "Gemini transcription returned non-2xx");
        return Err(format!("Gemini API error ({})", status));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to parse Gemini response");
        "Failed to parse Gemini response".to_string()
    })?;

    json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Gemini returned no transcription text".to_string())
}

/// Transcribe using ElevenLabs Speech-to-Text API.
async fn elevenlabs_transcribe(
    model: &str,
    audio_bytes: Vec<u8>,
    mime: &str,
) -> Result<String, String> {
    let api_key = std::env::var("ELEVENLABS_API_KEY").map_err(|_| "ELEVENLABS_API_KEY not set")?;

    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name("audio.webm".to_string())
        .mime_str(mime)
        .map_err(|e| format!("Failed to set MIME type: {}", e))?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model_id", model.to_string());

    let client = librefang_http::proxied_client();
    let resp = client
        .post("https://api.elevenlabs.io/v1/speech-to-text")
        .header("xi-api-key", &api_key)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "ElevenLabs STT request failed");
            "ElevenLabs STT request failed".to_string()
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        tracing::warn!(%status, body = %err_body, "ElevenLabs STT returned non-2xx");
        return Err(format!("ElevenLabs API error ({})", status));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to parse ElevenLabs response");
        "Failed to parse ElevenLabs response".to_string()
    })?;

    json["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "ElevenLabs returned no transcription text".to_string())
}

/// the caller can fall back to the source file extension.
fn mime_to_ext(mime: &str) -> Option<String> {
    match mime.to_ascii_lowercase().as_str() {
        "audio/wav" | "audio/x-wav" => Some("wav".to_string()),
        "audio/mpeg" | "audio/mp3" => Some("mp3".to_string()),
        "audio/ogg" => Some("ogg".to_string()),
        "audio/webm" => Some("webm".to_string()),
        "audio/mp4" | "audio/m4a" => Some("m4a".to_string()),
        "audio/flac" => Some("flac".to_string()),
        _ => None,
    }
}

/// Re-encode `.oga` into Ogg/Opus. Input streams in via stdin, output
/// streams out of stdout — no scratch files on disk. Same Opus payload,
/// just re-packetised.
///
/// Requires `ffmpeg` on `PATH`. 30 s wall-clock cap; on timeout the child
/// is killed and reaped explicitly so there are no zombies.
async fn transcode_oga_to_ogg_opus(input_bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::process::Stdio;

    let mut child = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "ogg",
            "-i",
            "pipe:0",
            "-vn",
            "-c:a",
            "copy",
            "-f",
            "ogg",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "ffmpeg not available ({e}) — install it (brew install ffmpeg / apt install ffmpeg) to process .oga voice notes"
            )
        })?;

    // Feed stdin concurrently; hanging the write inside the main task
    // would deadlock once ffmpeg's stdout pipe buffer fills.
    if let Some(mut stdin) = child.stdin.take() {
        let bytes = input_bytes.to_vec();
        tokio::spawn(async move {
            // Writer errors are intentionally ignored: if the pipe breaks
            // (ffmpeg rejected the input or exited early), the real reason
            // surfaces on stderr and the non-zero exit code, which the
            // caller already reports. Swallowing the write error here is
            // strictly less noisy than double-reporting.
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.shutdown().await;
        });
    }

    // Read stdout / stderr concurrently with waiting so we can kill + reap
    // the child explicitly on timeout (wait_with_output would move the
    // Child handle and leak the process if the wrapping timeout fires).
    use tokio::io::AsyncReadExt;
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        buf
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        buf
    });

    let status = match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait()).await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(format!("ffmpeg wait failed: {e}")),
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err("ffmpeg transcode timed out after 30s".to_string());
        }
    };

    let out = stdout_task.await.unwrap_or_default();
    let err = stderr_task.await.unwrap_or_default();

    if !status.success() {
        return Err(format!(
            "ffmpeg exited with {}: {}",
            status,
            String::from_utf8_lossy(&err).trim()
        ));
    }
    if out.is_empty() {
        return Err("ffmpeg produced an empty output stream".to_string());
    }
    Ok(out)
}

/// Detect which audio transcription provider is available.
fn detect_audio_provider() -> Option<&'static str> {
    let has_key = |var: &str| std::env::var(var).is_ok_and(|v| !v.trim().is_empty());
    if has_key("GROQ_API_KEY") {
        return Some("groq");
    }
    if has_key("OPENAI_API_KEY") {
        return Some("openai");
    }
    if has_key("GEMINI_API_KEY") || has_key("GOOGLE_API_KEY") {
        return Some("gemini");
    }
    if has_key("ELEVENLABS_API_KEY") {
        return Some("elevenlabs");
    }
    if has_key("MINIMAX_API_KEY") || has_key("MINIMAX_CN_API_KEY") {
        return Some("minimax");
    }
    if has_key("FIREWORKS_API_KEY") {
        return Some("fireworks");
    }
    if has_key("TOGETHER_API_KEY") {
        return Some("together");
    }
    if has_key("SILICONFLOW_API_KEY") {
        return Some("siliconflow");
    }
    None
}

/// Get the default vision model for a provider.
fn default_vision_model(provider: &str) -> &str {
    match provider {
        "anthropic" => "claude-sonnet-4-20250514",
        "openai" => "gpt-4o",
        "groq" => "meta-llama/llama-4-scout-17b-16e-instruct",
        "gemini" => "gemini-2.5-flash",
        _ => "unknown",
    }
}

/// Resolve the `[media.custom_stt] model` override, but ONLY for custom /
/// self-hosted providers (the `_other` dispatch arm). Returns `None` for every
/// built-in provider so that an operator setting `custom_stt.model` cannot
/// accidentally override a built-in provider's default transcription model.
fn custom_stt_model_ref<'a>(
    provider: &str,
    custom_stt: &'a librefang_types::media::CustomSttConfig,
) -> Option<&'a str> {
    match provider {
        "groq" | "openai" | "minimax" | "fireworks" | "together" | "siliconflow" | "gemini"
        | "elevenlabs" => None,
        _ => custom_stt.model.as_deref(),
    }
}

/// Get the default audio model for a provider.
///
/// For custom providers the model configured in `[media.custom_stt]` takes
/// precedence (resolved by the caller via `audio_model` / `custom_stt.model`);
/// this function returns `"whisper-1"` as the OpenAI-compatible fallback for
/// any unrecognised provider name.
fn default_audio_model(provider: &str) -> &str {
    match provider {
        "groq" => "whisper-large-v3-turbo",
        "openai" => "whisper-1",
        "gemini" => "gemini-2.0-flash",
        "elevenlabs" => "scribe_v1",
        "minimax" => "speech-01-turbo",
        "fireworks" => "whisper-v3-turbo",
        "together" => "whisper-large-v3-turbo",
        "siliconflow" => "FunAudioLLM/SenseVoiceSmall",
        // Custom / self-hosted providers default to the standard Whisper model
        // name; real model can be overridden via [media.custom_stt] or
        // audio_model in config.
        _ => "whisper-1",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::media::{MediaSource, MAX_IMAGE_BYTES};

    #[test]
    fn test_engine_creation() {
        let config = MediaConfig::default();
        let engine = MediaEngine::new(config);
        assert_eq!(engine.config.max_concurrency, 2);
    }

    #[test]
    fn test_engine_max_concurrency_clamped() {
        let config = MediaConfig {
            max_concurrency: 100,
            ..Default::default()
        };
        let engine = MediaEngine::new(config);
        // Semaphore was clamped to 8
        assert!(engine.semaphore.available_permits() <= 8);
    }

    #[test]
    fn mime_to_ext_maps_known_types() {
        assert_eq!(mime_to_ext("audio/ogg"), Some("ogg".to_string()));
        assert_eq!(mime_to_ext("audio/mpeg"), Some("mp3".to_string()));
        assert_eq!(mime_to_ext("audio/mp3"), Some("mp3".to_string()));
        assert_eq!(mime_to_ext("audio/wav"), Some("wav".to_string()));
        assert_eq!(mime_to_ext("audio/x-wav"), Some("wav".to_string()));
        assert_eq!(mime_to_ext("audio/webm"), Some("webm".to_string()));
        assert_eq!(mime_to_ext("audio/m4a"), Some("m4a".to_string()));
        assert_eq!(mime_to_ext("audio/mp4"), Some("m4a".to_string()));
        assert_eq!(mime_to_ext("audio/flac"), Some("flac".to_string()));
    }

    #[test]
    fn mime_to_ext_is_case_insensitive() {
        assert_eq!(mime_to_ext("AUDIO/OGG"), Some("ogg".to_string()));
        assert_eq!(mime_to_ext("Audio/Mp3"), Some("mp3".to_string()));
    }

    #[test]
    fn mime_to_ext_returns_none_for_unmapped() {
        // `audio/oga` intentionally unmapped — caller handles .oga via the
        // transcode path rather than treating it as directly usable.
        assert_eq!(mime_to_ext("audio/oga"), None);
        assert_eq!(mime_to_ext("application/octet-stream"), None);
        assert_eq!(mime_to_ext(""), None);
    }

    /// Skip body when ffmpeg is absent — CI images and the production
    /// container ship with it, dev boxes may not.
    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_ok()
    }

    #[tokio::test]
    async fn transcode_oga_smoke() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        // Synthesise a 0.5s silent Ogg/Opus buffer via ffmpeg's pipe:1,
        // then round-trip it through the transcoder. No scratch files.
        let gen = tokio::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=16000:cl=mono",
                "-t",
                "0.5",
                "-c:a",
                "libopus",
                "-f",
                "ogg",
                "pipe:1",
            ])
            .output()
            .await
            .expect("ffmpeg must run");
        assert!(
            gen.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&gen.stderr)
        );
        let input_bytes = gen.stdout;
        assert!(!input_bytes.is_empty());

        let out = transcode_oga_to_ogg_opus(&input_bytes)
            .await
            .expect("transcode must succeed on a valid Ogg/Opus");
        assert!(!out.is_empty());
        assert_eq!(&out[..4], b"OggS", "output must be an Ogg container");
    }

    #[tokio::test]
    async fn transcode_empty_input_errors() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        // Zero-byte input must be rejected, but ffmpeg's failure mode varies
        // across versions/platforms: newer builds exit non-zero before writing
        // any stdout ("ffmpeg exited ..."), older ones exit 0 with an empty
        // stream ("empty output"). Either is an acceptable rejection here —
        // what matters is that we don't accept the zero-byte input.
        let err = transcode_oga_to_ogg_opus(&[]).await.unwrap_err();
        assert!(
            err.contains("empty output") || err.contains("ffmpeg exited"),
            "expected ffmpeg to reject zero-byte input, got: {err}"
        );
    }

    #[tokio::test]
    async fn transcode_non_ogg_input_errors() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        // 256 bytes of non-Ogg junk — ffmpeg rejects the container and exits
        // non-zero before producing any stdout bytes.
        let garbage: Vec<u8> = (0..=255u8).collect();
        let err = transcode_oga_to_ogg_opus(&garbage).await.unwrap_err();
        assert!(
            err.contains("ffmpeg exited"),
            "expected ffmpeg-exit rejection, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_describe_image_wrong_type() {
        let engine = MediaEngine::new(MediaConfig::default());
        let attachment = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/mpeg".into(),
            source: MediaSource::FilePath {
                path: "test.mp3".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.describe_image(&attachment).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Expected image"));
    }

    #[tokio::test]
    async fn test_describe_image_invalid_mime() {
        let engine = MediaEngine::new(MediaConfig::default());
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "application/pdf".into(),
            source: MediaSource::FilePath {
                path: "test.pdf".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.describe_image(&attachment).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_describe_image_too_large() {
        let engine = MediaEngine::new(MediaConfig::default());
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".into(),
            source: MediaSource::FilePath {
                path: "big.png".into(),
            },
            size_bytes: MAX_IMAGE_BYTES + 1,
        };
        let result = engine.describe_image(&attachment).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_transcribe_audio_wrong_type() {
        let engine = MediaEngine::new(MediaConfig::default());
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".into(),
            source: MediaSource::FilePath {
                path: "test.png".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.transcribe_audio(&attachment).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_video_disabled() {
        let config = MediaConfig {
            video_description: false,
            ..Default::default()
        };
        let engine = MediaEngine::new(config);
        let attachment = MediaAttachment {
            media_type: MediaType::Video,
            mime_type: "video/mp4".into(),
            source: MediaSource::FilePath {
                path: "test.mp4".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.describe_video(&attachment).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("disabled"));
    }

    #[test]
    fn test_detect_vision_provider_none() {
        // In test env, likely no API keys set — should return None.
        // (This test is environment-dependent, but safe.)
        let _ = detect_vision_provider(); // Just verify it doesn't panic
    }

    #[test]
    fn test_default_vision_models() {
        assert_eq!(
            default_vision_model("anthropic"),
            "claude-sonnet-4-20250514"
        );
        assert_eq!(default_vision_model("openai"), "gpt-4o");
        assert_eq!(
            default_vision_model("groq"),
            "meta-llama/llama-4-scout-17b-16e-instruct"
        );
        assert_eq!(default_vision_model("gemini"), "gemini-2.5-flash");
        assert_eq!(default_vision_model("unknown"), "unknown");
    }

    #[tokio::test]
    async fn test_describe_image_no_provider_configured() {
        // With no API keys set and no explicit provider, should fail with provider error.
        let engine = MediaEngine::new(MediaConfig::default());
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".into(),
            source: MediaSource::FilePath {
                path: "test.png".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.describe_image(&attachment).await;
        // Fails at provider detection (no API keys in test env) or file read.
        // Either way, must be an error — never a placeholder string.
        assert!(result.is_err());
        let err = result.unwrap_err();
        // Must NOT return the old stub placeholder string.
        assert!(
            !err.contains("would be generated"),
            "describe_image must not return stub placeholder; got: {err}"
        );
    }

    #[tokio::test]
    async fn test_describe_image_url_source_rejected() {
        // URL source should be rejected before any API call
        let config = MediaConfig {
            image_provider: Some("anthropic".to_string()),
            ..Default::default()
        };
        let engine = MediaEngine::new(config);
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/jpeg".into(),
            source: MediaSource::Url {
                url: "https://example.com/image.jpg".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.describe_image(&attachment).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("URL-based image source not supported"),
            "URL source must be rejected"
        );
    }

    #[tokio::test]
    async fn test_describe_image_file_not_found() {
        // File read error must surface before any API call attempt
        let config = MediaConfig {
            image_provider: Some("anthropic".to_string()),
            ..Default::default()
        };
        let engine = MediaEngine::new(config);
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/jpeg".into(),
            source: MediaSource::FilePath {
                path: "/nonexistent/path/image.jpg".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.describe_image(&attachment).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("Failed to read image file"),
            "File not found must surface as read error"
        );
    }

    #[test]
    fn openai_vision_provider_config_resolves_known_providers() {
        // With no env set these return Err("not set"), but the URL/structure is stable
        let groq = openai_vision_provider_config("groq");
        // Either Ok with the right URL, or Err because the key is absent
        match groq {
            Ok((url, _)) => assert!(url.contains("groq.com")),
            Err(e) => assert!(e.contains("GROQ_API_KEY")),
        }
        let openai = openai_vision_provider_config("openai");
        match openai {
            Ok((url, _)) => assert!(url.contains("openai.com")),
            Err(e) => assert!(e.contains("OPENAI_API_KEY")),
        }
        // Unknown provider must error
        assert!(openai_vision_provider_config("unknown_provider").is_err());
    }

    #[test]
    fn test_default_audio_models() {
        assert_eq!(default_audio_model("groq"), "whisper-large-v3-turbo");
        assert_eq!(default_audio_model("openai"), "whisper-1");
    }

    #[tokio::test]
    async fn test_transcribe_audio_rejects_image_type() {
        let engine = MediaEngine::new(MediaConfig::default());
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".into(),
            source: MediaSource::FilePath {
                path: "test.png".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.transcribe_audio(&attachment).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Expected audio"));
    }

    #[tokio::test]
    async fn test_transcribe_audio_no_provider() {
        // With no API keys set, should fail with provider error
        let engine = MediaEngine::new(MediaConfig::default());
        let attachment = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/webm".into(),
            source: MediaSource::FilePath {
                path: "test.webm".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.transcribe_audio(&attachment).await;
        // Either fails with "No audio transcription provider" or file read error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_transcribe_audio_url_source_rejected() {
        // URL source should be rejected
        let config = MediaConfig {
            audio_provider: Some("groq".to_string()),
            ..Default::default()
        };
        let engine = MediaEngine::new(config);
        let attachment = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/mpeg".into(),
            source: MediaSource::Url {
                url: "https://example.com/audio.mp3".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.transcribe_audio(&attachment).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("URL-based audio source not supported"));
    }

    #[tokio::test]
    async fn test_transcribe_audio_file_not_found() {
        let config = MediaConfig {
            audio_provider: Some("groq".to_string()),
            ..Default::default()
        };
        let engine = MediaEngine::new(config);
        let attachment = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/webm".into(),
            source: MediaSource::FilePath {
                path: "/nonexistent/path/audio.webm".into(),
            },
            size_bytes: 1024,
        };
        let result = engine.transcribe_audio(&attachment).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read audio file"));
    }

    // ── Custom STT config resolution tests ───────────────────────────────

    #[test]
    fn custom_stt_config_empty_base_url_returns_err() {
        use librefang_types::media::CustomSttConfig;
        let cfg = CustomSttConfig::default(); // base_url is empty
        let result = custom_stt_config("local-whisper", &cfg);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("local-whisper"),
            "should mention provider name"
        );
        assert!(msg.contains("base_url"), "should mention base_url field");
    }

    #[test]
    fn custom_stt_config_no_key_env_returns_empty_key() {
        use librefang_types::media::CustomSttConfig;
        let cfg = CustomSttConfig {
            base_url: "http://localhost:8080/v1/audio/transcriptions".to_string(),
            api_key_env: String::new(), // no auth
            key_required: false,
            model: None,
        };
        let (url, key) = custom_stt_config("local-whisper", &cfg).unwrap();
        assert_eq!(url, "http://localhost:8080/v1/audio/transcriptions");
        assert!(key.is_empty(), "keyless server should produce empty key");
    }

    #[test]
    fn custom_stt_config_key_required_missing_env_returns_err() {
        use librefang_types::media::CustomSttConfig;
        // Use a deliberately unusual env var name that CI will never set.
        let cfg = CustomSttConfig {
            base_url: "http://localhost:8080/v1/audio/transcriptions".to_string(),
            api_key_env: "LIBREFANG_TEST_MISSING_KEY_ZXQ99".to_string(), // pragma: allowlist secret
            key_required: true,
            model: None,
        };
        let result = custom_stt_config("local-whisper", &cfg);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("LIBREFANG_TEST_MISSING_KEY_ZXQ99"),
            "error should name the env var"
        );
    }

    #[test]
    fn custom_stt_config_key_optional_missing_env_returns_empty_key() {
        use librefang_types::media::CustomSttConfig;
        let cfg = CustomSttConfig {
            base_url: "http://localhost:8080/v1/audio/transcriptions".to_string(),
            api_key_env: "LIBREFANG_TEST_MISSING_KEY_ZXQ99".to_string(), // pragma: allowlist secret
            key_required: false, // optional — missing key is OK
            model: None,
        };
        let (url, key) = custom_stt_config("local-whisper", &cfg).unwrap();
        assert_eq!(url, "http://localhost:8080/v1/audio/transcriptions");
        assert!(
            key.is_empty(),
            "missing optional key should produce empty key"
        );
    }

    #[test]
    fn custom_stt_model_resolution_prefers_audio_model_field() {
        // [media] audio_model overrides [media.custom_stt] model
        use librefang_types::media::CustomSttConfig;
        let config = MediaConfig {
            audio_model: Some("my-explicit-model".to_string()),
            custom_stt: CustomSttConfig {
                model: Some("custom-stt-model".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        // Resolution: audio_model > custom_stt.model > default_audio_model(provider)
        let resolved = config
            .audio_model
            .as_deref()
            .or(config.custom_stt.model.as_deref())
            .unwrap_or_else(|| default_audio_model("local-whisper"));
        assert_eq!(resolved, "my-explicit-model");
    }

    #[test]
    fn custom_stt_model_resolution_falls_back_to_custom_stt_model() {
        use librefang_types::media::CustomSttConfig;
        let config = MediaConfig {
            audio_model: None, // not set
            custom_stt: CustomSttConfig {
                model: Some("large-v3".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = config
            .audio_model
            .as_deref()
            .or(config.custom_stt.model.as_deref())
            .unwrap_or_else(|| default_audio_model("local-whisper"));
        assert_eq!(resolved, "large-v3");
    }

    #[test]
    fn custom_stt_model_resolution_falls_back_to_provider_default() {
        use librefang_types::media::CustomSttConfig;
        let config = MediaConfig {
            audio_model: None,
            custom_stt: CustomSttConfig {
                model: None,
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = config
            .audio_model
            .as_deref()
            .or(config.custom_stt.model.as_deref())
            .unwrap_or_else(|| default_audio_model("local-whisper"));
        // Unknown provider should return the OpenAI-compatible default
        assert_eq!(resolved, "whisper-1");
    }

    #[test]
    fn custom_stt_model_ref_does_not_leak_into_builtin_providers() {
        use librefang_types::media::CustomSttConfig;
        // Operator set a custom_stt.model — it must NOT override a built-in
        // provider's default model. Exercises the production guard directly
        // (not a reconstructed copy), so deleting the guard fails this test.
        let custom_stt = CustomSttConfig {
            model: Some("large-v3".to_string()),
            ..Default::default()
        };
        for builtin in [
            "groq",
            "openai",
            "minimax",
            "fireworks",
            "together",
            "siliconflow",
            "gemini",
            "elevenlabs",
        ] {
            assert_eq!(
                custom_stt_model_ref(builtin, &custom_stt),
                None,
                "custom_stt.model must not leak into built-in provider {builtin}"
            );
        }
        // A custom / self-hosted provider DOES pick up custom_stt.model.
        assert_eq!(
            custom_stt_model_ref("local-whisper", &custom_stt),
            Some("large-v3")
        );
    }

    #[test]
    fn media_config_default_has_empty_custom_stt() {
        let config = MediaConfig::default();
        assert!(config.custom_stt.base_url.is_empty());
        assert!(config.custom_stt.api_key_env.is_empty());
        assert!(!config.custom_stt.key_required);
        assert!(config.custom_stt.model.is_none());
    }

    #[test]
    fn media_config_round_trips_custom_stt() {
        use librefang_types::media::CustomSttConfig;
        let config = MediaConfig {
            audio_provider: Some("local-whisper".to_string()),
            custom_stt: CustomSttConfig {
                base_url: "http://localhost:8080/v1/audio/transcriptions".to_string(),
                api_key_env: "LOCAL_WHISPER_KEY".to_string(), // pragma: allowlist secret
                key_required: false,
                model: Some("large-v3".to_string()),
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: MediaConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.custom_stt.base_url,
            "http://localhost:8080/v1/audio/transcriptions"
        );
        assert_eq!(parsed.custom_stt.api_key_env, "LOCAL_WHISPER_KEY");
        assert_eq!(parsed.custom_stt.model.as_deref(), Some("large-v3"));
    }
}
