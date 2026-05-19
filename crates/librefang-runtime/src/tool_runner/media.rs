//! Media understanding & generation tools — vision describe / audio
//! transcribe, image / video / music generation, text-to-speech /
//! speech-to-text.

use super::resolve_file_path_ext;
use std::path::Path;
use tracing::warn;

/// Describe an image using a vision-capable LLM provider.
pub(super) async fn tool_media_describe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    // Route through the workspace sandbox so all media reads stay inside
    // the agent's dir — a plain `..` check would miss absolute paths like
    // `/etc/passwd`. Named workspace prefixes are honored via
    // `additional_roots` so agents can describe media that lives under
    // declared `[workspaces]` mounts.
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

    // Read image file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read image file: {e}"))?;

    // Detect MIME type from extension
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => return Err(format!("Unsupported image format: .{ext}")),
    };

    let attachment = librefang_types::media::MediaAttachment {
        media_type: librefang_types::media::MediaType::Image,
        mime_type: mime.to_string(),
        source: librefang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.describe_image(&attachment).await?;
    serde_json::to_string_pretty(&understanding).map_err(|e| format!("Serialize error: {e}"))
}

/// Human-readable list of audio extensions accepted by `audio_mime_from_ext`,
/// surfaced in `media_transcribe` / `speech_to_text` tool schema descriptions
/// so the agent-facing format list cannot drift from the actual mapping.
pub(super) const SUPPORTED_AUDIO_EXTS_DOC: &str = "mp3, wav, ogg, oga, flac, m4a, webm";

/// Map an audio file extension to the MIME type expected by
/// `MediaEngine::transcribe_audio`. `.oga` is intentionally mapped to
/// `audio/oga` (NOT `audio/ogg`) so the downstream transcode path in
/// `media_understanding::transcribe_audio` re-muxes the container before
/// the Whisper upload — Telegram voice notes are byte-identical Ogg/Opus
/// under the `.oga` extension, but Whisper's format probe rejects them.
pub(super) fn audio_mime_from_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        "ogg" => Some("audio/ogg"),
        "oga" => Some("audio/oga"),
        "flac" => Some("audio/flac"),
        "m4a" => Some("audio/mp4"),
        "webm" => Some("audio/webm"),
        _ => None,
    }
}

