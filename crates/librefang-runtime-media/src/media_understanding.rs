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

/// Record a media-understanding failure so operators can alert on a provider /
/// model that has silently stopped working — e.g. a hosted model retired by the
/// provider (`model_decommissioned`), which previously degraded to a raw
/// image/audio passthrough with only a `warn!` in the channel bridge and no
/// metric to diff against the hardcoded default (#6538).
///
/// Emits the `librefang_media_understanding_failures_total` counter and a
/// structured `warn!` carrying the same fields, at the point of failure inside
/// the media engine so it is captured regardless of the caller (channel bridge,
/// tool path, …), not only in `bridge.rs`.
///
/// Cardinality: `kind` is one of {image, audio}; `provider` and `model` are
/// drawn from the operator-configured or built-in default set (bounded) —
/// matching the bounded-label discipline of the other LibreFang counters
/// (`librefang_mcp_reconnect_total`, `librefang_tool_call_total`, …).
fn record_media_understanding_failure(kind: &str, provider: &str, model: &str, error: &str) {
    metrics::counter!(
        "librefang_media_understanding_failures_total",
        "kind" => kind.to_string(),
        "provider" => provider.to_string(),
        "model" => model.to_string(),
    )
    .increment(1);
    tracing::warn!(
        kind,
        provider,
        model,
        error,
        "Media understanding failed — the selected provider/model returned an \
         error; a hosted model may have been retired, check the provider's \
         current model list against the configured/default model"
    );
}

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

        // Capture the provider dispatch as a `Result` (rather than `?`-ing each
        // arm) so any failure is counted with its provider/model before it
        // propagates — this is the "hosted model silently retired" signal (#6538).
        let dispatch_result: Result<String, String> = async {
            match provider {
                "anthropic" => anthropic_describe_image(model, &image_bytes, mime_type).await,
                "openai" | "groq" => {
                    let (api_url, api_key) = openai_vision_provider_config(provider)?;
                    openai_describe_image(&api_url, &api_key, model, &image_bytes, mime_type).await
                }
                "gemini" => gemini_describe_image(model, &image_bytes, mime_type).await,
                other => Err(format!("Unsupported image description provider: {}", other)),
            }
        }
        .await;
        let description = match dispatch_result {
            Ok(d) => d,
            Err(e) => {
                record_media_understanding_failure("image", provider, model, &e);
                return Err(e);
            }
        };

        let description = description.trim().to_string();
        if description.is_empty() {
            let e = "Image description returned empty text".to_string();
            record_media_understanding_failure("image", provider, model, &e);
            return Err(e);
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
    ///
    /// Transcribes the whole file.
    /// Callers that need to bound the request — anything driven by a recording whose length they do not control — want [`Self::transcribe_audio_window`] instead (#6748).
    pub async fn transcribe_audio(
        &self,
        attachment: &MediaAttachment,
        language: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<MediaUnderstanding, String> {
        self.transcribe_audio_window(attachment, language, prompt, None)
            .await
            .map(|outcome| outcome.understanding)
    }

    /// Transcribe one bounded window of a recording (#6748).
    ///
    /// `window: None` is exactly [`Self::transcribe_audio`] — whole file, no ffmpeg hop added for callers that never wanted one.
    /// `Some` cuts the window out with ffmpeg first, which subsumes the video-extraction and `.oga` re-mux branches: whatever the container was, what reaches the provider is the window as Ogg/Opus.
    ///
    /// The returned [`TranscriptionOutcome::consumed_secs`] is what the caller advances by for the next window, and comparing it against the requested length is how the end of the recording is detected.
    /// Neither is inferable from the transcript, which is why this returns more than a string.
    pub async fn transcribe_audio_window(
        &self,
        attachment: &MediaAttachment,
        language: Option<&str>,
        prompt: Option<&str>,
        window: Option<MediaWindow>,
    ) -> Result<TranscriptionOutcome, String> {
        attachment.validate()?;
        if attachment.media_type != MediaType::Audio && attachment.media_type != MediaType::Video {
            return Err("Expected audio or video attachment".into());
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

        // A windowed call over a file on disk never needs the bytes in memory: ffmpeg seeks into the file itself, and everything downstream works on the window it cuts.
        // Skipping the read here is what keeps a walk's cost proportional to the recording rather than to the recording times the number of windows — the read, and for a `Base64` attachment the decode as well, would otherwise be repeated in full for every window.
        let windowed_path = match (&attachment.source, window) {
            (MediaSource::FilePath { path }, Some(_)) => Some(std::path::PathBuf::from(path)),
            _ => None,
        };

        // Read attachment bytes from source. For a Video attachment these are
        // the still-muxed container bytes; the video branch below extracts
        // the audio track before anything past it runs.
        let mut audio_bytes = if windowed_path.is_some() {
            Vec::new()
        } else {
            match &attachment.source {
                MediaSource::FilePath { path } => tokio::fs::read(path).await.map_err(|e| {
                    format!(
                        "Failed to read {} file '{path}': {e}",
                        attachment.media_type
                    )
                })?,
                MediaSource::Base64 { data, .. } => {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(|e| {
                            format!("Failed to decode base64 {}: {e}", attachment.media_type)
                        })?
                }
                MediaSource::Url { url } => {
                    return Err(format!(
                        "URL-based source not supported for transcription: {url}"
                    ));
                }
                other => {
                    return Err(format!(
                        "Unsupported source variant for transcription: {other:?}"
                    ));
                }
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

        // Windowed call (#6748): one ffmpeg pass cuts the requested span and lands it on the same Ogg/Opus target the two branches below produce, so neither has anything left to do.
        // Handled first for that reason — running either of them before this would decode the whole recording just to throw most of it away.
        let mut consumed_secs = None;
        if let Some(window) = window {
            let source = match &windowed_path {
                Some(path) => WindowSource::Path(path.as_path()),
                None => WindowSource::Bytes(&audio_bytes),
            };
            let Some(cut) = extract_media_window(source, window)
                .await
                .map_err(|e| format!("ffmpeg window extraction failed: {e}"))?
            else {
                // Overshoot: the window began past the end of the recording.
                // Reported as a zero-length window rather than an error, so the caller's loop ends on `has_more: false` the way the tool description tells it to, instead of on a failure it has no way to distinguish from a broken file.
                return Ok(TranscriptionOutcome {
                    understanding: MediaUnderstanding {
                        media_type: MediaType::Audio,
                        description: String::new(),
                        provider: provider.to_string(),
                        // Named even though nothing was transcribed: the field describes the configuration the call ran under, and a caller reading the last step of a walk should not see it blank out.
                        model: self
                            .config
                            .audio_model
                            .as_deref()
                            .or(custom_stt_model_ref(provider, &self.config.custom_stt))
                            .unwrap_or_else(|| default_audio_model(provider))
                            .to_string(),
                    },
                    consumed_secs: Some(0.0),
                });
            };
            let produced = ogg_opus_duration_secs(&cut);
            info!(
                // From the attachment, not the buffer: a windowed call over a
                // file never fills the buffer, so reading its length here
                // logged every long recording as zero bytes.
                original_size = attachment.size_bytes,
                window_size = cut.len(),
                start_sec = window.start_sec,
                max_secs = window.max_secs,
                produced_secs = ?produced,
                "Cut a media window before Whisper upload"
            );
            audio_bytes = cut;
            ext = "ogg".to_string();
            mime = "audio/ogg".to_string();
            consumed_secs = produced;
        }

        // Video containers (#6679): drop the video stream and re-encode
        // whatever audio codec the container held to Ogg/Opus, the same
        // target the `.oga` path below produces — so the whisper-upload code
        // that follows never has to know a video container was involved.
        if window.is_none() && attachment.media_type == MediaType::Video {
            let extracted = extract_video_audio_track(&audio_bytes)
                .await
                .map_err(|e| format!("ffmpeg audio extraction failed: {e}"))?;
            info!(
                original_size = audio_bytes.len(),
                extracted_size = extracted.len(),
                container = %ext,
                "Extracted audio track from video before Whisper upload"
            );
            audio_bytes = extracted;
            ext = "ogg".to_string();
            mime = "audio/ogg".to_string();
        }

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

        // Per-call value first, `[media]` operator default as the fallback —
        // same precedence `tool_text_to_speech` already uses for TTS (#6678).
        // Only the whisper-protocol arms below receive these: Gemini and
        // ElevenLabs are separate provider contracts (multimodal content /
        // `model_id` form field) with no equivalent parameter here.
        let effective_language = language.or(self.config.audio_language.as_deref());
        let effective_prompt = prompt.or(self.config.audio_prompt.as_deref());

        // Capture the provider dispatch as a `Result` so any failure is counted
        // with its provider/model before propagating — the STT analogue of the
        // vision "hosted model silently retired" signal (#6538).
        let dispatch_result: Result<String, String> = async {
            match provider {
                // Whisper-compatible providers (OpenAI multipart protocol)
                "groq" | "openai" | "minimax" | "fireworks" | "together" | "siliconflow" => {
                    let (api_url, api_key) = whisper_provider_config(provider)?;
                    whisper_transcribe(WhisperTranscribeParams {
                        api_url: &api_url,
                        api_key: &api_key,
                        model,
                        audio_bytes,
                        filename: &filename,
                        mime: &mime,
                        language: effective_language,
                        prompt: effective_prompt,
                    })
                    .await
                }
                // Gemini — multimodal content generation with audio input
                "gemini" => gemini_transcribe(model, audio_bytes, &mime).await,
                // ElevenLabs — Speech-to-Text API
                "elevenlabs" => elevenlabs_transcribe(model, audio_bytes, &mime).await,
                // Custom / self-hosted OpenAI-compatible Whisper endpoint
                _other => {
                    let (api_url, api_key) = custom_stt_config(provider, &self.config.custom_stt)?;
                    whisper_transcribe(WhisperTranscribeParams {
                        api_url: &api_url,
                        api_key: &api_key,
                        model,
                        audio_bytes,
                        filename: &filename,
                        mime: &mime,
                        language: effective_language,
                        prompt: effective_prompt,
                    })
                    .await
                }
            }
        }
        .await;
        let transcription = match dispatch_result {
            Ok(t) => t,
            Err(e) => {
                record_media_understanding_failure("audio", provider, model, &e);
                return Err(e);
            }
        };

        let transcription = transcription.trim().to_string();
        if transcription.is_empty() {
            // A whole file that transcribes to nothing is a failure worth surfacing.
            // One *window* of a recording that transcribes to nothing is ordinary — a pause, a silent stretch, a gap between speakers — and erroring there would abort the walk with no resume point, stranding the rest of the recording behind a silence.
            if window.is_some() {
                info!(
                    provider,
                    "Window transcribed to no speech — continuing the walk"
                );
                return Ok(TranscriptionOutcome {
                    understanding: MediaUnderstanding {
                        media_type: MediaType::Audio,
                        description: String::new(),
                        provider: provider.to_string(),
                        model: model.to_string(),
                    },
                    consumed_secs,
                });
            }
            let e = "Transcription returned empty text".to_string();
            record_media_understanding_failure("audio", provider, model, &e);
            return Err(e);
        }

        info!(
            provider,
            model,
            chars = transcription.len(),
            "Audio transcription complete"
        );

        Ok(TranscriptionOutcome {
            understanding: MediaUnderstanding {
                media_type: MediaType::Audio,
                description: transcription,
                provider: provider.to_string(),
                model: model.to_string(),
            },
            consumed_secs,
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
                    MediaType::Audio => engine.transcribe_audio(&attachment, None, None).await,
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

/// Parameters for `whisper_transcribe`. A plain field bag rather than a
/// builder: this is a private helper with two call sites in this module,
/// not a public request type — the struct exists only to stay under
/// clippy's argument-count lint.
struct WhisperTranscribeParams<'a> {
    api_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    audio_bytes: Vec<u8>,
    filename: &'a str,
    mime: &'a str,
    language: Option<&'a str>,
    prompt: Option<&'a str>,
}

/// Transcribe using an OpenAI-compatible Whisper endpoint.
///
/// `language` and `prompt` are emitted only when set (#6678): an install
/// that configures neither sees byte-identical requests to before either
/// parameter existed.
async fn whisper_transcribe(params: WhisperTranscribeParams<'_>) -> Result<String, String> {
    let WhisperTranscribeParams {
        api_url,
        api_key,
        model,
        audio_bytes,
        filename,
        mime,
        language,
        prompt,
    } = params;

    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|e| format!("Failed to set MIME type: {}", e))?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "text");
    if let Some(lang) = language {
        form = form.text("language", lang.to_string());
    }
    if let Some(p) = prompt {
        form = form.text("prompt", p.to_string());
    }

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

/// A scratch file that deletes itself when it goes out of scope.
///
/// Used only where ffmpeg genuinely cannot work from a pipe — see `extract_video_audio_track`.
/// Backed by `tempfile::NamedTempFile` (already a workspace dependency used by `librefang-runtime` and others) rather than a hand-rolled pid+counter path.
/// `NamedTempFile` creates with `O_EXCL` and `0600` permissions on Unix, which closes both the symlink-race and world-readable-recording gaps a predictable path in the shared temp dir would otherwise open on a multi-user host — the staged bytes here are a user's raw audio/video.
struct ScopedTempFile {
    file: tempfile::NamedTempFile,
}

impl ScopedTempFile {
    /// Create a securely-named scratch file and write `bytes` into it.
    ///
    /// Runs on the blocking pool: creation plus a write of up to the 50 MB `MediaAttachment::validate()` cap is real (if brief) disk I/O, matching the `spawn_blocking`-for-temp-file-writes pattern used elsewhere in the workspace (e.g. `librefang-api`'s device-token and backup-archive writers).
    async fn write(bytes: &[u8], extension: &str) -> Result<Self, String> {
        use std::io::Write as _;

        let bytes = bytes.to_vec();
        let suffix = format!(".{extension}");
        tokio::task::spawn_blocking(move || {
            let mut file = tempfile::Builder::new()
                .prefix("librefang-media-")
                .suffix(&suffix)
                .tempfile()
                .map_err(|e| format!("failed to create scratch file for media extraction: {e}"))?;
            file.write_all(&bytes)
                .map_err(|e| format!("failed to stage media for extraction: {e}"))?;
            Ok(Self { file })
        })
        .await
        .map_err(|e| format!("scratch-file staging task panicked: {e}"))?
    }

    fn path(&self) -> &std::path::Path {
        self.file.path()
    }
}

/// Run `ffmpeg` with the given arguments and collect stdout.
///
/// `input_bytes` is `Some` when the input is fed on stdin (the args then name `pipe:0`), and `None` when the args already point at a path on disk.
/// The distinction matters: a pipe cannot seek, and some containers require a backward seek to be demuxed at all — see `extract_video_audio_track`.
///
/// Shared by every ffmpeg-based transcode in this module so the spawn / pipe / timeout / kill-on-timeout plumbing exists once.
///
/// `install_hint` names the feature that needs ffmpeg, so the "not on PATH"
/// error tells the operator what stopped working rather than just that a
/// subprocess failed to spawn. 30 s wall-clock cap; on timeout the child is
/// killed and reaped explicitly so there are no zombies.
async fn run_ffmpeg_pipe(
    args: &[&str],
    input_bytes: Option<&[u8]>,
    install_hint: &str,
) -> Result<Vec<u8>, String> {
    use std::process::Stdio;

    let mut child = tokio::process::Command::new("ffmpeg")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "ffmpeg not available ({e}) — install it (brew install ffmpeg / apt install ffmpeg) to {install_hint}"
            )
        })?;

    // Feed stdin concurrently; hanging the write inside the main task
    // would deadlock once ffmpeg's stdout pipe buffer fills.
    // With a file input there is nothing to write — dropping the handle closes it so ffmpeg does not wait on a stdin that will never arrive.
    match (child.stdin.take(), input_bytes) {
        (Some(mut stdin), Some(bytes)) => {
            let bytes = bytes.to_vec();
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
        (Some(stdin), None) => drop(stdin),
        (None, _) => {}
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

/// Re-encode `.oga` into Ogg/Opus. Same Opus payload, just re-packetised —
/// `-c:a copy` avoids a re-encode since the input is already Opus.
async fn transcode_oga_to_ogg_opus(input_bytes: &[u8]) -> Result<Vec<u8>, String> {
    run_ffmpeg_pipe(
        &[
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
        ],
        Some(input_bytes),
        "process .oga voice notes",
    )
    .await
}

/// Extract the audio track from a video container and re-encode it to
/// Ogg/Opus (#6679). Unlike the `.oga` re-mux above, this always re-encodes
/// (`-c:a libopus`) rather than copying: `mp4`/`mov`/`mkv`/`avi` carry
/// whatever audio codec the source used (AAC, PCM, Vorbis, AC3, …), and
/// re-encoding to one known-good target is what lets the same Whisper-upload
/// path handle all of them without per-codec branching. No `-f` is given on
/// the input side — ffmpeg auto-detects the container from its content,
/// which is what makes this work across `mp4`/`mov`/`mkv`/`avi` uniformly
/// rather than needing a format hint per extension.
///
/// The input is staged to a scratch file rather than piped, because a pipe cannot seek and ISO-BMFF cannot always be demuxed without seeking (#6747).
///
/// An `mp4` / `mov` stores its index in the `moov` atom.
/// When `moov` sits after `mdat` — the default output of ffmpeg's own muxer, phone cameras, screen recorders and meeting exporters — the demuxer has to seek backwards to read it.
/// Over `pipe:0` that fails, and it fails *quietly*: ffmpeg writes `Error during demuxing` to stderr but can still exit 0, emitting a header-only Ogg with zero audio packets.
/// Those bytes then went to the transcription provider, and the operator saw whatever a provider says about a soundless file — arbitrarily far from the real cause.
///
/// `mkv` and `avi` are streamable and were never affected, which is why the four container types enabled by #6679 / #6683 split exactly in half.
async fn extract_video_audio_track(input_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let staged = ScopedTempFile::write(input_bytes, "media").await?;
    let input_path = staged.path().to_string_lossy().into_owned();

    let out = run_ffmpeg_pipe(
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            &input_path,
            "-vn",
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-ar",
            "48000",
            "-ac",
            "1",
            "-f",
            "ogg",
            "pipe:1",
        ],
        None,
        "extract audio from video files",
    )
    .await?;

    // Defence in depth for the silent half of #6747.
    // The staged file fixes the cause, but neither guard in `run_ffmpeg_pipe` can catch a demux that fails while the process still exits 0 and still writes a container: the exit code is success and the output is not empty.
    // Exit-code behaviour also varies across ffmpeg builds, so it cannot be the only line.
    if !ogg_contains_audio(&out) {
        return Err(format!(
            "ffmpeg produced an Ogg stream with no audio packets ({} bytes) — \
             the container's audio track could not be decoded",
            out.len()
        ));
    }

    Ok(out)
}

/// Whether an Ogg stream carries audio, rather than only its headers.
///
/// An Opus stream opens with two header pages, `OpusHead` and `OpusTags`; audio data begins on the third.
/// Counting `OggS` page captures is enough to tell a real stream from the 261-byte headers-only artefact a failed demux produces, and does not require parsing the container.
fn ogg_contains_audio(bytes: &[u8]) -> bool {
    const OGG_PAGE_MAGIC: &[u8; 4] = b"OggS";
    bytes
        .windows(OGG_PAGE_MAGIC.len())
        .filter(|w| w == OGG_PAGE_MAGIC)
        .count()
        > 2
}

/// What a transcription call produced, plus how much media it consumed (#6748).
///
/// The transcript alone cannot answer "where does the next window start" or "was that the end of the recording", and both are needed to walk a long recording without either skipping audio or looping on the tail.
#[derive(Debug, Clone)]
pub struct TranscriptionOutcome {
    /// The transcript and the provider that produced it.
    pub understanding: MediaUnderstanding,
    /// Playable seconds actually transcribed — `None` when no window was requested (the whole file was sent) or when the produced stream carried no usable granule position.
    pub consumed_secs: Option<f64>,
}

/// A bounded slice of a recording, in seconds from the start of the media (#6748).
///
/// Exists because the transcription request is otherwise unbounded: its duration scales with the input, while the timeout guarding it does not.
/// Bounding the request is also what makes a defensible default timeout statable at all — an unbounded request has no correct value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaWindow {
    /// Offset of the window from the start of the media.
    pub start_sec: f64,
    /// Requested length.
    /// The produced window is shorter when the media ends first, which is how the end of the recording is detected.
    pub max_secs: f64,
}

/// Where the bytes of a window's source live (#6748).
///
/// A transcription driven by a tool call already has the recording on disk, and ffmpeg can seek into it directly.
/// One driven by an inbound attachment has only bytes, which have to reach a file before ffmpeg can seek at all — see #6747 for what happens when they do not.
#[derive(Debug, Clone, Copy)]
enum WindowSource<'a> {
    /// A file already on disk; used as-is.
    Path(&'a Path),
    /// Bytes in memory; staged to a scratch file first.
    Bytes(&'a [u8]),
}

/// Cut `[start_sec, start_sec + max_secs)` out of any container ffmpeg can open and re-encode it to the same Ogg/Opus target [`extract_video_audio_track`] produces (#6748).
///
/// The input is staged to a file for the same reason that path does it (#6747): a pipe cannot seek, `mp4` / `mov` keep their index in a trailing `moov` atom, and a demux that fails for want of a backward seek still exits 0 and still emits a headers-only Ogg.
/// Seeking is doubly load-bearing here — `-ss` before `-i` is what makes a window a seek rather than a decode of everything ahead of it, which on a 45-minute recording is the difference between constant time and re-decoding the whole file for every chunk.
///
/// The video stream is dropped unconditionally (`-vn`), so this subsumes the video-extraction path for windowed calls and needs no branch on media type.
///
/// A seek lands on the nearest keyframe, so the real window edge can differ from the requested one by a keyframe interval — recordings with a broken index (meeting exporters produce these routinely) seek especially coarsely.
/// That is why the caller advances by the *produced* duration from [`ogg_opus_duration_secs`] rather than by `max_secs`: an assumed edge would drift and eventually skip audio.
///
/// `Ok(None)` means the window carried no audio and started past the opening of the recording — an overshoot, which ends the walk rather than failing it.
async fn extract_media_window(
    input: WindowSource<'_>,
    window: MediaWindow,
) -> Result<Option<Vec<u8>>, String> {
    // Staging only when the caller has bytes rather than a file.
    // A source already on disk is already seekable, so copying it to scratch would buy nothing and cost a full write per window — and a walk issues one call per window, so that cost would be paid for the whole recording every time rather than once.
    let staged = match input {
        WindowSource::Bytes(bytes) => Some(ScopedTempFile::write(bytes, "media").await?),
        WindowSource::Path(_) => None,
    };
    let input_path = match (&staged, input) {
        (Some(tmp), _) => tmp.path().to_string_lossy().into_owned(),
        (None, WindowSource::Path(path)) => path.to_string_lossy().into_owned(),
        (None, WindowSource::Bytes(_)) => unreachable!("bytes are always staged"),
    };
    let start = format!("{:.3}", window.start_sec.max(0.0));
    let dur = format!("{:.3}", window.max_secs.max(0.0));

    let out = run_ffmpeg_pipe(
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &start,
            "-i",
            &input_path,
            "-t",
            &dur,
            "-vn",
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-ar",
            "48000",
            "-ac",
            "1",
            "-f",
            "ogg",
            "pipe:1",
        ],
        None,
        "transcribe a time window of a recording",
    )
    .await?;

    // Same defence in depth as the video path: a demux that fails for want of a seek still exits 0 and still emits a headers-only container, so neither guard in `run_ffmpeg_pipe` catches it.
    //
    // Unlike that path, an empty result here is not always a failure.
    // A window at or past the end of the recording is a legitimate request — the walk reaches one by itself whenever a recording's length is close to a multiple of `max_secs` — and it produces exactly the same headers-only bytes as a track that would not decode.
    //
    // Where the window starts is not enough to tell them apart, because `start_sec` is a parameter a caller sets directly: "transcribe from the tenth minute" of a file whose audio is broken would otherwise be reported as a successful empty window, and the caller would conclude there is nothing there.
    // That is exactly the silent failure #6747 exists to make loud, so it is not reintroduced on a technicality.
    // One cheap probe settles it: cut a second from the very start of the same source.
    // If that carries audio the track decodes fine and the empty window really is past the end; if it does not, the input is the problem and saying so is worth the extra second.
    if !ogg_contains_audio(&out) {
        if window.start_sec > 0.0 && source_decodes(&input_path).await? {
            return Ok(None);
        }
        return Err(format!(
            "ffmpeg produced an Ogg stream with no audio packets ({} bytes) — \
             the audio track could not be decoded",
            out.len()
        ));
    }

    Ok(Some(out))
}

/// Whether the source at `input_path` yields any audio at all, probed by cutting one second from its start (#6748).
///
/// Only consulted when a window came back empty, which is rare — once at the end of a walk — so the extra ffmpeg call costs a second on a path that was already stopping.
/// It buys the difference between "this window is past the end" and "this file's audio does not decode", which are byte-identical outcomes and mean opposite things to the caller.
async fn source_decodes(input_path: &str) -> Result<bool, String> {
    let probe = run_ffmpeg_pipe(
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            input_path,
            "-t",
            "1",
            "-vn",
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-ar",
            "48000",
            "-ac",
            "1",
            "-f",
            "ogg",
            "pipe:1",
        ],
        None,
        "check whether a recording's audio track decodes",
    )
    .await;
    // A probe that cannot even run says nothing about the source, so it must not be read as "the file is fine" — treat it as undecodable and let the caller report the original failure.
    Ok(probe
        .map(|bytes| ogg_contains_audio(&bytes))
        .unwrap_or(false))
}

/// Playable duration of an Ogg/Opus stream, read from the granule position of its final page.
///
/// Needed so a windowed transcription can report how much media it actually consumed, which is the only honest basis for "is there more after this" — the requested length is not, since a window that runs past the end of the recording still *asks* for its full span.
///
/// Read out of the container rather than probed with `ffprobe`: that binary is not called anywhere else in the kernel, so depending on it would add an external requirement to every deployment for one number that the bytes already carry.
/// An Opus stream's granule position counts samples at a fixed 48 kHz regardless of the encoder's own rate (RFC 7845 §4), and the last page's value is the end of the stream, so duration is that value less the pre-skip the header declares.
///
/// Returns `None` for input that is not a parseable Ogg/Opus stream, leaving the caller to fall back rather than fabricating a number.
///
/// Pages are walked by following each page's own length rather than by scanning for the next `OggS`.
/// A raw scan would accept a coincidental `4F 67 67 53` inside an entropy-coded Opus payload as a page header and read eight bytes of compressed audio as a granule position — producing a plausible-looking but wrong duration, which is exactly the "windows drift and audio is skipped" failure the produced-duration contract exists to prevent.
/// Walking the chain instead means a candidate is only accepted if the previous page's declared length lands on it, so payload bytes are never interpreted as a header.
fn ogg_opus_duration_secs(ogg: &[u8]) -> Option<f64> {
    const CAPTURE: &[u8; 4] = b"OggS";
    const OPUS_SAMPLE_RATE: f64 = 48_000.0;
    /// Bytes before the segment table: capture, version, flags, granule, serial, sequence, CRC, and the segment count itself.
    const HEADER_LEN: usize = 27;

    // Pre-skip lives at bytes 10..12 of the OpusHead packet, which is the first page's payload.
    // Absent it, a stream reports a few milliseconds more than it plays; that is small, but it accumulates across chunks.
    let pre_skip = ogg
        .windows(8)
        .position(|w| w == b"OpusHead")
        .and_then(|at| ogg.get(at + 10..at + 12))
        .map(|b| u16::from_le_bytes([b[0], b[1]]) as f64)
        .unwrap_or(0.0);

    // A well-formed stream starts on a page boundary; anything else is not one.
    if ogg.len() < HEADER_LEN || &ogg[..CAPTURE.len()] != CAPTURE {
        return None;
    }

    let mut last_granule: Option<u64> = None;
    let mut at = 0usize;
    loop {
        if ogg.len() < at + HEADER_LEN || &ogg[at..at + CAPTURE.len()] != CAPTURE {
            // The chain stopped landing on page boundaries: the stream is
            // truncated or malformed past this point.
            // Whatever was read up to here is still a real granule, so report that rather than discarding a usable answer.
            break;
        }
        // Granule position sits at bytes 6..14 of the header, little-endian.
        let Some(granule_bytes) = ogg.get(at + 6..at + 14) else {
            break;
        };
        let Ok(granule_bytes) = <[u8; 8]>::try_from(granule_bytes) else {
            break;
        };
        let granule = u64::from_le_bytes(granule_bytes);
        // Page length is the header, the segment table, and the sum of the
        // table's entries — following it is what keeps payload bytes from
        // being mistaken for the next header.
        let segments = ogg[at + 26] as usize;
        // A truncated tail must not discard what was already read — the guard above says so and this is the other half of it: a stream that stops inside the segment table or mid-payload leaves the granules of every complete page before it perfectly usable, and `?` here would throw them away along with the incomplete one.
        let Some(table) = ogg.get(at + HEADER_LEN..at + HEADER_LEN + segments) else {
            break;
        };
        let payload: usize = table.iter().map(|&n| n as usize).sum();
        let page_end = at + HEADER_LEN + segments + payload;
        // The granule is only meaningful once the page it describes has actually arrived: accepting it from a page whose payload was cut off would report audio the caller never received, and the caller would advance past it.
        if page_end > ogg.len() {
            break;
        }

        last_granule = Some(granule);
        at = page_end;
    }

    let granule = last_granule?;
    // `u64::MAX` is the "no packet finishes on this page" sentinel, not a length; a stream ending on one carries no usable duration.
    if granule == u64::MAX {
        return None;
    }
    Some(((granule as f64) - pre_skip).max(0.0) / OPUS_SAMPLE_RATE)
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
///
/// These hardcoded ids rot: a provider can retire a hosted model at any time
/// (e.g. Groq removed `meta-llama/llama-4-scout-17b-16e-instruct`), after which
/// every non-explicitly-configured setup silently degrades. There is no safe
/// way to keep this list evergreen in-tree; the mitigation is observability —
/// `record_media_understanding_failure` emits
/// `librefang_media_understanding_failures_total{kind,provider,model}` so the
/// rot surfaces as an actionable signal instead of a days-later user report
/// (#6538). Prefer setting `[media] image_model` explicitly in production.
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

    // #6538: a media-understanding failure must be observable via the
    // `librefang_media_understanding_failures_total` counter, labeled by
    // kind/provider/model, so operators can alert on a retired hosted model
    // instead of finding out days later. Mirrors the `DebuggingRecorder` +
    // `with_local_recorder` pattern in `librefang-runtime::command_lane`.
    #[test]
    fn media_understanding_failure_increments_counter_with_labels() {
        use metrics_util::debugging::{DebugValue, DebuggingRecorder};

        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            record_media_understanding_failure(
                "image",
                "groq",
                "meta-llama/llama-4-scout-17b-16e-instruct",
                "Vision API error (404): model_decommissioned",
            );
        });

        let snap = snapshotter.snapshot().into_vec();
        let hit = snap.iter().find(|(ckey, _, _, _)| {
            ckey.key().name() == "librefang_media_understanding_failures_total"
                && ckey
                    .key()
                    .labels()
                    .any(|l| l.key() == "kind" && l.value() == "image")
                && ckey
                    .key()
                    .labels()
                    .any(|l| l.key() == "provider" && l.value() == "groq")
                && ckey.key().labels().any(|l| {
                    l.key() == "model" && l.value() == "meta-llama/llama-4-scout-17b-16e-instruct"
                })
        });
        let (_, _, _, val) = hit.expect("media-understanding failure counter must be recorded");
        assert_eq!(*val, DebugValue::Counter(1), "counter must increment by 1");
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

    /// #6679: a synthetic mp4 (color bars + a tone, generated by ffmpeg
    /// itself via `-f lavfi` so the test needs no bundled fixture) must come
    /// out as a playable Ogg/Opus stream with the video stream dropped.
    #[tokio::test]
    async fn extract_video_audio_track_smoke() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        let gen = tokio::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x64:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440",
                "-t",
                "0.5",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-f",
                "mp4",
                "-movflags",
                "frag_keyframe+empty_moov",
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
        let mp4_bytes = gen.stdout;
        assert!(!mp4_bytes.is_empty());

        let out = extract_video_audio_track(&mp4_bytes)
            .await
            .expect("extraction must succeed on a valid mp4 with an audio track");
        assert!(!out.is_empty());
        assert_eq!(&out[..4], b"OggS", "output must be an Ogg container");

        // The extracted stream must be smaller than the source mp4 — proof
        // the video track (64x64 color bars) was actually dropped rather
        // than the whole container being passed through unchanged.
        assert!(
            out.len() < mp4_bytes.len(),
            "extracted audio-only stream ({} bytes) should be smaller than the \
             source mp4 with video ({} bytes)",
            out.len(),
            mp4_bytes.len()
        );
    }

    /// #6747: a plain, non-fragmented mp4 — `moov` written after `mdat`, which is what every phone camera, screen recorder and meeting exporter produces — must yield a stream with actual audio in it.
    ///
    /// The existing smoke test cannot catch this, for two independent reasons, and both had to be fixed here for the test to discriminate.
    ///
    /// Its fixture passes `-movflags frag_keyframe+empty_moov`, producing a *fragmented* mp4 — the one mp4 flavour that demuxes from a non-seekable pipe.
    /// It exercised precisely the shape that worked.
    ///
    /// It is also only ~8 KB, and that alone is disqualifying.
    /// ffmpeg buffers the head of an unseekable input (32 KB by default), so a file small enough to fit entirely in that buffer can still be "seeked" backwards and demuxes fine over a pipe.
    /// Measured against ffmpeg 8.1.1: a 0.5 s / 11 KB clip yields 2,500 bytes of Ogg and succeeds, while 5 s / 69 KB, 30 s / 389 KB and 120 s / 1.5 MB all yield exactly 261 bytes with `stream 0, offset 0x30: partial file` on stderr.
    /// That is why the fixture below is 10 s rather than the smoke test's 0.5 s — a shorter one passes with or without the fix.
    ///
    /// The assertions are the ones the old code failed: a header-only Ogg is non-empty, starts with `OggS`, and is smaller than the source mp4, so every assertion in the smoke test passed on a stream carrying no audio.
    #[tokio::test]
    async fn extract_video_audio_track_handles_non_fragmented_mp4_6747() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }

        // Written to a file rather than `pipe:1`: a non-fragmented mp4 cannot be *muxed* to a pipe either, since the muxer rewinds to write moov.
        // Generating the fixture the same way the failing input arises is the point of the test.
        // `TempDir` cleans up on drop even if an `assert!` below panics — a hand-rolled path plus `remove_dir_all` after the assertions would leak the directory on that path.
        let dir = tempfile::Builder::new()
            .prefix("librefang-6747-")
            .tempdir()
            .expect("temp dir");
        let fixture = dir.path().join("plain.mp4");

        let gen = tokio::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=15",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440",
                // 10 s at this size lands around 130 KB, comfortably past the 32 KB read-ahead buffer that lets a small unseekable input demux anyway.
                // See the doc comment for the measured cutoff.
                "-t",
                "10",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
            ])
            .arg(&fixture)
            .output()
            .await
            .expect("ffmpeg must run");
        assert!(
            gen.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&gen.stderr)
        );

        let mp4_bytes = std::fs::read(&fixture).expect("fixture readable");
        assert!(!mp4_bytes.is_empty());

        let out = extract_video_audio_track(&mp4_bytes)
            .await
            .expect("a plain non-fragmented mp4 must extract");

        assert_eq!(&out[..4], b"OggS", "output must be an Ogg container");
        assert!(
            ogg_contains_audio(&out),
            "extraction produced {} bytes but no audio packets — the demux \
             failed silently, which is exactly #6747",
            out.len()
        );
    }

    /// The audio guard must reject a headers-only stream and accept a real one.
    ///
    /// Pinned directly so the #6747 regression above cannot be satisfied by a guard that returns `true` unconditionally.
    #[test]
    fn ogg_contains_audio_distinguishes_headers_from_audio() {
        // Two pages: OpusHead + OpusTags, no audio.
        // This is the shape of the 261-byte artefact a failed demux emitted.
        let headers_only = b"OggS____OpusHead____OggS____OpusTags____".to_vec();
        assert!(!ogg_contains_audio(&headers_only));

        let mut with_audio = headers_only.clone();
        with_audio.extend_from_slice(b"OggS____audio payload");
        assert!(ogg_contains_audio(&with_audio));

        assert!(!ogg_contains_audio(&[]), "empty input carries no audio");
    }

    #[tokio::test]
    async fn extract_video_audio_track_rejects_non_video_input() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        let garbage: Vec<u8> = (0..=255u8).collect();
        let err = extract_video_audio_track(&garbage).await.unwrap_err();
        assert!(
            err.contains("ffmpeg exited") || err.contains("empty output"),
            "expected ffmpeg to reject non-video junk, got: {err}"
        );
    }

    // ── whisper_transcribe language / prompt (#6678) ─────────────────────
    //
    // No HTTP mock crate is a dependency of this crate, so these spin up a
    // raw `TcpListener`, read the request `whisper_transcribe` actually
    // sends, and answer with a minimal valid response — same shape as the
    // raw-socket pattern already used in `librefang-runtime::a2a` tests.

    /// Read one HTTP/1.1 request off `stream`: headers, then exactly
    /// `Content-Length` more bytes for the body. Good enough for a
    /// single-shot local test server; not a general HTTP parser.
    async fn read_one_http_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt as _};
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let n = stream.read(&mut chunk).await.expect("read request chunk");
            assert!(n > 0, "connection closed before headers completed");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let header_text = String::from_utf8_lossy(&buf[..header_end]);
        let content_length: usize = header_text
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().to_string())
            })
            .and_then(|v| v.parse().ok())
            .expect("request must carry Content-Length");
        while buf.len() < header_end + content_length {
            let n = stream.read(&mut chunk).await.expect("read request body");
            assert!(n > 0, "connection closed before body completed");
            buf.extend_from_slice(&chunk[..n]);
        }
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
            .await;
        let _ = stream.shutdown().await;
        buf
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    #[tokio::test]
    async fn whisper_transcribe_sends_language_and_prompt_when_set() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_one_http_request(&mut stream).await
        });

        let url = format!("http://{addr}/v1/audio/transcriptions");
        let result = whisper_transcribe(WhisperTranscribeParams {
            api_url: &url,
            api_key: "test-key",
            model: "whisper-1",
            audio_bytes: b"fake audio bytes".to_vec(),
            filename: "audio.mp3",
            mime: "audio/mpeg",
            language: Some("en"),
            prompt: Some("proper nouns: LibreFang, Whisper"),
        })
        .await;

        let captured = server.await.unwrap();
        let body = String::from_utf8_lossy(&captured);

        assert_eq!(result.as_deref(), Ok("hello"));
        assert!(
            body.contains("name=\"language\"") && body.contains("\r\n\r\nen\r\n"),
            "request body must carry the language field:\n{body}"
        );
        assert!(
            body.contains("name=\"prompt\"") && body.contains("proper nouns: LibreFang, Whisper"),
            "request body must carry the prompt field:\n{body}"
        );
    }

    /// The doc comment on `whisper_transcribe` promises a byte-identical
    /// request when neither parameter is set — assert the request the
    /// server actually received omits both field names entirely, not just
    /// that the call succeeds.
    #[tokio::test]
    async fn whisper_transcribe_omits_language_and_prompt_when_unset() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_one_http_request(&mut stream).await
        });

        let url = format!("http://{addr}/v1/audio/transcriptions");
        let result = whisper_transcribe(WhisperTranscribeParams {
            api_url: &url,
            api_key: "test-key",
            model: "whisper-1",
            audio_bytes: b"fake audio bytes".to_vec(),
            filename: "audio.mp3",
            mime: "audio/mpeg",
            language: None,
            prompt: None,
        })
        .await;

        let captured = server.await.unwrap();
        let body = String::from_utf8_lossy(&captured);

        assert_eq!(result.as_deref(), Ok("hello"));
        assert!(
            !body.contains("name=\"language\"") && !body.contains("name=\"prompt\""),
            "request body must not carry either field when both are unset:\n{body}"
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

    /// #6679: `transcribe_audio` must accept `MediaType::Video` (it used to reject every non-`Audio` type outright) — asserted by checking the call gets PAST the type guard, which needs no ffmpeg and reaches no network.
    ///
    /// Which error comes back past the guard depends on the developer's environment, so both are accepted.
    /// `transcribe_audio` resolves the provider (`config.audio_provider`, else `detect_audio_provider()`, which reads `OPENAI_API_KEY` / `GROQ_API_KEY` / … from the process) BEFORE it reads the file.
    /// On a clean machine and in CI that resolution fails and the call stops there; with any STT key exported it succeeds and the call proceeds to the file read.
    /// Pinning only the provider message made this test fail on any machine with a key configured, which is a statement about the shell rather than about the type guard.
    ///
    /// The path must not exist for a second reason: it is what keeps the test off the network.
    /// Provider resolution succeeding is not hypothetical, and if the read then succeeded the next step would be a real, billable STT call.
    /// A guaranteed-absent path makes that unreachable.
    /// Constructed rather than borrowed from the host filesystem — see #5716.
    #[tokio::test]
    async fn transcribe_audio_accepts_video_type() {
        let engine = MediaEngine::new(MediaConfig::default());
        let missing = std::env::temp_dir()
            .join("librefang-nonexistent-6679")
            .join("test.mp4");
        let attachment = MediaAttachment {
            media_type: MediaType::Video,
            mime_type: "video/mp4".into(),
            source: MediaSource::FilePath {
                path: missing.to_string_lossy().into_owned(),
            },
            size_bytes: 1024,
        };
        let err = engine
            .transcribe_audio(&attachment, None, None)
            .await
            .unwrap_err();
        assert!(
            !err.contains("Expected audio"),
            "Video must pass the type guard, got: {err}"
        );
        assert!(
            err.contains("No audio transcription provider configured")
                || err.contains("Failed to read video file"),
            "expected provider resolution or the file read to be what fails, \
             i.e. something strictly past the type guard, got: {err}"
        );
    }

    /// The type guard must still reject an unrelated type — this is not the
    /// same coverage as `test_describe_image_wrong_type` above, which
    /// exercises a *different* method (`describe_image`).
    #[tokio::test]
    async fn transcribe_audio_still_rejects_image_type() {
        let engine = MediaEngine::new(MediaConfig::default());
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".into(),
            source: MediaSource::FilePath {
                path: "test.png".into(),
            },
            size_bytes: 1024,
        };
        let err = engine
            .transcribe_audio(&attachment, None, None)
            .await
            .unwrap_err();
        assert!(err.contains("Expected audio or video"), "got: {err}");
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
        let result = engine.transcribe_audio(&attachment, None, None).await;
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

    /// Same latent dependency as `test_transcribe_audio_no_provider` above: `describe_image` resolves the provider (`config.image_provider`, else `detect_vision_provider()`, which reads vision-capable keys from the process) BEFORE it reads the file.
    /// With no explicit `image_provider` set, a machine with a vision-capable key exported resolves a real provider here; only a guaranteed-absent input path keeps the call from proceeding to a real, billable description request.
    #[tokio::test]
    async fn test_describe_image_no_provider_configured() {
        // With no API keys set, should fail with provider error.
        // With a key set, provider resolution succeeds instead and the call must still stop at the guaranteed-absent file read, never reaching a real description request.
        let engine = MediaEngine::new(MediaConfig::default());
        let missing = std::env::temp_dir()
            .join("librefang-nonexistent-describe-no-provider")
            .join("test.png");
        let attachment = MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/png".into(),
            source: MediaSource::FilePath {
                path: missing.to_string_lossy().into_owned(),
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
        let result = engine.transcribe_audio(&attachment, None, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Expected audio"));
    }

    /// Same latent dependency `transcribe_audio_accepts_video_type` above was fixed for: `transcribe_audio` resolves the provider (`config.audio_provider`, else `detect_audio_provider()`, which reads STT keys from the process) BEFORE it reads the file.
    /// With no explicit `audio_provider` set, a machine with an STT key exported resolves a real provider here; the only thing that then kept this test off the network was the input path happening not to exist at the process's working directory.
    /// A guaranteed-absent path removes that assumption instead of relying on it — see #5716 on not depending on incidental filesystem state in tests.
    #[tokio::test]
    async fn test_transcribe_audio_no_provider() {
        // With no API keys set, should fail with provider error.
        // With a key set, provider resolution succeeds instead and the call must still stop at the guaranteed-absent file read, never reaching a real transcription request.
        let engine = MediaEngine::new(MediaConfig::default());
        let missing = std::env::temp_dir()
            .join("librefang-nonexistent-transcribe-no-provider")
            .join("test.webm");
        let attachment = MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: "audio/webm".into(),
            source: MediaSource::FilePath {
                path: missing.to_string_lossy().into_owned(),
            },
            size_bytes: 1024,
        };
        let result = engine.transcribe_audio(&attachment, None, None).await;
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
        let result = engine.transcribe_audio(&attachment, None, None).await;
        assert!(result.is_err());
        // Wording generalized from "URL-based audio source" to "URL-based
        // source" when this code path started accepting Video attachments
        // too (#6679) — the same rejection now applies to both.
        assert!(result
            .unwrap_err()
            .contains("URL-based source not supported"));
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
        let result = engine.transcribe_audio(&attachment, None, None).await;
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

    // ── windowed transcription (#6748) ──────────────────────────────────
    //
    // These pin the two facts a caller walks a long recording on: the window ffmpeg cut, and how much of the recording it actually covered.
    // Both are read back out of the produced bytes rather than assumed, because a seek lands on a keyframe and a window past the end of the media is short.

    /// Generate `secs` seconds of tone as an Ogg/Opus stream at the same 48 kHz mono target the extraction paths produce.
    async fn synth_ogg_opus(secs: f64) -> Vec<u8> {
        let gen = tokio::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440",
                "-t",
                &format!("{secs}"),
                "-c:a",
                "libopus",
                "-b:a",
                "32k",
                "-ar",
                "48000",
                "-ac",
                "1",
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
        gen.stdout
    }

    #[tokio::test]
    async fn ogg_opus_duration_reads_the_final_granule_position() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        for expected in [1.0_f64, 7.5] {
            let ogg = synth_ogg_opus(expected).await;
            let got = ogg_opus_duration_secs(&ogg)
                .unwrap_or_else(|| panic!("duration must parse for a {expected}s stream"));
            assert!(
                // Tight on purpose: granule arithmetic is exact, and the pre-skip
                // subtraction this pins is worth only ~0.0065 s.
                // A 0.05 s bar is seven times wider than the effect, so dropping
                // the subtraction passed — the assertion has to be narrower than
                // the thing it guards.
                (got - expected).abs() < 0.002,
                "expected ~{expected}s, got {got}s"
            );
        }
    }

    /// A page payload that happens to contain the bytes `OggS` must not be read as a page header.
    /// Opus payloads are entropy-coded, so the sequence occurs by chance; taking eight bytes of compressed audio as a granule position would yield a plausible but wrong duration, and the caller advances by that number.
    /// Walking the page chain by its declared lengths is what prevents it — a raw scan for the magic cannot.
    #[test]
    fn ogg_opus_duration_ignores_page_magic_inside_a_payload() {
        /// One Ogg page: 27-byte header, a one-entry segment table, then the payload.
        fn page(granule: u64, payload: &[u8]) -> Vec<u8> {
            let mut p = Vec::new();
            p.extend_from_slice(b"OggS");
            p.push(0); // version
            p.push(0); // header type
            p.extend_from_slice(&granule.to_le_bytes());
            p.extend_from_slice(&[0u8; 4]); // serial
            p.extend_from_slice(&[0u8; 4]); // sequence
            p.extend_from_slice(&[0u8; 4]); // crc
            p.push(1); // one segment
            p.push(payload.len() as u8);
            p.extend_from_slice(payload);
            p
        }

        // The decoy is `OggS` followed by bytes that read as a granule of one
        // hour if this position is mistaken for a page header, plus enough
        // trailing bytes that a naive reader finds a whole header's worth of
        // data here rather than stopping short of the end.
        let mut decoy = b"OggS".to_vec();
        decoy.extend_from_slice(&[0, 0]);
        decoy.extend_from_slice(&(48_000u64 * 3600).to_le_bytes());
        decoy.extend_from_slice(&[0u8; 40]);

        let mut stream = page(
            0,
            b"OpusHead\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
        );
        stream.extend(page(48_000, &decoy));

        let secs = ogg_opus_duration_secs(&stream)
            .expect("a well-formed chain must parse even with a decoy in the payload");
        assert!(
            (secs - 1.0).abs() < 0.01,
            "must report the real final granule (1s), not the payload's bytes, got {secs}s"
        );
    }

    /// Guards the fallback contract: input that is not an Ogg stream yields `None` rather than a fabricated number the caller would then advance by.
    #[test]
    fn ogg_opus_duration_rejects_non_ogg_input() {
        assert_eq!(ogg_opus_duration_secs(&[0u8; 64]), None);
        assert_eq!(ogg_opus_duration_secs(b""), None);
    }

    /// A stream that stops inside a page must still report the granules of the complete pages before it — the walk's own comment promises exactly that, and an early `?` used to discard them, stopping a caller's walk short of the recording's end.
    #[test]
    fn ogg_opus_duration_keeps_what_it_read_when_the_tail_is_truncated() {
        fn page(granule: u64, payload: &[u8]) -> Vec<u8> {
            let mut p = Vec::new();
            p.extend_from_slice(b"OggS");
            p.push(0);
            p.push(0);
            p.extend_from_slice(&granule.to_le_bytes());
            p.extend_from_slice(&[0u8; 4]);
            p.extend_from_slice(&[0u8; 4]);
            p.extend_from_slice(&[0u8; 4]);
            p.push(1);
            p.push(payload.len() as u8);
            p.extend_from_slice(payload);
            p
        }

        let mut whole = page(
            0,
            b"OpusHead\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
        );
        whole.extend(page(48_000, &[7u8; 32]));
        let complete_len = whole.len();
        whole.extend(page(96_000, &[7u8; 32]));

        // Cut inside the third page's header, its segment table, and its payload in turn.
        // Every one of those leaves the 1 s of complete pages before it intact, so every one must report 1 s rather than nothing.
        for cut in [complete_len + 4, complete_len + 27, complete_len + 30] {
            let truncated = &whole[..cut];
            assert_eq!(
                ogg_opus_duration_secs(truncated),
                Some(1.0),
                "a tail cut at byte {cut} must still report the last complete page"
            );
        }

        // And the granule of a page whose payload never arrived must not be accepted: reporting 2 s here would advance the caller past audio it never received.
        let mid_payload = &whole[..whole.len() - 8];
        assert_eq!(ogg_opus_duration_secs(mid_payload), Some(1.0));
    }

    #[tokio::test]
    async fn extract_media_window_cuts_the_requested_span() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        let source = synth_ogg_opus(12.0).await;

        let cut = extract_media_window(
            WindowSource::Bytes(&source),
            MediaWindow {
                start_sec: 2.0,
                max_secs: 5.0,
            },
        )
        .await
        .expect("window extraction must succeed")
        .expect("a window inside the recording must carry audio");
        assert_eq!(&cut[..4], b"OggS", "output must be an Ogg container");
        let got = ogg_opus_duration_secs(&cut).expect("window duration must parse");
        assert!(
            (got - 5.0).abs() < 0.25,
            "a fully-covered window must be about as long as requested, got {got}s"
        );
    }

    /// The branch the walk's termination rests on: a window that begins **past** the end of the recording, rather than merely overlapping it.
    /// ffmpeg emits headers with no audio there, which is byte-identical to a failed demux, so the two are told apart by where the window started — and this shape must end the walk rather than fail it.
    #[tokio::test]
    async fn extract_media_window_fully_past_the_end_reports_no_window() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        let source = synth_ogg_opus(1.0).await;

        for start_sec in [1.2_f64, 1.5, 2.0, 5.0] {
            let out = extract_media_window(
                WindowSource::Bytes(&source),
                MediaWindow {
                    start_sec,
                    max_secs: 600.0,
                },
            )
            .await
            .expect("an overshoot must not be an error");
            assert!(
                out.is_none(),
                "a window starting at {start_sec}s of a 1s recording carries no audio"
            );
        }
    }

    /// A source whose audio will not decode must fail loudly even when the window starts past the opening, rather than being reported as a successful empty window — the caller would otherwise conclude the recording holds nothing there and move on, which is the silent failure #6747 exists to prevent.
    ///
    /// ⚠️ This covers the case ffmpeg rejects outright, which is the reachable one: garbage input exits non-zero and `run_ffmpeg_pipe` turns that into an error before the guard is consulted.
    /// It does **not** exercise the `source_decodes` probe, and neutralising that probe leaves this test green.
    /// The shape the probe exists for — ffmpeg exiting 0 while emitting a headers-only container from an input it accepted — could not be synthesised here: with file input every undecodable source tried exits non-zero.
    /// That shape was real over a pipe (#6747), so the probe is kept as defence rather than removed for want of a fixture, but it is deliberately not claimed to be pinned.
    #[tokio::test]
    async fn extract_media_window_reports_an_undecodable_source_at_any_offset() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        let garbage: Vec<u8> = (0..=255u8).cycle().take(4096).collect();

        for start_sec in [0.0_f64, 600.0] {
            let err = extract_media_window(
                WindowSource::Bytes(&garbage),
                MediaWindow {
                    start_sec,
                    max_secs: 600.0,
                },
            )
            .await
            .err();
            assert!(
                err.is_some(),
                "an undecodable source must be an error at start_sec={start_sec}, not an empty window"
            );
        }
    }

    /// Records a limitation this guard does **not** cover, so it is not mistaken for one that does.
    ///
    /// ffmpeg abandons the seek once `-ss` is past the end by more than a fixed margin and returns the recording from the beginning instead of nothing.
    ///
    /// The margin is **absolute, not proportional**: measured on ffmpeg 8.1 across 10 s, 60 s and 300 s sources, every one of them yields the headers-only stream this guard rejects up to about 8 s past the end and the 5524-byte opening of the recording from about 10 s past it.
    /// An earlier note here put the safe range at "five times the recording", which was an artefact of measuring a 1 s source — five seconds of slack looked proportional and is not.
    /// On a 45-minute recording the margin is 0.4 % of its length.
    ///
    /// A walk never asks for that: `next_start_sec` advances by the produced duration, so it overshoots by a fraction of a second and lands well inside the safe range.
    /// It is reachable when a caller sets `start_sec` itself and is off by more than ten seconds — "from the fiftieth minute" of a recording that runs forty-five.
    /// The cost is a billed request for audio already transcribed, plus the opening of the recording appended to `out_path` as though it were the window that was asked for, with `has_more: false` reporting success.
    ///
    /// Not detectable from the output: the clamped stream is ordinary audio, and comparing it against the recording's opening does not work either, since Ogg page headers carry per-encode serials and CRCs (measured: 3 % byte agreement, the same as for a legitimate tail).
    /// Knowing the source duration would settle it and there is no way to learn it here without `ffprobe`, which the kernel does not call.
    /// So the constraint is stated where the caller can act on it — the tool schema and both documentation pages tell it to advance with `next_start_sec` rather than guess an offset.
    ///
    /// Left as a known limitation rather than papered over: the obvious remedy, `-copyts`, makes the granule absolute (a tail window at `-ss 8` of a 10 s source reports 10.007 s instead of 2.006 s) and changes what `-t` means, which would break the produced-duration contract the whole walk rests on.
    #[tokio::test]
    async fn extract_media_window_far_past_the_end_is_not_detected() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        let source = synth_ogg_opus(1.0).await;

        let out = extract_media_window(
            WindowSource::Bytes(&source),
            MediaWindow {
                start_sec: 60.0,
                max_secs: 600.0,
            },
        )
        .await
        .expect("extraction itself succeeds");
        // Asserts only that this does not error, which holds both today and after any future fix.
        // Pinning `is_some()` would make the fix look like a regression to whoever writes it.
        let _ = out;
    }

    /// The same window cut straight from a file on disk, which is the path a tool call takes: no read, no base64, no staging copy per window.
    #[tokio::test]
    async fn extract_media_window_accepts_a_path_without_staging() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        let bytes = synth_ogg_opus(6.0).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("source.ogg");
        tokio::fs::write(&path, &bytes).await.expect("write source");

        let from_path = extract_media_window(
            WindowSource::Path(&path),
            MediaWindow {
                start_sec: 1.0,
                max_secs: 3.0,
            },
        )
        .await
        .expect("path input must work")
        .expect("a window inside the recording carries audio");
        let from_bytes = extract_media_window(
            WindowSource::Bytes(&bytes),
            MediaWindow {
                start_sec: 1.0,
                max_secs: 3.0,
            },
        )
        .await
        .expect("byte input must work")
        .expect("a window inside the recording carries audio");

        let a = ogg_opus_duration_secs(&from_path).expect("duration");
        let b = ogg_opus_duration_secs(&from_bytes).expect("duration");
        assert!(
            (a - b).abs() < 0.05,
            "path and byte inputs must produce the same window, got {a}s vs {b}s"
        );
    }

    /// The end-of-recording signal: a window that runs past the end comes back short, and that shortfall — not any assumption about the request — is what tells the caller to stop.
    #[tokio::test]
    async fn extract_media_window_past_the_end_returns_a_short_window() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not on PATH — skipping");
            return;
        }
        let source = synth_ogg_opus(6.0).await;

        let cut = extract_media_window(
            WindowSource::Bytes(&source),
            MediaWindow {
                start_sec: 4.0,
                max_secs: 600.0,
            },
        )
        .await
        .expect("a window overlapping the end must still produce audio")
        .expect("a window that overlaps the end still carries its tail");
        let got = ogg_opus_duration_secs(&cut).expect("window duration must parse");
        assert!(
            got < 600.0 && (got - 2.0).abs() < 0.25,
            "expected the ~2s tail rather than the requested 600s, got {got}s"
        );
    }
}
