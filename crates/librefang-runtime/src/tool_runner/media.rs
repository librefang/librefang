//! Media understanding & generation tools — vision describe / audio
//! transcribe, image / video / music generation, text-to-speech /
//! speech-to-text.

use super::error::{ToolError, ToolResult};
use super::resolve_file_path_ext;
use librefang_types::media::{MAX_AUDIO_BYTES, MAX_IMAGE_BYTES, MAX_VIDEO_BYTES};
use std::path::Path;
use tracing::warn;

/// #3576: map the shared `resolve_file_path_ext` (still `Result<_, String>`)
/// rejection onto a typed `InvalidParameter`, message preserved.
fn resolve_media_path(
    raw_path: &str,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<std::path::PathBuf, ToolError> {
    resolve_file_path_ext(raw_path, workspace_root, additional_roots).map_err(|reason| {
        ToolError::InvalidParameter {
            name: "path",
            reason,
        }
    })
}

const ALLOWED_MEDIA_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "mp4", "mov", "mkv", "avi", "mp3", "wav",
    "ogg", "oga", "flac", "m4a", "webm", "pdf",
];

fn validate_ext(ext: &str) -> Result<(), ToolError> {
    let lower = ext.to_lowercase();
    if ALLOWED_MEDIA_EXTS.contains(&lower.as_str()) {
        Ok(())
    } else {
        Err(ToolError::InvalidParameter {
            name: "path",
            reason: format!("File extension '.{ext}' not in allowed set"),
        })
    }
}

fn image_mime_from_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

async fn read_with_size_limit(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ToolError> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to stat file: {e}")))?;
    if meta.len() > max_bytes {
        return Err(ToolError::upstream_msg(format!(
            "File too large: {} bytes (limit: {max_bytes} bytes)",
            meta.len()
        )));
    }
    tokio::fs::read(path)
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to read file: {e}")))
}

fn unique_timestamp_suffix() -> String {
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S%.3f").to_string();
    let short_id = &uuid::Uuid::new_v4().to_string()[..8];
    format!("{ts}_{short_id}")
}

/// Describe an image using a vision-capable LLM provider.
pub(super) async fn tool_media_describe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> ToolResult {
    use base64::Engine;
    let engine = media_engine.ok_or(ToolError::Unavailable("Media engine"))?;
    let raw_path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;
    let resolved = resolve_media_path(raw_path, workspace_root, additional_roots)?;

    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    validate_ext(&ext)?;
    let mime = image_mime_from_ext(&ext).ok_or_else(|| ToolError::InvalidParameter {
        name: "path",
        reason: format!("Unsupported image format: .{ext}"),
    })?;

    let data = read_with_size_limit(&resolved, MAX_IMAGE_BYTES).await?;

    let attachment = librefang_types::media::MediaAttachment {
        media_type: librefang_types::media::MediaType::Image,
        mime_type: mime.to_string(),
        source: librefang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine
        .describe_image(&attachment)
        .await
        .map_err(ToolError::upstream_msg)?;
    Ok(serde_json::to_string_pretty(&understanding)?)
}

/// Human-readable list of audio extensions accepted by `audio_mime_from_ext`,
/// surfaced in `media_transcribe` / `speech_to_text` tool schema descriptions
/// so the agent-facing format list cannot drift from the actual mapping.
pub(super) const SUPPORTED_AUDIO_EXTS_DOC: &str = "mp3, wav, ogg, oga, flac, m4a, webm";

/// Human-readable list of video-container extensions accepted by
/// `video_mime_from_ext` (#6679) — kept separate from
/// `SUPPORTED_AUDIO_EXTS_DOC` because these route through a different
/// mapping function and a different budget (`MAX_VIDEO_BYTES`).
pub(super) const SUPPORTED_VIDEO_EXTS_DOC: &str = "mp4, mov, mkv, avi";

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

/// Map a video-container-only extension to its MIME type (#6679).
///
/// `.webm` is deliberately absent here — it is also a valid audio container
/// and already reaches the provider unchanged via `audio_mime_from_ext`, so
/// routing it through the video (extract-then-transcode) path would only add
/// an unnecessary ffmpeg hop. Only extensions that hold nothing *but* video
/// belong on this list.
pub(super) fn video_mime_from_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "mp4" => Some("video/mp4"),
        "mov" => Some("video/quicktime"),
        "mkv" => Some("video/x-matroska"),
        "avi" => Some("video/x-msvideo"),
        _ => None,
    }
}

/// Build the audio (or video, extracted server-side) attachment for a
/// transcription tool call. Shared by `tool_media_transcribe` and
/// `tool_speech_to_text` so the two tools admit the same set of extensions
/// and budget video containers against `MAX_VIDEO_BYTES` rather than the
/// tighter `MAX_AUDIO_BYTES` (#6678, #6679).
async fn build_transcription_attachment(
    resolved: &Path,
    ext: &str,
) -> Result<librefang_types::media::MediaAttachment, ToolError> {
    use base64::Engine;

    if let Some(video_mime) = video_mime_from_ext(ext) {
        let data = read_with_size_limit(resolved, MAX_VIDEO_BYTES).await?;
        return Ok(librefang_types::media::MediaAttachment {
            media_type: librefang_types::media::MediaType::Video,
            mime_type: video_mime.to_string(),
            source: librefang_types::media::MediaSource::Base64 {
                data: base64::engine::general_purpose::STANDARD.encode(&data),
                mime_type: video_mime.to_string(),
            },
            size_bytes: data.len() as u64,
        });
    }

    let audio_mime = audio_mime_from_ext(ext).ok_or_else(|| ToolError::InvalidParameter {
        name: "path",
        reason: format!(
            "Unsupported audio format: .{ext} (supported: {SUPPORTED_AUDIO_EXTS_DOC}, {SUPPORTED_VIDEO_EXTS_DOC})"
        ),
    })?;
    let data = read_with_size_limit(resolved, MAX_AUDIO_BYTES).await?;
    Ok(librefang_types::media::MediaAttachment {
        media_type: librefang_types::media::MediaType::Audio,
        mime_type: audio_mime.to_string(),
        source: librefang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: audio_mime.to_string(),
        },
        size_bytes: data.len() as u64,
    })
}

