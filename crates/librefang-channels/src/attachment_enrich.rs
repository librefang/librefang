//! Content-aware enrichment of channel-downloaded attachments (#4448).
//!
//! When the channel bridge downloads a non-image file, the historical
//! behavior was to emit a single `[File: name] saved to /path` text
//! block — the LLM only saw the path and had to reach for a generic
//! file-reader tool, which fails on binary formats like PDF.
//!
//! The dashboard's upload flow (`librefang-api::routes::agents::resolve_attachments`)
//! already handles content-type-aware extraction: PDF text extraction,
//! inline-as-text for `text/*` and common code/data extensions, base64
//! for images. This module factors that matrix out so the channel bridge
//! can call it after streaming the download to disk and the LLM ends up
//! with parity content regardless of upload path.
//!
//! The enrichment is **additive** — callers keep the existing
//! `[File: ...] saved to ...` path block so tools that legitimately want
//! the raw bytes (e.g. `media_transcribe`, custom file readers) still
//! work. The returned blocks are inserted *before* the path block.
//!
//! Audio/voice are intentionally NOT enriched here — they go through
//! `media_transcribe` out of band, and decoding them inline would just
//! waste tokens on binary noise.

use std::panic::AssertUnwindSafe;
use std::path::Path;

use librefang_types::message::ContentBlock;
use tracing::{debug, warn};

/// Hard cap on extracted text length (chars). Mirrors
/// `librefang-runtime::pdf_text::MAX_PDF_TEXT_CHARS` and
/// `librefang-api::routes::agents::MAX_TEXT_ATTACHMENT_CHARS` so a single
/// 5 MB log paste or 200-page report doesn't blow the LLM context.
pub const MAX_ENRICHED_TEXT_CHARS: usize = 200_000;

const PDF_TRUNCATION_MARKER: &str =
    "\n\n[…PDF truncated at 200K chars; original document is longer…]";
const TEXT_TRUNCATION_MARKER: &str =
    "\n\n[…file truncated at 200K chars; content continues beyond this point…]";

/// Build extra LLM-visible content blocks for a saved channel attachment,
/// based on its media type and filename.
///
/// `saved_path` is the on-disk location the bridge streamed the download
/// to. `media_type` is the (already trimmed/lowercased) MIME from the
/// HTTP response. `filename` is the original sender-supplied name used
/// for header text and extension fallback.
///
/// Returns `Vec<ContentBlock>` containing zero or more `Text` blocks:
///   - `application/pdf` → `[Attached PDF: name (N bytes)]\n\n<extracted text>`
///   - text-like (`text/*`, json/xml/yaml/toml/code extensions) →
///     `[Attached file: name (N bytes[, truncated])]\n\n<file text>`
///   - everything else → empty vec (caller emits the path block alone)
///
/// Image content types are also returned empty here: the bridge already
/// emits a richer `ContentBlock::ImageFile` for them and double-encoding
/// would just waste tokens.
pub fn enrich_saved_file(saved_path: &Path, media_type: &str, filename: &str) -> Vec<ContentBlock> {
    let mt = media_type.trim().to_ascii_lowercase();
    let mt_base = mt.split(';').next().unwrap_or(&mt).trim();

    if mt_base == "application/pdf" {
        return enrich_pdf(saved_path, filename);
    }

    if is_text_like(mt_base, filename) {
        return enrich_text(saved_path, filename);
    }

    Vec::new()
}

