//! `image_analyze` — read an image from the agent's workspace sandbox,
//! identify its format and dimensions, and return a JSON description plus
//! a base64 preview the LLM can hand to a vision-capable provider.

use super::resolve_file_path_ext;
use std::path::Path;

pub(super) async fn tool_image_analyze(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let prompt = input["prompt"].as_str().unwrap_or("");
    // Route through the workspace sandbox so user-supplied paths cannot
    // escape to arbitrary filesystem locations (e.g. /etc/passwd). Named
    // workspace prefixes are honored via `additional_roots` so agents can
    // analyze images that live under declared `[workspaces]` mounts.
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;

    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read image '{raw_path}': {e}"))?;

    let file_size = data.len();

    // Detect image format from magic bytes
    let format = detect_image_format(&data);

    // Extract dimensions for common formats
    let dimensions = extract_image_dimensions(&data, &format);

    // Base64-encode (truncate for very large images in the response)
    let base64_preview = if file_size <= 512 * 1024 {
        // Under 512KB — include full base64
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&data)
    } else {
        // Over 512KB — include first 64KB preview
        use base64::Engine;
        let preview_bytes = &data[..64 * 1024];
        format!(
            "{}... [truncated, {} total bytes]",
            base64::engine::general_purpose::STANDARD.encode(preview_bytes),
            file_size
        )
    };

    let mut result = serde_json::json!({
        "path": raw_path,
        "format": format,
        "file_size_bytes": file_size,
        "file_size_human": format_file_size(file_size),
    });

    if let Some((w, h)) = dimensions {
        result["width"] = serde_json::json!(w);
        result["height"] = serde_json::json!(h);
    }

    if !prompt.is_empty() {
        result["prompt"] = serde_json::json!(prompt);
        result["note"] = serde_json::json!(
            "Vision analysis requires a vision-capable LLM. The base64 data is included for downstream processing."
        );
    }

    result["base64_preview"] = serde_json::json!(base64_preview);

    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

/// Detect image format from magic bytes.
pub(super) fn detect_image_format(data: &[u8]) -> String {
    if data.len() < 4 {
        return "unknown".to_string();
    }
    if data.starts_with(b"\x89PNG") {
        "png".to_string()
    } else if data.starts_with(b"\xFF\xD8\xFF") {
        "jpeg".to_string()
    } else if data.starts_with(b"GIF8") {
        "gif".to_string()
    } else if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
        "webp".to_string()
    } else if data.starts_with(b"BM") {
        "bmp".to_string()
    } else if data.starts_with(b"\x00\x00\x01\x00") {
        "ico".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Extract image dimensions from common formats.
pub(super) fn extract_image_dimensions(data: &[u8], format: &str) -> Option<(u32, u32)> {
    match format {
        "png" => {
            // PNG: IHDR chunk starts at byte 16, width at 16-19, height at 20-23
            if data.len() >= 24 {
                let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
                let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
                Some((w, h))
            } else {
                None
            }
        }
        "gif" => {
            // GIF: width at bytes 6-7, height at bytes 8-9 (little-endian)
            if data.len() >= 10 {
                let w = u16::from_le_bytes([data[6], data[7]]) as u32;
                let h = u16::from_le_bytes([data[8], data[9]]) as u32;
                Some((w, h))
            } else {
                None
            }
        }
        "bmp" => {
            // BMP: width at bytes 18-21, height at bytes 22-25 (little-endian)
            if data.len() >= 26 {
                let w = u32::from_le_bytes([data[18], data[19], data[20], data[21]]);
                let h = u32::from_le_bytes([data[22], data[23], data[24], data[25]]);
                Some((w, h))
            } else {
                None
            }
        }
        "jpeg" => {
            // JPEG: scan for SOF0 marker (0xFF 0xC0) to find dimensions
            extract_jpeg_dimensions(data)
        }
        _ => None,
    }
}

/// Extract JPEG dimensions by scanning for SOF markers.
pub(super) fn extract_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2; // Skip SOI marker
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        // SOF0-SOF3 markers contain dimensions
        if (0xC0..=0xC3).contains(&marker) && i + 9 < data.len() {
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
            return Some((w, h));
        }
        if i + 3 < data.len() {
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + seg_len;
        } else {
            break;
        }
    }
    None
}

/// Format file size in human-readable form.
pub(super) fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