/// Build the attachment for a windowed transcription: same format admission and same size budget as [`build_transcription_attachment`], but pointing at the file rather than carrying it (#6748).
///
/// The size still comes from the file's metadata, so `MediaAttachment::validate()` applies exactly the limit it applied before — the budget is unchanged, only the transport is.
async fn build_windowed_attachment(
    resolved: &Path,
    ext: &str,
) -> Result<librefang_types::media::MediaAttachment, ToolError> {
    let (media_type, mime, max_bytes) = match video_mime_from_ext(ext) {
        Some(video_mime) => (
            librefang_types::media::MediaType::Video,
            video_mime,
            MAX_VIDEO_BYTES,
        ),
        None => (
            librefang_types::media::MediaType::Audio,
            audio_mime_from_ext(ext).ok_or_else(|| ToolError::InvalidParameter {
                name: "path",
                reason: format!(
                    "Unsupported audio format: .{ext} (supported: {SUPPORTED_AUDIO_EXTS_DOC}, {SUPPORTED_VIDEO_EXTS_DOC})"
                ),
            })?,
            MAX_AUDIO_BYTES,
        ),
    };

    let meta = tokio::fs::metadata(resolved)
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to stat file: {e}")))?;
    if meta.len() > max_bytes {
        return Err(ToolError::upstream_msg(format!(
            "File too large: {} bytes (limit: {max_bytes} bytes)",
            meta.len()
        )));
    }

    Ok(librefang_types::media::MediaAttachment {
        media_type,
        mime_type: mime.to_string(),
        source: librefang_types::media::MediaSource::FilePath {
            path: resolved.to_string_lossy().into_owned(),
        },
        size_bytes: meta.len(),
    })
}

/// Default window length, in seconds of media, when `max_secs` is omitted (#6748).
///
/// Ten minutes is the largest round number that stays inside the tool-result budget on ordinary speech: a 600 s window measured ~16.8 KB of transcript against a 16 KB `spill_threshold_bytes`, so the inline path is already at its limit here and anything longer reliably spills.
/// It is deliberately not larger for the timeout's sake either — the same window took ~72 s against a self-hosted endpoint, which is already past the 60 s transcription timeout, so a caller on slow provider hardware wants `max_secs` *below* this default rather than above it.
const DEFAULT_WINDOW_SECS: f64 = 600.0;

/// Characters of transcript echoed back when the result went to a file.
///
/// Enough to confirm the right recording and the right offset at a glance, short enough that a caller looping over a long recording never accumulates a meaningful amount of it in context — which is the entire point of writing to a file.
const OUT_PATH_PREVIEW_CHARS: usize = 200;

/// Transcribe audio (or a video container's audio track) to text.
///
/// Two mechanisms bound what reaches the agent's context, and long recordings need both (#6748):
///
/// - `start_sec` / `max_secs` bound the *request*, so one call transcribes one window rather than a recording of unknown length, and the response carries `has_more` / `next_start_sec` to walk the rest.
/// - `out_path` bounds the *result*, writing the transcript to a workspace file and returning only path, byte count, sha256 and a short preview — the same contract `web_fetch_to_file` established, and for the same reason.
///
/// Windowing alone is not enough: window size varies with how much people said, so a fixed window straddles the spill threshold rather than staying under it.
pub(super) async fn tool_media_transcribe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> ToolResult {
    let engine = media_engine.ok_or(ToolError::Unavailable("Media engine"))?;
    let raw_path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;
    let resolved = resolve_media_path(raw_path, workspace_root, additional_roots)?;

    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    validate_ext(&ext)?;

    // A windowed call hands the provider a path rather than the bytes: ffmpeg seeks into the file itself, so reading and base64-encoding the whole recording would be repeated for every window of a walk and thrown away each time.
    let window = parse_window(input)?;
    let attachment = match window {
        Some(_) => build_windowed_attachment(&resolved, &ext).await?,
        None => build_transcription_attachment(&resolved, &ext).await?,
    };
    let language = input["language"].as_str();
    let prompt = input["prompt"].as_str();

    // Resolved before the provider call, not inside the write below.
    // Transcription costs an ffmpeg pass and a billed request, and on this branch the transcript exists nowhere else — it is not put in the response when a destination was named — so a path rejected afterwards destroys work that was already paid for and makes the caller pay again for the retry.
    // `web_fetch_to_file` orders it the same way, resolving its destination ahead of the SSRF check and the fetch.
    let out_dest = match input["out_path"].as_str() {
        Some(out_path) => Some(resolve_transcript_dest(
            out_path,
            workspace_root,
            additional_roots,
        )?),
        None => None,
    };

    let outcome = engine
        .transcribe_audio_window(&attachment, language, prompt, window)
        .await
        .map_err(ToolError::upstream_msg)?;

    let mut response = serde_json::json!({
        "provider": outcome.understanding.provider,
        "model": outcome.understanding.model,
    });

    // Continuation fields only when a window was requested.
    // Emitting them for a whole-file call would invite a caller to loop on a transcript that is already complete.
    if let Some(window) = window {
        let continuation = window_continuation(window, outcome.consumed_secs);
        response["window"] = serde_json::json!({
            "start_sec": window.start_sec,
            "max_secs": window.max_secs,
            // Null rather than a substituted number when the produced stream carried no readable duration: the caller can see that the walk stopped because the edge was unknown, not because the recording ended.
            "consumed_secs": outcome.consumed_secs,
        });
        response["has_more"] = serde_json::json!(continuation.has_more);
        if let Some(next) = continuation.next_start_sec {
            response["next_start_sec"] = serde_json::json!(next);
        }
    }

    let transcript = &outcome.understanding.description;
    match out_dest {
        None => {
            response["transcript"] = serde_json::json!(transcript);
        }
        Some(dest) => {
            let written = write_transcript(
                &dest,
                transcript,
                // A window starting at zero begins a fresh transcript; every later window continues the same file.
                // Without this a caller walking a recording would either overwrite each chunk with the next or have to invent its own assembly step.
                window.is_some_and(|w| w.start_sec > 0.0),
            )
            .await?;
            response["written_to"] = serde_json::json!(written.path);
            // Deliberately distinct scopes, named so: the file fields describe the artefact as it now stands on disk (every window so far), while the window fields describe only what this call produced.
            // A caller assembling a recording needs both — one to verify the artefact, one to see this step's contribution.
            response["file_bytes"] = serde_json::json!(written.bytes);
            response["file_sha256"] = serde_json::json!(written.sha256);
            response["window_chars"] = serde_json::json!(transcript.chars().count());
            response["window_preview"] = serde_json::json!(preview_of(transcript));
        }
    }

    Ok(serde_json::to_string_pretty(&response)?)
}

