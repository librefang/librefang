//! PDF text extraction for chat attachments.
//!
//! Wraps `pdf_extract::extract_text_from_mem` with two pieces of safety
//! the chat send pipeline needs:
//!   1. Panic isolation. `pdf-extract` (and its lopdf dependency) is
//!      historically panic-happy on malformed/encrypted PDFs. We catch
//!      unwinds so a single bad attachment never takes down the request.
//!   2. Output capping. The agent loop forwards the extracted text into
//!      the LLM context — a 200-page report would otherwise blow through
//!      the context window. We truncate at `MAX_PDF_TEXT_CHARS` and append
//!      a clear marker so callers know it happened.

use std::panic::AssertUnwindSafe;

/// Hard cap on extracted text length (characters, not bytes). 200K chars
/// is roughly 40-50K tokens — enough for most contracts/papers, well under
/// any frontier model's context. If callers want the full document they
/// should chunk + summarize, not jam it into a single user message.
pub const MAX_PDF_TEXT_CHARS: usize = 200_000;

/// Suffix appended when truncation occurs.
const TRUNCATION_MARKER: &str = "\n\n[…PDF truncated at 200K chars; original document is longer…]";

/// Extract plain text from PDF bytes.
///
/// Returns `Ok(text)` on success. Empty/whitespace-only output (typical
/// of scanned/image-only PDFs) is converted to `Err` so callers can
/// surface a useful message instead of feeding the LLM a blank attachment.
pub fn extract_text_from_pdf(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("PDF is empty".to_string());
    }

    // Catch unwind because pdf-extract / lopdf can panic on malformed
    // or encrypted documents. We keep the request alive and turn the
    // panic into a structured error string.
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(bytes)
    }));

    let raw = match result {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => return Err(format!("PDF parse failed: {e}")),
        Err(_) => return Err("PDF parser panicked (likely malformed or encrypted)".to_string()),
    };

    if raw.trim().is_empty() {
        return Err(
            "PDF contains no extractable text (scanned image-only PDF — OCR is not supported yet)"
                .to_string(),
        );
    }

    // Truncate by char count, not byte count, so we don't split a UTF-8 codepoint.
    let mut out =
        String::with_capacity(raw.len().min(MAX_PDF_TEXT_CHARS + TRUNCATION_MARKER.len()));
    for (count, c) in raw.chars().enumerate() {
        if count >= MAX_PDF_TEXT_CHARS {
            out.push_str(TRUNCATION_MARKER);
            break;
        }
        out.push(c);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_errors() {
        assert!(extract_text_from_pdf(&[]).is_err());
    }

    #[test]
    fn non_pdf_garbage_errors_without_panic() {
        // Random bytes that look nothing like a PDF — must not panic the test.
        let result = extract_text_from_pdf(b"not a pdf, definitely not");
        assert!(result.is_err());
    }
}