/// Transcribe audio to text using speech-to-text.
pub(super) async fn tool_media_transcribe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    // Route through the workspace sandbox so all media reads stay inside
    // the agent's dir — a plain `..` check would miss absolute paths like
    // `/etc/passwd`. Named workspace prefixes are honored via
    // `additional_roots` so agents can transcribe audio under declared
    // `[workspaces]` mounts.
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

    // Read audio file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    // Detect MIME type from extension
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime =
        audio_mime_from_ext(&ext).ok_or_else(|| format!("Unsupported audio format: .{ext}"))?;

    let attachment = librefang_types::media::MediaAttachment {
        media_type: librefang_types::media::MediaType::Audio,
        mime_type: mime.to_string(),
        source: librefang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;
    serde_json::to_string_pretty(&understanding).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Image generation tool
// ---------------------------------------------------------------------------

/// Generate images from a text prompt.
pub(super) async fn tool_image_generate(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    workspace_root: Option<&Path>,
    upload_dir: &Path,
) -> Result<String, String> {
    let prompt = input["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?;

    let provider = input["provider"].as_str().map(|s| s.to_string());
    let model = input["model"].as_str().map(|s| s.to_string());
    let aspect_ratio = input["aspect_ratio"].as_str().map(|s| s.to_string());
    let width = input["width"].as_u64().map(|v| v as u32);
    let height = input["height"].as_u64().map(|v| v as u32);
    let quality = input["quality"].as_str().map(|s| s.to_string());
    let count = input["count"].as_u64().unwrap_or(1).min(9) as u8;

    // Use MediaDriverCache if available (multi-provider), fall back to old OpenAI-only path.
    if let Some(cache) = media_drivers {
        let request = librefang_types::media::MediaImageRequest {
            prompt: prompt.to_string(),
            provider: provider.clone(),
            model,
            width,
            height,
            aspect_ratio,
            quality,
            count,
            seed: None,
        };

        request.validate().map_err(|e| e.to_string())?;

        let driver = if let Some(ref name) = provider {
            cache.get_or_create(name, None)
        } else {
            cache.detect_for_capability(librefang_types::media::MediaCapability::ImageGeneration)
        }
        .map_err(|e| e.to_string())?;

        let result = driver
            .generate_image(&request)
            .await
            .map_err(|e| e.to_string())?;

        // Save images to workspace and uploads dir
        let saved_paths = save_media_images_to_workspace(&result.images, workspace_root);
        let image_urls = save_media_images_to_uploads(&result.images, upload_dir);

        let response = serde_json::json!({
            "model": result.model,
            "provider": result.provider,
            "images_generated": result.images.len(),
            "saved_to": saved_paths,
            "revised_prompt": result.revised_prompt,
            "image_urls": image_urls,
        });

        return serde_json::to_string_pretty(&response)
            .map_err(|e| format!("Serialize error: {e}"));
    }

    // Fallback: old OpenAI-only path (when media_drivers is None)
    let model_str = input["model"].as_str().unwrap_or("dall-e-3");
    let model = match model_str {
        "dall-e-3" | "dalle3" | "dalle-3" => librefang_types::media::ImageGenModel::DallE3,
        "dall-e-2" | "dalle2" | "dalle-2" => librefang_types::media::ImageGenModel::DallE2,
        "gpt-image-1" | "gpt_image_1" => librefang_types::media::ImageGenModel::GptImage1,
        _ => {
            return Err(format!(
                "Unknown image model: {model_str}. Use 'dall-e-3', 'dall-e-2', or 'gpt-image-1'."
            ))
        }
    };

    let size = input["size"].as_str().unwrap_or("1024x1024").to_string();
    let quality_str = input["quality"].as_str().unwrap_or("hd").to_string();
    let count_val = input["count"].as_u64().unwrap_or(1).min(4) as u8;

    let request = librefang_types::media::ImageGenRequest {
        prompt: prompt.to_string(),
        model,
        size,
        quality: quality_str,
        count: count_val,
    };

    let result = crate::image_gen::generate_image(&request).await?;

    let saved_paths = if let Some(workspace) = workspace_root {
        match crate::image_gen::save_images_to_workspace(&result, workspace) {
            Ok(paths) => paths,
            Err(e) => {
                warn!("Failed to save images to workspace: {e}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    let mut image_urls: Vec<String> = Vec::new();
    {
        use base64::Engine;
        let _ = std::fs::create_dir_all(upload_dir);
        for img in &result.images {
            let file_id = uuid::Uuid::new_v4().to_string();
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64)
            {
                let path = upload_dir.join(&file_id);
                if std::fs::write(&path, &decoded).is_ok() {
                    image_urls.push(format!("/api/uploads/{file_id}"));
                }
            }
        }
    }

    let response = serde_json::json!({
        "model": result.model,
        "images_generated": result.images.len(),
        "saved_to": saved_paths,
        "revised_prompt": result.revised_prompt,
        "image_urls": image_urls,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

/// Save MediaImageResult images to workspace output/ dir.
fn save_media_images_to_workspace(
    images: &[librefang_types::media::GeneratedImage],
    workspace_root: Option<&Path>,
) -> Vec<String> {
    let Some(workspace) = workspace_root else {
        return Vec::new();
    };
    use base64::Engine;
    let output_dir = workspace.join("output");
    let _ = std::fs::create_dir_all(&output_dir);
    let mut paths = Vec::new();
    for (i, img) in images.iter().enumerate() {
        if img.data_base64.is_empty() {
            continue;
        }
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64) {
            let filename = format!("image_{}.png", i);
            let path = output_dir.join(&filename);
            if std::fs::write(&path, &decoded).is_ok() {
                paths.push(path.display().to_string());
            }
        }
    }
    paths
}

/// Save MediaImageResult images to uploads temp dir, returning /api/uploads/... URLs.
fn save_media_images_to_uploads(
    images: &[librefang_types::media::GeneratedImage],
    upload_dir: &Path,
) -> Vec<String> {
    use base64::Engine;
    let _ = std::fs::create_dir_all(upload_dir);
    let mut urls = Vec::new();
    for img in images {
        // If provider returned a URL directly, use it as-is
        if img.data_base64.is_empty() {
            if let Some(ref url) = img.url {
                urls.push(url.clone());
            }
            continue;
        }
        let file_id = uuid::Uuid::new_v4().to_string();
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64) {
            if !decoded.is_empty() {
                let path = upload_dir.join(&file_id);
                if std::fs::write(&path, &decoded).is_ok() {
                    urls.push(format!("/api/uploads/{file_id}"));
                }
            }
        }
    }
    urls
}

// ---------------------------------------------------------------------------
// Video / Music generation tools (MediaDriver-based)
// ---------------------------------------------------------------------------

/// Generate a video from a text prompt. Returns a task_id for async polling.
pub(super) async fn tool_video_generate(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
) -> Result<String, String> {
    let cache =
        media_drivers.ok_or("Media drivers not available. Ensure media drivers are configured.")?;
    let prompt = input["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?;

    let request = librefang_types::media::MediaVideoRequest {
        prompt: prompt.to_string(),
        provider: input["provider"].as_str().map(String::from),
        model: input["model"].as_str().map(String::from),
        duration_secs: input["duration"].as_u64().map(|v| v as u32),
        resolution: input["resolution"].as_str().map(String::from),
        image_url: None,
        optimize_prompt: None,
    };

    // Validate request parameters before sending to the provider
    request
        .validate()
        .map_err(|e| format!("Invalid request: {e}"))?;

    let driver = if let Some(p) = &request.provider {
        cache.get_or_create(p, None).map_err(|e| e.to_string())?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::VideoGeneration)
            .map_err(|e| e.to_string())?
    };

    let result = driver
        .submit_video(&request)
        .await
        .map_err(|e| e.to_string())?;

    let response = serde_json::json!({
        "task_id": result.task_id,
        "provider": result.provider,
        "status": "submitted",
        "note": "Use video_status tool with this task_id to check progress"
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

/// Check the status of a video generation task. Returns download URL when complete.
pub(super) async fn tool_video_status(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
) -> Result<String, String> {
    let cache =
        media_drivers.ok_or("Media drivers not available. Ensure media drivers are configured.")?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    let provider = input["provider"].as_str();

    let driver = if let Some(p) = provider {
        cache.get_or_create(p, None).map_err(|e| e.to_string())?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::VideoGeneration)
            .map_err(|e| e.to_string())?
    };

    let status = driver
        .poll_video(task_id)
        .await
        .map_err(|e| e.to_string())?;

    // If completed, also fetch the full result with download URL
    if status == librefang_types::media::MediaTaskStatus::Completed {
        let video_result = driver
            .get_video_result(task_id)
            .await
            .map_err(|e| e.to_string())?;
        let response = serde_json::json!({
            "status": "completed",
            "file_url": video_result.file_url,
            "width": video_result.width,
            "height": video_result.height,
            "duration_secs": video_result.duration_secs,
            "provider": video_result.provider,
            "model": video_result.model,
        });
        return serde_json::to_string_pretty(&response)
            .map_err(|e| format!("Serialize error: {e}"));
    }

    let response = serde_json::json!({
        "status": status.to_string(),
        "task_id": task_id,
        "note": "Task is still in progress. Poll again after a few seconds."
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

/// Generate music from a prompt and/or lyrics. Saves audio to workspace output/ directory.
pub(super) async fn tool_music_generate(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let cache =
        media_drivers.ok_or("Media drivers not available. Ensure media drivers are configured.")?;

    let request = librefang_types::media::MediaMusicRequest {
        prompt: input["prompt"].as_str().map(String::from),
        lyrics: input["lyrics"].as_str().map(String::from),
        provider: input["provider"].as_str().map(String::from),
        model: input["model"].as_str().map(String::from),
        instrumental: input["instrumental"].as_bool().unwrap_or(false),
        format: None,
    };

    // Validate request parameters before sending to the provider
    request
        .validate()
        .map_err(|e| format!("Invalid request: {e}"))?;

    let driver = if let Some(p) = &request.provider {
        cache.get_or_create(p, None).map_err(|e| e.to_string())?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::MusicGeneration)
            .map_err(|e| e.to_string())?
    };

    let result = driver
        .generate_music(&request)
        .await
        .map_err(|e| e.to_string())?;

    // Save audio to workspace output/ directory (same pattern as text_to_speech)
    let saved_path = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| format!("Failed to create output dir: {e}"))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("music_{timestamp}.{}", result.format);
        let path = output_dir.join(&filename);

        tokio::fs::write(&path, &result.audio_data)
            .await
            .map_err(|e| format!("Failed to write audio file: {e}"))?;

        Some(path.display().to_string())
    } else {
        None
    };

    let mut response = serde_json::json!({
        "saved_to": saved_path,
        "format": result.format,
        "provider": result.provider,
        "model": result.model,
        "duration_ms": result.duration_ms,
        "size_bytes": result.audio_data.len(),
    });

    // When no workspace is available (e.g. MCP context), include base64-encoded
    // audio so the caller can still retrieve the generated content.
    if saved_path.is_none() && !result.audio_data.is_empty() {
        use base64::Engine;
        response["audio_base64"] =
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode(&result.audio_data));
    }

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// TTS / STT tools
// ---------------------------------------------------------------------------

pub(super) async fn tool_text_to_speech(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let text = input["text"].as_str().ok_or("Missing 'text' parameter")?;
    let voice = input["voice"].as_str();
    let format = input["format"].as_str();
    let provider = input["provider"].as_str();
    let output_format = input["output_format"].as_str().unwrap_or("mp3");

    if let Some(cache) = media_drivers {
        let resolved_provider =
            provider.or_else(|| tts_engine.and_then(|e| e.tts_config().provider.as_deref()));

        let driver_result = if let Some(p) = resolved_provider {
            cache.get_or_create(p, None)
        } else {
            cache.detect_for_capability(librefang_types::media::MediaCapability::TextToSpeech)
        };

        // Google TTS: override LLM-provided voice (e.g. "alloy") with the
        // configured one — Google doesn't recognise OpenAI voice names.
        let (effective_voice, effective_language, effective_speed, effective_pitch) =
            if resolved_provider == Some("google_tts") {
                if let Some(engine) = tts_engine {
                    let cfg = &engine.tts_config().google;
                    (
                        Some(cfg.voice.clone()),
                        Some(cfg.language_code.clone()),
                        Some(cfg.speaking_rate),
                        Some(cfg.pitch),
                    )
                } else {
                    (None, None, None, None)
                }
            } else {
                (None, None, None, None)
            };

        let request = librefang_types::media::MediaTtsRequest {
            text: text.to_string(),
            provider: resolved_provider.map(String::from),
            model: input["model"].as_str().map(String::from),
            voice: effective_voice.or_else(|| voice.map(String::from)),
            format: format.map(String::from),
            speed: effective_speed.or_else(|| input["speed"].as_f64().map(|v| v as f32)),
            language: effective_language.or_else(|| input["language"].as_str().map(String::from)),
            pitch: effective_pitch.or_else(|| input["pitch"].as_f64().map(|v| v as f32)),
        };

        if let Ok(driver) = driver_result {
            let result = driver
                .synthesize_speech(&request)
                .await
                .map_err(|e| e.to_string())?;

            return finish_tts_result(
                &result.audio_data,
                &result.format,
                &result.provider,
                result.duration_ms,
                workspace_root,
                output_format,
            )
            .await;
        }
        // If no driver is configured for TTS, fall through to old TtsEngine
    }

    // Fallback: old TtsEngine (OpenAI / ElevenLabs only)
    let engine =
        tts_engine.ok_or("TTS not available. No media driver or TTS engine configured.")?;

    let result = engine.synthesize(text, voice, format).await?;

    finish_tts_result(
        &result.audio_data,
        &result.format,
        &result.provider,
        Some(result.duration_estimate_ms),
        workspace_root,
        output_format,
    )
    .await
}

/// Convert audio data to OGG Opus via ffmpeg.
/// Returns `Ok(None)` if ffmpeg is not installed (caller should fall back to
/// saving the original format). Returns `Ok(Some(...))` on success with the
/// saved path, format string, and file size.
async fn convert_to_ogg_opus(
    audio_data: &[u8],
    output_dir: &Path,
    timestamp: &str,
) -> Result<Option<(Option<String>, String, usize)>, String> {
    let ogg_filename = format!("tts_{timestamp}.ogg");
    let ogg_path = output_dir.join(&ogg_filename);
    let ogg_path_str = ogg_path
        .to_str()
        .ok_or_else(|| "Output path contains invalid UTF-8".to_string())?;

    let spawn_result = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            "pipe:0",
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-ar",
            "48000",
            "-ac",
            "1",
            ogg_path_str,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let mut child = match spawn_result {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to run ffmpeg: {e}")),
    };

    // Write audio to ffmpeg stdin, then close it (EOF triggers encoding).
    // Sequential write→wait is safe: stdout is Stdio::null() so ffmpeg
    // never blocks on output, and stderr is piped but read after exit.
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(audio_data)
            .await
            .map_err(|e| format!("Failed to pipe audio to ffmpeg: {e}"))?;
        // stdin drops here → EOF sent to ffmpeg
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("ffmpeg process error: {e}"))?;

    if !output.status.success() {
        // Clean up partial output file
        let _ = tokio::fs::remove_file(&ogg_path).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last_lines: String = stderr
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "ffmpeg conversion to OGG Opus failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            last_lines
        ));
    }

    let ogg_size = tokio::fs::metadata(&ogg_path)
        .await
        .map(|m| m.len() as usize)
        .unwrap_or(0);

    if ogg_size == 0 {
        let _ = tokio::fs::remove_file(&ogg_path).await;
        return Err("ffmpeg exited successfully but produced an empty OGG file".into());
    }

    Ok(Some((
        Some(ogg_path.display().to_string()),
        "ogg".to_string(),
        ogg_size,
    )))
}

/// Save TTS audio to workspace and build JSON response.
/// When `output_format` is `"ogg_opus"` and ffmpeg is available, the saved file
/// is converted from the provider format (typically MP3) to OGG Opus so it can
/// be sent as a WhatsApp voice note. Falls back to the original format if ffmpeg
/// is not installed.
async fn finish_tts_result(
    audio_data: &[u8],
    format: &str,
    provider: &str,
    duration_ms: Option<u64>,
    workspace_root: Option<&Path>,
    output_format: &str,
) -> Result<String, String> {
    let (saved_path, final_format, final_size, warning) = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| format!("Failed to create output dir: {e}"))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();

        if output_format == "ogg_opus" && !matches!(format, "ogg" | "opus" | "ogg_opus") {
            // Try ffmpeg conversion; fall back to saving the original format if
            // ffmpeg is not installed (preserves backward compatibility).
            match convert_to_ogg_opus(audio_data, &output_dir, &timestamp).await {
                Ok(Some(result)) => (result.0, result.1, result.2, None),
                Ok(None) => {
                    let filename = format!("tts_{timestamp}.{format}");
                    let path = output_dir.join(&filename);
                    tokio::fs::write(&path, audio_data)
                        .await
                        .map_err(|e| format!("Failed to write audio file: {e}"))?;
                    (
                        Some(path.display().to_string()),
                        format.to_string(),
                        audio_data.len(),
                        Some(
                            "ffmpeg not found; saved as original format instead of ogg_opus"
                                .to_string(),
                        ),
                    )
                }
                Err(e) => {
                    tracing::warn!("OGG Opus conversion failed, falling back to {format}: {e}");
                    let filename = format!("tts_{timestamp}.{format}");
                    let path = output_dir.join(&filename);
                    tokio::fs::write(&path, audio_data)
                        .await
                        .map_err(|e| format!("Failed to write audio file: {e}"))?;
                    (
                        Some(path.display().to_string()),
                        format.to_string(),
                        audio_data.len(),
                        Some(format!(
                            "OGG Opus conversion failed, saved as {format}: {e}"
                        )),
                    )
                }
            }
        } else {
            let filename = format!("tts_{timestamp}.{format}");
            let path = output_dir.join(&filename);
            tokio::fs::write(&path, audio_data)
                .await
                .map_err(|e| format!("Failed to write audio file: {e}"))?;

            (
                Some(path.display().to_string()),
                format.to_string(),
                audio_data.len(),
                None,
            )
        }
    } else {
        (None, format.to_string(), audio_data.len(), None)
    };

    let mut response = serde_json::json!({
        "saved_to": saved_path,
        "format": final_format,
        "provider": provider,
        "duration_estimate_ms": duration_ms,
        "size_bytes": final_size,
    });

    if let Some(w) = &warning {
        response["warning"] = serde_json::json!(w);
    }

    // When no workspace is available (e.g. MCP context), include base64 audio
    if saved_path.is_none() && !audio_data.is_empty() {
        use base64::Engine;
        response["audio_base64"] =
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode(audio_data));
    }

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

pub(super) async fn tool_speech_to_text(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let engine = media_engine.ok_or("Media engine not available for speech-to-text")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _language = input["language"].as_str();

    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

    // Read the audio file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    // Determine MIME type from extension. Unknown extensions fall back to
    // audio/mpeg here (the speech_to_text path is permissive); the strict
    // form lives in `tool_media_transcribe`, which rejects unknown formats.
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp3")
        .to_lowercase();
    let mime_type = audio_mime_from_ext(&ext).unwrap_or("audio/mpeg");

    use librefang_types::media::{MediaAttachment, MediaSource, MediaType};
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: mime_type.to_string(),
        source: MediaSource::Base64 {
            data: {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&data)
            },
            mime_type: mime_type.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;

    let response = serde_json::json!({
        "transcript": understanding.description,
        "provider": understanding.provider,
        "model": understanding.model,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}