/// How short of `max_secs` a produced window may fall and still count as full.
///
/// Opus packets are 20 ms by default, so a window that covers its whole span still ends up to one packet short; without slack `has_more` would read every window as the last and truncate the recording after the first call.
const WINDOW_COMPLETE_TOLERANCE_SECS: f64 = 0.25;

/// Whether the walk continues after this window, and where it resumes.
#[derive(Debug, PartialEq)]
struct WindowContinuation {
    has_more: bool,
    next_start_sec: Option<f64>,
}

/// Decide whether a recording has more after `window`, given how much of it the extraction actually produced.
///
/// Extracted rather than left inline so the two boundary cases that decide whether a walk terminates — a window that came back exactly full, and one whose duration could not be read — are unit-testable without a live provider.
///
/// Advancing uses the produced length, never the requested one: a seek lands on a keyframe and a window overlapping the end of the recording is short, so an assumed edge drifts and eventually skips audio.
///
/// `consumed_secs: None` **stops** the walk.
/// That case means the produced stream carried no readable granule position, and the shape most likely to produce it is a window cut at or past the true end of the recording — where ffmpeg emits header-only or truncated output.
/// Treating unknown as "assume the full window was consumed" would advance by `max_secs` and ask again, get the same unreadable shape, and never terminate.
/// Stopping instead costs at most a short read the caller can see in the transcript, which is strictly better than a walk that cannot end.
fn window_continuation(
    window: crate::media_understanding::MediaWindow,
    consumed_secs: Option<f64>,
) -> WindowContinuation {
    let Some(consumed) = consumed_secs else {
        return WindowContinuation {
            has_more: false,
            next_start_sec: None,
        };
    };
    // A window is "full" only within a tolerance: the produced stream ends on a packet boundary, so an exactly-covered window still lands a few milliseconds short and a strict comparison would call every window the last one.
    //
    // The tolerance is capped at half the window so it can never exceed what it is a tolerance *on*.
    // `parse_window` admits any positive `max_secs`, and for one below the flat tolerance the subtraction goes negative — after which every positive `consumed` clears the bar, so a 0.2 s window that produced 0.01 s (the recording ended almost immediately) would still report "there is more".
    // Clamping the threshold at zero does not fix that: 0.01 clears a threshold of 0 just as easily.
    // Scaling does, and leaves the flat value in force for any window over half a second — which is every window the tool's own defaults and documentation steer callers toward.
    let tolerance = WINDOW_COMPLETE_TOLERANCE_SECS.min(window.max_secs / 2.0);
    let has_more = consumed > 0.0 && consumed >= window.max_secs - tolerance;
    WindowContinuation {
        has_more,
        // "There is more" is only actionable alongside a place to continue, so the cursor is derived from the same decision rather than computed separately.
        next_start_sec: has_more.then_some(window.start_sec + consumed),
    }
}

/// Read `start_sec` / `max_secs` into a window, or `None` for a whole-file call.
///
/// A call that names neither stays on the unwindowed path exactly as before, so nothing about existing callers changes.
/// Naming either one opts into windowing, with the other taking its default — asking for "the first ten minutes" should not require also spelling out that it starts at zero.
fn parse_window(
    input: &serde_json::Value,
) -> Result<Option<crate::media_understanding::MediaWindow>, ToolError> {
    let start = input.get("start_sec").filter(|v| !v.is_null());
    let max = input.get("max_secs").filter(|v| !v.is_null());
    if start.is_none() && max.is_none() {
        return Ok(None);
    }

    let as_secs = |v: Option<&serde_json::Value>, name: &'static str| -> Result<_, ToolError> {
        match v {
            None => Ok(None),
            Some(v) => {
                let n = v.as_f64().ok_or(ToolError::InvalidParameter {
                    name,
                    reason: "must be a number of seconds".to_string(),
                })?;
                if !n.is_finite() || n < 0.0 {
                    return Err(ToolError::InvalidParameter {
                        name,
                        reason: format!(
                            "must be a non-negative, finite number of seconds, got {n}"
                        ),
                    });
                }
                Ok(Some(n))
            }
        }
    };

    let start_sec = as_secs(start, "start_sec")?.unwrap_or(0.0);
    let max_secs = as_secs(max, "max_secs")?.unwrap_or(DEFAULT_WINDOW_SECS);
    if max_secs == 0.0 {
        return Err(ToolError::InvalidParameter {
            name: "max_secs",
            reason: "must be greater than zero — a zero-length window transcribes nothing"
                .to_string(),
        });
    }

    Ok(Some(crate::media_understanding::MediaWindow {
        start_sec,
        max_secs,
    }))
}