fn enrich_pdf(path: &Path, filename: &str) -> Vec<ContentBlock> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read saved PDF for enrichment");
            return Vec::new();
        }
    };

    let header = format!("[Attached PDF: {} ({} bytes)]", filename, bytes.len());

    // pdf-extract / lopdf can panic on malformed or encrypted documents,
    // so isolate the unwind. Mirrors librefang-runtime::pdf_text behavior.
    let extracted = std::panic::catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(&bytes)
    }));

    let body = match extracted {
        Ok(Ok(text)) if !text.trim().is_empty() => truncate_chars(&text, PDF_TRUNCATION_MARKER),
        Ok(Ok(_)) => "[Could not extract text: scanned image-only PDF — OCR is not supported yet]"
            .to_string(),
        Ok(Err(e)) => {
            warn!(
                path = %path.display(),
                filename = %filename,
                error = %e,
                "PDF parse failed during channel enrichment; surfacing note"
            );
            format!("[Could not extract text: PDF parse failed: {e}]")
        }
        Err(_) => {
            warn!(
                path = %path.display(),
                filename = %filename,
                "PDF parser panicked during channel enrichment"
            );
            "[Could not extract text: PDF parser panicked (likely malformed or encrypted)]"
                .to_string()
        }
    };

    debug!(
        path = %path.display(),
        filename = %filename,
        size_bytes = bytes.len(),
        "Enriched channel PDF attachment with extracted text"
    );

    vec![ContentBlock::Text {
        text: format!("{header}\n\n{body}"),
        provider_metadata: None,
    }]
}

fn enrich_text(path: &Path, filename: &str) -> Vec<ContentBlock> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read saved text file for enrichment");
            return Vec::new();
        }
    };

    let raw = String::from_utf8_lossy(&bytes);
    let total_chars = raw.chars().count();
    let (body, truncated) = if total_chars > MAX_ENRICHED_TEXT_CHARS {
        let mut s: String = raw.chars().take(MAX_ENRICHED_TEXT_CHARS).collect();
        s.push_str(TEXT_TRUNCATION_MARKER);
        (s, true)
    } else {
        (raw.into_owned(), false)
    };
    let suffix = if truncated { ", truncated" } else { "" };
    let header = format!(
        "[Attached file: {} ({} bytes{})]",
        filename,
        bytes.len(),
        suffix
    );

    debug!(
        path = %path.display(),
        filename = %filename,
        size_bytes = bytes.len(),
        kept_chars = body.chars().count(),
        truncated,
        "Enriched channel text attachment inline"
    );

    vec![ContentBlock::Text {
        text: format!("{header}\n\n{body}"),
        provider_metadata: None,
    }]
}

fn truncate_chars(raw: &str, marker: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_ENRICHED_TEXT_CHARS + marker.len()));
    for (count, c) in raw.chars().enumerate() {
        if count >= MAX_ENRICHED_TEXT_CHARS {
            out.push_str(marker);
            return out;
        }
        out.push(c);
    }
    out
}

