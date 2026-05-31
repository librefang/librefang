//! `image_analyze` — read an image from the agent's workspace sandbox,
//! identify its format and dimensions, and return a JSON description plus
//! a base64 preview the LLM can hand to a vision-capable provider.
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576). The shared `resolve_file_path_ext` (sandbox path resolver, still
//! `Result<_, String>`) maps to `InvalidParameter { name: "path" }` with its
//! message preserved; the file read (`io::Error`) maps to `ToolError::Upstream`
//! keeping the prefix message and the source. The format/dimension helpers are
//! infallible and unchanged.

use super::error::{ToolError, ToolResult};
use super::resolve_file_path_ext;
use std::path::Path;

const MAX_IMAGE_SIZE: u64 = 50 * 1024 * 1024;

const ALLOWED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff", "tif", "svg",
];

pub(super) async fn tool_image_analyze(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> ToolResult {
    let raw_path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;
    let prompt = input["prompt"].as_str().unwrap_or("");
    let resolved =
        resolve_file_path_ext(raw_path, workspace_root, additional_roots).map_err(|reason| {
            ToolError::InvalidParameter {
                name: "path",
                reason,
            }
        })?;

    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(ToolError::InvalidParameter {
            name: "path",
            reason: format!("File extension '.{ext}' is not a supported image format"),
        });
    }

    let metadata = tokio::fs::metadata(&resolved)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to stat image '{raw_path}': {e}"),
            source: Some(Box::new(e)),
        })?;
    if metadata.len() > MAX_IMAGE_SIZE {
        return Err(ToolError::InvalidParameter {
            name: "path",
            reason: format!(
                "File '{raw_path}' is {} bytes, exceeding the {} byte limit",
                metadata.len(),
                MAX_IMAGE_SIZE
            ),
        });
    }

    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to read image '{raw_path}': {e}"),
            source: Some(Box::new(e)),
        })?;

    let file_size = data.len();

    let format = detect_image_format(&data);
    if format == "unknown" {
        return Err(ToolError::InvalidParameter {
            name: "path",
            reason: format!("File '{raw_path}' does not match any recognized image format"),
        });
    }

    let dimensions = extract_image_dimensions(&data, &format);

    let base64_preview = if file_size <= 512 * 1024 {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&data)
    } else {
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

    Ok(serde_json::to_string_pretty(&result)?)
}

/// Check whether the byte stream looks like an SVG (XML document whose root
/// element is `<svg`).
fn is_svg(data: &[u8]) -> bool {
    let s = match std::str::from_utf8(&data[..data.len().min(512)]) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let trimmed = s.trim_start();
    if trimmed.starts_with("<?xml") {
        trimmed.contains("<svg")
    } else {
        trimmed.starts_with("<svg")
    }
}

/// Detect image format from magic bytes.
pub(super) fn detect_image_format(data: &[u8]) -> String {
    if data.len() < 4 {
        return "unknown".to_string();
    }
    if data.len() >= 8 && data.starts_with(b"\x89PNG\r\n\x1a\n") {
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
    } else if data.starts_with(b"II\x2A\x00") || data.starts_with(b"MM\x00\x2A") {
        "tiff".to_string()
    } else if is_svg(data) {
        "svg".to_string()
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
            if data.len() >= 26 {
                let w = u32::from_le_bytes([data[18], data[19], data[20], data[21]]);
                let h = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
                Some((w, h.unsigned_abs()))
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
    let mut i = 2;
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0x00 {
            i += 2;
            continue;
        }
        if (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if (0xC0..=0xC3).contains(&marker) && i + 9 < data.len() {
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
            return Some((w, h));
        }
        if (0xD8..=0xD9).contains(&marker) {
            i += 2;
            continue;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn image_analyze_missing_path_is_missing_parameter() {
        let r = tool_image_analyze(&serde_json::json!({}), None, &[]).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("path"))));
    }

    #[tokio::test]
    async fn image_analyze_without_workspace_is_invalid_parameter() {
        let r = tool_image_analyze(&serde_json::json!({"path": "x.png"}), None, &[]).await;
        assert!(matches!(
            r,
            Err(ToolError::InvalidParameter { name: "path", .. })
        ));
    }

    #[test]
    fn test_detect_image_format_png() {
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x10\x00\x00\x00\x10";
        assert_eq!(detect_image_format(data), "png");
    }

    #[test]
    fn test_detect_image_format_jpeg() {
        let data = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
        assert_eq!(detect_image_format(data), "jpeg");
    }

    #[test]
    fn test_detect_image_format_gif() {
        let data = b"GIF89a\x10\x00\x10\x00";
        assert_eq!(detect_image_format(data), "gif");
    }

    #[test]
    fn test_detect_image_format_bmp() {
        let data = b"BM\x00\x00\x00\x00";
        assert_eq!(detect_image_format(data), "bmp");
    }

    #[test]
    fn test_detect_image_format_unknown() {
        let data = b"\x00\x00\x00\x00";
        assert_eq!(detect_image_format(data), "unknown");
    }

    #[test]
    fn test_detect_image_format_tiff_le() {
        let data = b"II\x2A\x00\x08\x00\x00\x00";
        assert_eq!(detect_image_format(data), "tiff");
    }

    #[test]
    fn test_detect_image_format_tiff_be() {
        let data = b"MM\x00\x2A\x00\x00\x00\x08";
        assert_eq!(detect_image_format(data), "tiff");
    }

    #[test]
    fn test_detect_image_format_svg_bare() {
        let data = b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
        assert_eq!(detect_image_format(data), "svg");
    }

    #[test]
    fn test_detect_image_format_svg_with_xml_decl() {
        let data = b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
        assert_eq!(detect_image_format(data), "svg");
    }
}