/// First [`OUT_PATH_PREVIEW_CHARS`] characters of `text`, counted in characters rather than bytes so a multi-byte script is never cut mid-codepoint.
fn preview_of(text: &str) -> String {
    let mut out: String = text.chars().take(OUT_PATH_PREVIEW_CHARS).collect();
    if text.chars().nth(OUT_PATH_PREVIEW_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// Written between consecutive windows in an assembled transcript.
///
/// A newline rather than a space so the boundary is visible to whoever reads the file, and because a window edge frequently lands mid-sentence — a line break reads as "the recording continues here", where a space would silently imply the two fragments are one phrase.
/// Nothing that was not spoken is inserted: no timestamps, no markers, so the file stays a transcript rather than a rendering of one.
const WINDOW_SEPARATOR: &str = "\n";

/// Outcome of persisting a transcript, as reported back to the agent.
struct WrittenTranscript {
    path: String,
    bytes: u64,
    sha256: String,
}

/// Resolve `out_path` against the workspace sandbox — the same call `web_fetch_to_file` and `file_write` use, so a transcript cannot be written outside the agent's roots and `..` is rejected.
///
/// Separate from the write so it can run **before** the provider call: a path rejected afterwards would destroy a transcript that cost an ffmpeg pass and a billed request and exists nowhere else.
/// It stays a pure decision — nothing is created here, so a rejected call leaves no trace on disk.
fn resolve_transcript_dest(
    out_path: &str,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<std::path::PathBuf, ToolError> {
    let root = workspace_root.ok_or_else(|| ToolError::InvalidParameter {
        name: "out_path",
        reason: "workspace sandbox is not configured, so there is no root to write under"
            .to_string(),
    })?;
    let resolved =
        crate::workspace_sandbox::resolve_sandbox_path_ext(out_path, root, additional_roots)
            .map_err(|reason| ToolError::InvalidParameter {
                name: "out_path",
                reason,
            })?;

    Ok(resolved)
}

/// Write (or append) a transcript to an already-resolved workspace path.
///
/// `append` exists because assembling a long recording is the normal case: each window continues the file the previous one started.
/// The reported `bytes` and `sha256` describe the **whole file** after the write, not just this window's share, so a caller that has walked five windows can verify the artefact it ends up with rather than the last fragment of it.
///
/// Appended windows are separated by [`WINDOW_SEPARATOR`], because a window boundary lands mid-sentence by design and the provider path trims each transcript's surrounding whitespace (`media_understanding.rs`, right after dispatch).
/// Concatenating the trimmed pieces directly would fuse the last word of one window to the first word of the next at *every* boundary, not occasionally.
/// An empty window writes nothing at all, separator included — a silent stretch of the recording must not leave a blank line in the artefact.
///
/// A failed write is not rolled back, and for the append case it cannot be cheaply: buffering the assembled transcript to rewrite it atomically would restore, on disk, the proportional-to-recording-length cost this whole parameter exists to remove.
/// What the caller gets instead is detection — `bytes` and `sha256` describe the file as it now stands, so a short or corrupted artefact is visible rather than silent.
/// Recovery is to restart the walk from `start_sec = 0`, which truncates; retrying only the failed window would append after the partial bytes.
async fn write_transcript(
    resolved: &Path,
    transcript: &str,
    append: bool,
) -> Result<WrittenTranscript, ToolError> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    let out_path = resolved.display();

    // Directories are created here, next to the write, rather than during resolution: resolution runs before the provider call and must stay a pure decision about the path, or a mistyped `out_path` would leave a directory behind on every rejected call.
    // Both neighbours order it the same way — `web_fetch_to_file` creates after the download, `file_write` immediately before writing.
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            ToolError::upstream_msg(format!("Failed to create parent directories: {e}"))
        })?;
    }

    // Appended rather than rewritten: holding every prior window in memory to rewrite the file whole would reintroduce, on disk, exactly the proportional-to-recording-length cost this parameter exists to remove.
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(resolved)
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to open '{out_path}': {e}")))?;
    // Only between windows, only when there is something to separate from, and only when there is something to separate.
    // A separator ahead of the first window would head every transcript with a stray newline; one ahead of an empty window — a pause in the recording, or the overshoot that ends a walk — would leave a blank line in the artefact for every silence.
    let needs_separator = !transcript.is_empty()
        && append
        && tokio::fs::metadata(resolved)
            .await
            .is_ok_and(|m| m.len() > 0);
    if needs_separator {
        file.write_all(WINDOW_SEPARATOR.as_bytes())
            .await
            .map_err(|e| {
                ToolError::upstream_msg(format!("Failed to write separator to '{out_path}': {e}"))
            })?;
    }
    file.write_all(transcript.as_bytes())
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to write '{out_path}': {e}")))?;
    file.flush()
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to flush '{out_path}': {e}")))?;
    drop(file);

    let whole = tokio::fs::read(resolved)
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to re-read '{out_path}': {e}")))?;
    let mut hasher = Sha256::new();
    hasher.update(&whole);

    Ok(WrittenTranscript {
        path: resolved.display().to_string(),
        bytes: whole.len() as u64,
        sha256: format!("{:x}", hasher.finalize()),
    })
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
) -> ToolResult {
    let prompt = input["prompt"]
        .as_str()
        .ok_or(ToolError::MissingParameter("prompt"))?;

    let provider = input["provider"].as_str().map(|s| s.to_string());
    let model = input["model"].as_str().map(|s| s.to_string());
    let aspect_ratio = input["aspect_ratio"].as_str().map(|s| s.to_string());
    let width = input["width"]
        .as_u64()
        .filter(|&v| v <= u64::from(u32::MAX))
        .map(|v| v as u32);
    let height = input["height"]
        .as_u64()
        .filter(|&v| v <= u64::from(u32::MAX))
        .map(|v| v as u32);
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

        request
            .validate()
            .map_err(|e| ToolError::InvalidParameter {
                name: "request",
                reason: e.to_string(),
            })?;

        let driver = if let Some(ref name) = provider {
            cache.get_or_create(name, None)
        } else {
            cache.detect_for_capability(librefang_types::media::MediaCapability::ImageGeneration)
        }
        .map_err(|e| ToolError::upstream_msg(e.to_string()))?;

        let result = driver
            .generate_image(&request)
            .await
            .map_err(|e| ToolError::upstream_msg(e.to_string()))?;

        // Save images to workspace and uploads dir
        let saved_paths = save_media_images_to_workspace(&result.images, workspace_root).await?;
        let image_urls = save_media_images_to_uploads(&result.images, upload_dir).await?;

        let response = serde_json::json!({
            "model": result.model,
            "provider": result.provider,
            "images_generated": result.images.len(),
            "saved_to": saved_paths,
            "revised_prompt": result.revised_prompt,
            "image_urls": image_urls,
        });

        return Ok(serde_json::to_string_pretty(&response)?);
    }

    // Fallback: old OpenAI-only path (when media_drivers is None)
    let model_str = input["model"].as_str().unwrap_or("dall-e-3");
    let model = match model_str {
        "dall-e-3" | "dalle3" | "dalle-3" => librefang_types::media::ImageGenModel::DallE3,
        "dall-e-2" | "dalle2" | "dalle-2" => librefang_types::media::ImageGenModel::DallE2,
        "gpt-image-1" | "gpt_image_1" => librefang_types::media::ImageGenModel::GptImage1,
        _ => {
            let reason = format!(
                "Unknown image model: {model_str}. Use 'dall-e-3', 'dall-e-2', or 'gpt-image-1'."
            );
            return Err(ToolError::InvalidParameter {
                name: "model",
                reason,
            });
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

    let result = crate::image_gen::generate_image(&request)
        .await
        .map_err(ToolError::upstream_msg)?;

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
        for img in &result.images {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&img.data_base64)
                .map_err(|e| {
                    ToolError::upstream_msg(format!("Failed to decode image data: {e}"))
                })?;
            let url = crate::uploaded_file::save_shared_upload(
                upload_dir,
                &decoded,
                "image/png",
                "generated.png",
            )
            .await
            .map_err(ToolError::upstream_msg)?;
            image_urls.push(url);
        }
    }

    let response = serde_json::json!({
        "model": result.model,
        "images_generated": result.images.len(),
        "saved_to": saved_paths,
        "revised_prompt": result.revised_prompt,
        "image_urls": image_urls,
    });

    Ok(serde_json::to_string_pretty(&response)?)
}