/// Mirror of `librefang-api::routes::agents::is_text_like_attachment` —
/// kept in lock-step. Browsers / channel APIs frequently set empty or
/// `application/octet-stream` content types for code files, so we fall
/// back to extension matching.
fn is_text_like(content_type: &str, filename: &str) -> bool {
    if content_type.starts_with("text/") {
        return true;
    }
    let known_mime = matches!(
        content_type,
        "application/json"
            | "application/xml"
            | "application/yaml"
            | "application/x-yaml"
            | "application/toml"
            | "application/x-toml"
            | "application/x-ipynb+json"
            | "application/javascript"
            | "application/x-javascript"
            | "application/typescript"
            | "application/sql"
            | "application/graphql"
    );
    if known_mime {
        return true;
    }
    let ext = filename
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "rst"
            | "csv"
            | "tsv"
            | "log"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "ini"
            | "conf"
            | "cfg"
            | "env"
            | "properties"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "mjs"
            | "cjs"
            | "vue"
            | "svelte"
            | "py"
            | "rs"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "swift"
            | "scala"
            | "clj"
            | "ex"
            | "exs"
            | "c"
            | "cpp"
            | "cc"
            | "cxx"
            | "h"
            | "hpp"
            | "hh"
            | "m"
            | "mm"
            | "rb"
            | "php"
            | "pl"
            | "lua"
            | "r"
            | "jl"
            | "dart"
            | "zig"
            | "nim"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "sql"
            | "graphql"
            | "gql"
            | "proto"
            | "ipynb"
            | "dockerfile"
            | "makefile"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_tmp(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn unknown_binary_returns_empty() {
        let f = write_tmp(b"\x00\x01\x02 binary garbage");
        let out = enrich_saved_file(f.path(), "application/octet-stream", "blob.bin");
        assert!(out.is_empty(), "binary blobs must not be inlined");
    }

    #[test]
    fn image_type_returns_empty() {
        // Caller already emits ContentBlock::ImageFile; we should not double-encode.
        let f = write_tmp(b"\x89PNG\r\n\x1a\n");
        let out = enrich_saved_file(f.path(), "image/png", "pic.png");
        assert!(out.is_empty());
    }

    #[test]
    fn text_file_inlined_with_header() {
        let f = write_tmp(b"hello world\nline two\n");
        let out = enrich_saved_file(f.path(), "text/plain", "notes.txt");
        assert_eq!(out.len(), 1);
        match &out[0] {
            ContentBlock::Text { text, .. } => {
                assert!(text.starts_with("[Attached file: notes.txt"));
                assert!(text.contains("hello world"));
                assert!(text.contains("line two"));
            }
            other => panic!("expected Text block, got {other:?}"),
        }
    }

    #[test]
    fn code_file_recognized_by_extension_when_mime_is_octet_stream() {
        let f = write_tmp(b"fn main() { println!(\"hi\"); }\n");
        let out = enrich_saved_file(f.path(), "application/octet-stream", "main.rs");
        assert_eq!(
            out.len(),
            1,
            "Rust source must be inlined via extension fallback"
        );
        match &out[0] {
            ContentBlock::Text { text, .. } => {
                assert!(text.contains("fn main()"));
            }
            _ => panic!("expected Text block"),
        }
    }

    #[test]
    fn json_with_application_mime_is_inlined() {
        let f = write_tmp(b"{\"k\": 1}");
        let out = enrich_saved_file(f.path(), "application/json", "data.json");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn text_truncates_long_content_with_marker() {
        let big: String = "A".repeat(MAX_ENRICHED_TEXT_CHARS + 1000);
        let f = write_tmp(big.as_bytes());
        let out = enrich_saved_file(f.path(), "text/plain", "huge.txt");
        assert_eq!(out.len(), 1);
        match &out[0] {
            ContentBlock::Text { text, .. } => {
                assert!(text.contains(", truncated)"));
                assert!(text.ends_with("beyond this point…]"));
            }
            _ => panic!("expected Text block"),
        }
    }

    #[test]
    fn pdf_garbage_does_not_panic_returns_note() {
        // Non-PDF bytes labeled as PDF must surface a "could not extract"
        // note rather than panic out of the bridge. Mirrors
        // librefang-runtime::pdf_text panic-isolation guarantee.
        let f = write_tmp(b"definitely not a pdf");
        let out = enrich_saved_file(f.path(), "application/pdf", "fake.pdf");
        assert_eq!(out.len(), 1);
        match &out[0] {
            ContentBlock::Text { text, .. } => {
                assert!(text.starts_with("[Attached PDF: fake.pdf"));
                assert!(text.contains("Could not extract text"));
            }
            _ => panic!("expected Text block"),
        }
    }

    #[test]
    fn missing_file_returns_empty_without_panic() {
        let out = enrich_saved_file(
            Path::new("/nonexistent/path/xyz.txt"),
            "text/plain",
            "xyz.txt",
        );
        assert!(out.is_empty());
    }

    #[test]
    fn mime_with_parameters_still_matches() {
        let f = write_tmp(b"hi");
        // e.g. "text/plain; charset=utf-8"
        let out = enrich_saved_file(f.path(), "text/plain; charset=utf-8", "x.txt");
        assert_eq!(out.len(), 1);
    }
}