/// Save MediaImageResult images to workspace output/ dir.
async fn save_media_images_to_workspace(
    images: &[librefang_types::media::GeneratedImage],
    workspace_root: Option<&Path>,
) -> Result<Vec<String>, ToolError> {
    let Some(workspace) = workspace_root else {
        return Ok(Vec::new());
    };
    use base64::Engine;
    let output_dir = workspace.join("output");
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to create output dir: {e}")))?;
    let mut paths = Vec::new();
    for img in images {
        if img.data_base64.is_empty() {
            continue;
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&img.data_base64)
            .map_err(|e| ToolError::upstream_msg(format!("Failed to decode image: {e}")))?;
        let file_id = uuid::Uuid::new_v4();
        // Converge on the shared `<uuid>.<ext>` naming (#6530); these are PNGs.
        let filename = librefang_types::media::on_disk_name(&file_id.to_string(), "image/png", "");
        let path = output_dir.join(&filename);
        tokio::fs::write(&path, &decoded)
            .await
            .map_err(|e| ToolError::upstream_msg(format!("Failed to write image: {e}")))?;
        paths.push(path.display().to_string());
    }
    Ok(paths)
}

/// Save MediaImageResult images to uploads temp dir, returning /api/uploads/... URLs.
async fn save_media_images_to_uploads(
    images: &[librefang_types::media::GeneratedImage],
    upload_dir: &Path,
) -> Result<Vec<String>, ToolError> {
    use base64::Engine;
    tokio::fs::create_dir_all(upload_dir)
        .await
        .map_err(|e| ToolError::upstream_msg(format!("Failed to create upload dir: {e}")))?;
    let mut urls = Vec::new();
    for img in images {
        if img.data_base64.is_empty() {
            if let Some(ref url) = img.url {
                urls.push(url.clone());
            }
            continue;
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&img.data_base64)
            .map_err(|e| ToolError::upstream_msg(format!("Failed to decode image: {e}")))?;
        if decoded.is_empty() {
            continue;
        }
        urls.push(
            crate::uploaded_file::save_shared_upload(
                upload_dir,
                &decoded,
                "image/png",
                "generated.png",
            )
            .await
            .map_err(ToolError::upstream_msg)?,
        );
    }
    Ok(urls)
}

// ---------------------------------------------------------------------------
// Video / Music generation tools (MediaDriver-based)
// ---------------------------------------------------------------------------

/// Generate a video from a text prompt. Returns a task_id for async polling.
pub(super) async fn tool_video_generate(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
) -> ToolResult {
    let cache = media_drivers.ok_or(ToolError::Unavailable("Media drivers"))?;
    let prompt = input["prompt"]
        .as_str()
        .ok_or(ToolError::MissingParameter("prompt"))?;

    let request = librefang_types::media::MediaVideoRequest {
        prompt: prompt.to_string(),
        provider: input["provider"].as_str().map(String::from),
        model: input["model"].as_str().map(String::from),
        duration_secs: input["duration"]
            .as_u64()
            .filter(|&v| v <= u64::from(u32::MAX))
            .map(|v| v as u32),
        resolution: input["resolution"].as_str().map(String::from),
        image_url: None,
        optimize_prompt: None,
    };

    // Validate request parameters before sending to the provider
    request
        .validate()
        .map_err(|e| ToolError::InvalidParameter {
            name: "request",
            reason: e.to_string(),
        })?;

    let driver = if let Some(p) = &request.provider {
        cache
            .get_or_create(p, None)
            .map_err(|e| ToolError::upstream_msg(e.to_string()))?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::VideoGeneration)
            .map_err(|e| ToolError::upstream_msg(e.to_string()))?
    };

    let result = driver
        .submit_video(&request)
        .await
        .map_err(|e| ToolError::upstream_msg(e.to_string()))?;

    let response = serde_json::json!({
        "task_id": result.task_id,
        "provider": result.provider,
        "status": "submitted",
        "note": "Use video_status tool with this task_id to check progress"
    });

    Ok(serde_json::to_string_pretty(&response)?)
}

/// Check the status of a video generation task. Returns download URL when complete.
pub(super) async fn tool_video_status(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
) -> ToolResult {
    let cache = media_drivers.ok_or(ToolError::Unavailable("Media drivers"))?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("task_id"))?;
    let provider = input["provider"].as_str();

    let driver = if let Some(p) = provider {
        cache
            .get_or_create(p, None)
            .map_err(|e| ToolError::upstream_msg(e.to_string()))?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::VideoGeneration)
            .map_err(|e| ToolError::upstream_msg(e.to_string()))?
    };

    let status = driver
        .poll_video(task_id)
        .await
        .map_err(|e| ToolError::upstream_msg(e.to_string()))?;

    // If completed, also fetch the full result with download URL
    if status == librefang_types::media::MediaTaskStatus::Completed {
        let video_result = driver
            .get_video_result(task_id)
            .await
            .map_err(|e| ToolError::upstream_msg(e.to_string()))?;
        let response = serde_json::json!({
            "status": "completed",
            "file_url": video_result.file_url,
            "width": video_result.width,
            "height": video_result.height,
            "duration_secs": video_result.duration_secs,
            "provider": video_result.provider,
            "model": video_result.model,
        });
        return Ok(serde_json::to_string_pretty(&response)?);
    }

    let response = serde_json::json!({
        "status": status.to_string(),
        "task_id": task_id,
        "note": "Task is still in progress. Poll again after a few seconds."
    });

    Ok(serde_json::to_string_pretty(&response)?)
}

/// Generate music from a prompt and/or lyrics. Saves audio to workspace output/ directory.
pub(super) async fn tool_music_generate(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    workspace_root: Option<&Path>,
) -> ToolResult {
    let cache = media_drivers.ok_or(ToolError::Unavailable("Media drivers"))?;

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
        .map_err(|e| ToolError::InvalidParameter {
            name: "request",
            reason: e.to_string(),
        })?;

    let driver = if let Some(p) = &request.provider {
        cache
            .get_or_create(p, None)
            .map_err(|e| ToolError::upstream_msg(e.to_string()))?
    } else {
        cache
            .detect_for_capability(librefang_types::media::MediaCapability::MusicGeneration)
            .map_err(|e| ToolError::upstream_msg(e.to_string()))?
    };

    let result = driver
        .generate_music(&request)
        .await
        .map_err(|e| ToolError::upstream_msg(e.to_string()))?;

    // Save audio to workspace output/ directory (same pattern as text_to_speech)
    let saved_path = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| ToolError::upstream_msg(format!("Failed to create output dir: {e}")))?;

        let suffix = unique_timestamp_suffix();
        let filename = format!("music_{suffix}.{}", result.format);
        let path = output_dir.join(&filename);

        tokio::fs::write(&path, &result.audio_data)
            .await
            .map_err(|e| ToolError::upstream_msg(format!("Failed to write audio file: {e}")))?;

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

    Ok(serde_json::to_string_pretty(&response)?)
}

// ---------------------------------------------------------------------------
// TTS / STT tools
// ---------------------------------------------------------------------------

pub(super) async fn tool_text_to_speech(
    input: &serde_json::Value,
    media_drivers: Option<&crate::media::MediaDriverCache>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    workspace_root: Option<&Path>,
) -> ToolResult {
    let text = input["text"]
        .as_str()
        .ok_or(ToolError::MissingParameter("text"))?;
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

        // Provider-specific config overrides: inject the operator's configured
        // defaults when the tool call omitted an explicit value, so the driver
        // gets the chosen settings rather than its own hard-coded fallback.
        let (
            effective_voice,
            effective_language,
            effective_speed,
            effective_pitch,
            effective_format,
        ) = if resolved_provider == Some("google_tts") {
            // Google TTS: override LLM-provided voice (e.g. "alloy") with the
            // configured one — Google doesn't recognise OpenAI voice names.
            if let Some(engine) = tts_engine {
                let cfg = &engine.tts_config().google;
                (
                    Some(cfg.voice.clone()),
                    Some(cfg.language_code.clone()),
                    Some(cfg.speaking_rate),
                    Some(cfg.pitch),
                    None, // Google format handled by its own driver
                )
            } else {
                (None, None, None, None, None)
            }
        } else if resolved_provider == Some("elevenlabs") {
            // ElevenLabs: when the tool call omits `format`, inject the config's
            // `output_format` (default `opus_48000_32`) so the media-driver path
            // also produces Ogg/Opus for WhatsApp PTT (#6116).
            let el_format = if format.is_none() {
                tts_engine.map(|e| e.tts_config().elevenlabs.output_format.clone())
            } else {
                None
            };
            (None, None, None, None, el_format)
        } else {
            (None, None, None, None, None)
        };

        let request = librefang_types::media::MediaTtsRequest {
            text: text.to_string(),
            provider: resolved_provider.map(String::from),
            model: input["model"].as_str().map(String::from),
            voice: effective_voice.or_else(|| voice.map(String::from)),
            format: format.map(String::from).or(effective_format),
            speed: effective_speed.or_else(|| input["speed"].as_f64().map(|v| v as f32)),
            language: effective_language.or_else(|| input["language"].as_str().map(String::from)),
            pitch: effective_pitch.or_else(|| input["pitch"].as_f64().map(|v| v as f32)),
        };

        if let Ok(driver) = driver_result {
            let result = driver
                .synthesize_speech(&request)
                .await
                .map_err(|e| ToolError::upstream_msg(e.to_string()))?;

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
    let engine = tts_engine.ok_or(ToolError::Unavailable("TTS"))?;

    let result = engine
        .synthesize(text, voice, format)
        .await
        .map_err(ToolError::upstream_msg)?;

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

    let mut stdin_opt = child.stdin.take();

    let stdin_write = async {
        if let Some(mut stdin) = stdin_opt.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(audio_data).await
        } else {
            Ok(())
        }
    };

    let read_output = async { child.wait_with_output().await };

    let (write_result, output_result) = tokio::join!(stdin_write, read_output);

    write_result.map_err(|e| format!("Failed to pipe audio to ffmpeg: {e}"))?;

    let output = output_result.map_err(|e| format!("ffmpeg process error: {e}"))?;

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
) -> ToolResult {
    let (saved_path, final_format, final_size, warning) = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| ToolError::upstream_msg(format!("Failed to create output dir: {e}")))?;

        let suffix = unique_timestamp_suffix();

        if output_format == "ogg_opus" && !matches!(format, "ogg" | "opus" | "ogg_opus") {
            match convert_to_ogg_opus(audio_data, &output_dir, &suffix).await {
                Ok(Some(result)) => (result.0, result.1, result.2, None),
                Ok(None) => {
                    let filename = format!("tts_{suffix}.{format}");
                    let path = output_dir.join(&filename);
                    tokio::fs::write(&path, audio_data).await.map_err(|e| {
                        ToolError::upstream_msg(format!("Failed to write audio file: {e}"))
                    })?;
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
                    let filename = format!("tts_{suffix}.{format}");
                    let path = output_dir.join(&filename);
                    tokio::fs::write(&path, audio_data).await.map_err(|e| {
                        ToolError::upstream_msg(format!("Failed to write audio file: {e}"))
                    })?;
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
            let filename = format!("tts_{suffix}.{format}");
            let path = output_dir.join(&filename);
            tokio::fs::write(&path, audio_data)
                .await
                .map_err(|e| ToolError::upstream_msg(format!("Failed to write audio file: {e}")))?;

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

    Ok(serde_json::to_string_pretty(&response)?)
}

pub(super) async fn tool_speech_to_text(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> ToolResult {
    use base64::Engine;
    use librefang_types::media::{MediaAttachment, MediaSource, MediaType};

    let engine = media_engine.ok_or(ToolError::Unavailable("Media engine"))?;
    let raw_path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;
    let language = input["language"].as_str();
    let prompt = input["prompt"].as_str();

    let resolved = resolve_media_path(raw_path, workspace_root, additional_roots)?;

    // Determine MIME type from extension. Unknown extensions fall back to
    // audio/mpeg here (the speech_to_text path is permissive); the strict
    // form lives in `tool_media_transcribe`, which rejects unknown formats.
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp3")
        .to_lowercase();

    // A recognised video container (#6679) still needs the ffmpeg extraction
    // path and the wider `MAX_VIDEO_BYTES` budget — sending its bytes under
    // a fabricated "audio/mpeg" label, which the permissive branch below
    // would otherwise do, does not make Whisper able to decode them.
    let attachment = if let Some(video_mime) = video_mime_from_ext(&ext) {
        let data = read_with_size_limit(&resolved, MAX_VIDEO_BYTES).await?;
        MediaAttachment {
            media_type: MediaType::Video,
            mime_type: video_mime.to_string(),
            source: MediaSource::Base64 {
                data: base64::engine::general_purpose::STANDARD.encode(&data),
                mime_type: video_mime.to_string(),
            },
            size_bytes: data.len() as u64,
        }
    } else {
        let data = read_with_size_limit(&resolved, MAX_AUDIO_BYTES).await?;
        let mime_type = audio_mime_from_ext(&ext).unwrap_or("audio/mpeg");
        MediaAttachment {
            media_type: MediaType::Audio,
            mime_type: mime_type.to_string(),
            source: MediaSource::Base64 {
                data: base64::engine::general_purpose::STANDARD.encode(&data),
                mime_type: mime_type.to_string(),
            },
            size_bytes: data.len() as u64,
        }
    };

    let understanding = engine
        .transcribe_audio(&attachment, language, prompt)
        .await
        .map_err(ToolError::upstream_msg)?;

    let response = serde_json::json!({
        "transcript": understanding.description,
        "provider": understanding.provider,
        "model": understanding.model,
    });

    Ok(serde_json::to_string_pretty(&response)?)
}

#[cfg(test)]
mod transcribe_window_tests {
    use super::*;

    fn input(json: serde_json::Value) -> serde_json::Value {
        json
    }

    /// A call that names no window field stays on the whole-file path, so every pre-#6748 caller keeps its exact behaviour and gains no ffmpeg hop.
    #[test]
    fn absent_window_fields_mean_no_window() {
        assert!(parse_window(&input(serde_json::json!({"path": "a.mp3"})))
            .unwrap()
            .is_none());
        assert!(
            parse_window(&input(
                serde_json::json!({"start_sec": null, "max_secs": null})
            ))
            .unwrap()
            .is_none(),
            "explicit nulls must read as absent, not as zero"
        );
    }

    /// Naming one field opts into windowing with the other defaulted — asking for "the first ten minutes" should not require spelling out that it starts at zero.
    #[test]
    fn either_field_alone_opts_into_windowing() {
        let only_max = parse_window(&input(serde_json::json!({"max_secs": 120})))
            .unwrap()
            .expect("max_secs alone must produce a window");
        assert_eq!(only_max.start_sec, 0.0);
        assert_eq!(only_max.max_secs, 120.0);

        let only_start = parse_window(&input(serde_json::json!({"start_sec": 30})))
            .unwrap()
            .expect("start_sec alone must produce a window");
        assert_eq!(only_start.start_sec, 30.0);
        assert_eq!(
            only_start.max_secs, DEFAULT_WINDOW_SECS,
            "an unspecified length must fall back to the documented default"
        );
    }

    #[test]
    fn rejects_windows_that_cannot_describe_a_span() {
        for bad in [
            serde_json::json!({"start_sec": -1}),
            serde_json::json!({"max_secs": -0.5}),
            serde_json::json!({"max_secs": 0}),
            serde_json::json!({"start_sec": "0"}),
        ] {
            assert!(
                parse_window(&input(bad.clone())).is_err(),
                "must reject {bad}"
            );
        }
    }

    /// The preview is counted in characters, not bytes: a byte-based cut would split a multi-byte codepoint and produce invalid output for exactly the non-Latin recordings this feature is aimed at.
    #[test]
    fn preview_truncates_on_character_boundaries() {
        let cyrillic: String = "я".repeat(OUT_PATH_PREVIEW_CHARS + 50);
        let preview = preview_of(&cyrillic);
        assert_eq!(preview.chars().count(), OUT_PATH_PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));

        let short = "short transcript";
        assert_eq!(preview_of(short), short, "no ellipsis when nothing was cut");
    }

    /// Windows assemble into one artefact: the first truncates, later ones append, and the reported size and digest describe the whole file rather than the last fragment — otherwise a caller could not verify what it ended up with.
    /// The separator is the load-bearing part.
    /// A window boundary lands mid-sentence by design and each window arrives already trimmed, so without it the last word of one window fuses to the first of the next at every boundary — the common case, not an edge case.
    #[tokio::test]
    async fn windows_append_into_one_transcript() {
        let root = tempfile::tempdir().expect("tempdir");
        let first = write_transcript(&root.path().join("t.txt"), "and then we", false)
            .await
            .expect("initial write must succeed");
        let second = write_transcript(&root.path().join("t.txt"), "discussed the budget", true)
            .await
            .expect("append must succeed");

        let on_disk = tokio::fs::read_to_string(root.path().join("t.txt"))
            .await
            .expect("file must exist");
        assert_eq!(on_disk, "and then we\ndiscussed the budget");
        assert!(
            !on_disk.contains("wediscussed"),
            "consecutive windows must not fuse into one word: {on_disk:?}"
        );
        assert!(
            !on_disk.starts_with(WINDOW_SEPARATOR),
            "the first window must not be preceded by a separator"
        );
        assert_eq!(second.bytes, on_disk.len() as u64);
        assert!(
            second.bytes > first.bytes,
            "the appended write must report the whole file, not its own share"
        );
        assert_ne!(first.sha256, second.sha256);
    }

    /// An append onto a file that does not exist yet (a caller resuming a walk whose artefact was cleaned up) must not open with a stray separator.
    #[tokio::test]
    async fn append_to_a_missing_file_does_not_lead_with_a_separator() {
        let root = tempfile::tempdir().expect("tempdir");
        write_transcript(&root.path().join("t.txt"), "resumed", true)
            .await
            .expect("append must create the file");

        let on_disk = tokio::fs::read_to_string(root.path().join("t.txt"))
            .await
            .expect("file must exist");
        assert_eq!(on_disk, "resumed");
    }

    /// Walk termination, table-driven.
    /// These are the branches that decide whether a caller stops, loops, or silently skips audio, and none of them is reachable through the tool without a live provider.
    #[test]
    fn window_continuation_decides_when_the_walk_stops() {
        let w = |max_secs| crate::media_understanding::MediaWindow {
            start_sec: 100.0,
            max_secs,
        };

        // Full window → more to come, resuming at the produced edge.
        let full = window_continuation(w(600.0), Some(600.0));
        assert!(full.has_more);
        assert_eq!(full.next_start_sec, Some(700.0));

        // Short by less than one Opus packet still counts as full: the stream ends on a packet boundary, so a strict comparison would call every window the last one.
        let nearly_full = window_continuation(w(600.0), Some(599.9));
        assert!(nearly_full.has_more);
        assert_eq!(nearly_full.next_start_sec, Some(699.9));

        // Genuinely short → the recording ended inside this window.
        let short = window_continuation(w(600.0), Some(340.0));
        assert!(!short.has_more);
        assert_eq!(short.next_start_sec, None);

        // Unknown duration stops the walk rather than assuming a full window.
        // Assuming would advance past the end, get the same unreadable shape again, and never terminate.
        let unknown = window_continuation(w(600.0), None);
        assert!(!unknown.has_more);
        assert_eq!(unknown.next_start_sec, None);

        // A window that produced nothing cannot advance the cursor, and must not claim there is more either — with `max_secs` under the tolerance the comparison alone would call an empty window full, and the caller would be told to continue with nowhere to continue from.
        // Asserting only on `next_start_sec` here is what let the two fields disagree in the first place.
        let empty = window_continuation(w(0.1), Some(0.0));
        assert!(!empty.has_more, "an empty window is not a full one");
        assert_eq!(empty.next_start_sec, None);

        // A window shorter than the flat tolerance must still be judged against
        // its own length.
        // With an unscaled tolerance the threshold goes negative and any positive `consumed` clears it, so a recording that ended almost immediately would report more to come — and clamping the threshold at zero would not help, since a hair above zero clears zero too.
        let barely_started = window_continuation(w(0.2), Some(0.01));
        assert!(
            !barely_started.has_more,
            "0.01s of a 0.2s window is not a full window"
        );
        assert_eq!(barely_started.next_start_sec, None);

        // The same short window, actually filled, still continues — the scaled
        // tolerance must not make small windows unwalkable.
        let short_but_full = window_continuation(w(0.2), Some(0.19));
        assert!(
            short_but_full.has_more,
            "a short window that came back full must still continue"
        );

        // Windows above the flat tolerance keep the flat behaviour: a 20 ms
        // packet's worth of shortfall is still "full".
        let packet_short = window_continuation(w(600.0), Some(599.8));
        assert!(packet_short.has_more);

        // The two fields agree in every case, which is the invariant a caller relies on: `has_more` without a resume point is unactionable.
        for (max_secs, consumed) in [
            (600.0, Some(600.0)),
            (600.0, Some(599.9)),
            (600.0, Some(340.0)),
            (600.0, None),
            (0.1, Some(0.0)),
            (0.1, Some(0.05)),
            (0.2, Some(0.2)),
        ] {
            let c = window_continuation(w(max_secs), consumed);
            assert_eq!(
                c.has_more,
                c.next_start_sec.is_some(),
                "has_more and next_start_sec disagreed for max_secs={max_secs}, consumed={consumed:?}"
            );
        }
    }

    /// A window that transcribed to nothing — a pause in the recording, or the
    /// overshoot that ends a walk — must leave the artefact untouched.
    /// Writing the separator alone would put a blank line in the transcript for
    /// every silence, and a trailing one at the end of every walk.
    #[tokio::test]
    async fn an_empty_window_writes_nothing_at_all() {
        let root = tempfile::tempdir().expect("tempdir");
        let dest = root.path().join("t.txt");

        write_transcript(&dest, "first window", false)
            .await
            .expect("write");
        let after_silence = write_transcript(&dest, "", true)
            .await
            .expect("an empty window must not fail");

        let on_disk = tokio::fs::read_to_string(&dest).await.expect("read");
        assert_eq!(
            on_disk, "first window",
            "an empty window must add neither text nor separator"
        );
        assert_eq!(after_silence.bytes, on_disk.len() as u64);

        // And the window after the silence still separates properly from the
        // one before it, rather than inheriting a swallowed separator.
        write_transcript(&dest, "third window", true)
            .await
            .expect("write");
        assert_eq!(
            tokio::fs::read_to_string(&dest).await.expect("read"),
            "first window\nthird window"
        );
    }

    /// A restart of the walk (`start_sec = 0`) must not leave the previous run's tail behind it, which is what makes re-running a failed transcription safe.
    #[tokio::test]
    async fn a_fresh_window_truncates_the_previous_transcript() {
        let root = tempfile::tempdir().expect("tempdir");
        write_transcript(
            &root.path().join("t.txt"),
            "stale content that must not survive",
            false,
        )
        .await
        .expect("write");
        write_transcript(&root.path().join("t.txt"), "new.", false)
            .await
            .expect("write");

        let on_disk = tokio::fs::read_to_string(root.path().join("t.txt"))
            .await
            .expect("file must exist");
        assert_eq!(on_disk, "new.");
    }

    /// `out_path` goes through the same sandbox resolution as `file_write` and `web_fetch_to_file`; a transcript must not be writable outside the agent's roots.
    ///
    /// Resolution happens before the provider call, so a rejected path costs
    /// nothing; the test drives the resolver directly for that reason.
    #[test]
    fn out_path_cannot_escape_the_workspace() {
        let root = tempfile::tempdir().expect("tempdir");
        assert!(
            resolve_transcript_dest("../escaped.txt", Some(root.path()), &[]).is_err(),
            "traversal must be rejected"
        );
        assert!(
            resolve_transcript_dest("t.txt", None, &[]).is_err(),
            "without a workspace root there is no root to write under"
        );
        assert!(
            resolve_transcript_dest("t.txt", Some(root.path()), &[]).is_ok(),
            "an ordinary workspace-relative path must resolve"
        );
    }
}
