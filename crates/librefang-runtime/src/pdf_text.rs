//! PDF text extraction for chat attachments.
//!
//! Wraps `pdf-extract` with three pieces of safety
//! the chat send pipeline needs:
//!   1. Input and output bounds. Oversized PDF payloads are rejected before
//!      parsing, and extracted characters are accumulated into a capped sink.
//!   2. Panic isolation. `pdf-extract` (and its lopdf dependency) is
//!      historically panic-happy on malformed/encrypted PDFs. We catch
//!      unwinds so a single bad attachment never takes down the request.
//!   3. Output capping. The agent loop forwards the extracted text into
//!      the LLM context — a 200-page report would otherwise blow through
//!      the context window. We truncate at `MAX_PDF_TEXT_CHARS` and append
//!      a clear marker so callers know it happened.

use std::panic::AssertUnwindSafe;

/// Maximum accepted PDF payload. Matches the channel attachment ceiling and
/// prevents the parser from receiving arbitrarily large in-memory documents.
pub const MAX_PDF_INPUT_BYTES: usize = 20 * 1024 * 1024;

/// Hard cap on extracted text length (characters, not bytes). 200K chars
/// is roughly 40-50K tokens — enough for most contracts/papers, well under
/// any frontier model's context. If callers want the full document they
/// should chunk + summarize, not jam it into a single user message.
pub const MAX_PDF_TEXT_CHARS: usize = 200_000;

/// Suffix appended when truncation occurs.
const TRUNCATION_MARKER: &str = "\n\n[…PDF truncated at 200K chars; original document is longer…]";

struct CappedTextWriter {
    text: String,
    chars: usize,
    truncated: bool,
}

impl CappedTextWriter {
    fn new() -> Self {
        Self {
            text: String::with_capacity(MAX_PDF_TEXT_CHARS + TRUNCATION_MARKER.len()),
            chars: 0,
            truncated: false,
        }
    }

    fn finish(mut self) -> String {
        if self.truncated {
            self.text.push_str(TRUNCATION_MARKER);
        }
        self.text
    }
}

impl std::io::Write for CappedTextWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        for character in text.chars() {
            if self.chars < MAX_PDF_TEXT_CHARS {
                self.text.push(character);
                self.chars += 1;
            } else {
                self.truncated = true;
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Extract plain text from PDF bytes.
///
/// Returns `Ok(text)` on success. Empty/whitespace-only output (typical
/// of scanned/image-only PDFs) is converted to `Err` so callers can
/// surface a useful message instead of feeding the LLM a blank attachment.
pub fn extract_text_from_pdf(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("PDF is empty".to_string());
    }
    if bytes.len() > MAX_PDF_INPUT_BYTES {
        return Err(format!(
            "PDF exceeds the {} MiB extraction limit; chunk the document or use OCR instead",
            MAX_PDF_INPUT_BYTES / (1024 * 1024)
        ));
    }

    // Catch unwind because pdf-extract / lopdf can panic on malformed
    // or encrypted documents. We keep the request alive and turn the
    // panic into a structured error string.
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut document = pdf_extract::Document::load_mem(bytes)?;
        if document.is_encrypted() {
            document.decrypt("")?;
        }
        let mut writer = CappedTextWriter::new();
        {
            let sink: &mut dyn std::io::Write = &mut writer;
            let mut output = pdf_extract::PlainTextOutput::new(sink);
            pdf_extract::output_doc(&document, &mut output)?;
        }
        Ok::<String, pdf_extract::OutputError>(writer.finish())
    }));

    let text = match result {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => return Err(format!("PDF parse failed: {e}")),
        Err(_) => return Err("PDF parser panicked (likely malformed or encrypted)".to_string()),
    };

    if text.trim().is_empty() {
        return Err(
            "PDF contains no extractable text (scanned image-only PDF — OCR is not supported yet)"
                .to_string(),
        );
    }

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_pdf(text: &str) -> Vec<u8> {
        use pdf_extract::content::{Content, Operation};
        use pdf_extract::{dictionary, Document, Object, Stream};

        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 720.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("test PDF content should encode"),
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        document
            .save_to(&mut bytes)
            .expect("test PDF should serialize");
        bytes
    }

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

    #[test]
    fn valid_pdf_text_flows_through_capped_writer() {
        let text = extract_text_from_pdf(&text_pdf("bounded output")).unwrap();
        assert!(text.contains("bounded output"));
        assert!(!text.contains(TRUNCATION_MARKER));
    }

    #[test]
    fn oversized_input_is_rejected_before_parser_entry() {
        let bytes = vec![0; MAX_PDF_INPUT_BYTES + 1];
        let error = extract_text_from_pdf(&bytes).unwrap_err();
        assert!(error.contains("20 MiB extraction limit"));
    }

    #[test]
    fn capped_writer_counts_unicode_characters_without_overallocation() {
        use std::io::Write as _;

        let mut writer = CappedTextWriter::new();
        let prefix = "界".repeat(MAX_PDF_TEXT_CHARS);
        writer.write_all(prefix.as_bytes()).unwrap();
        writer.write_all("extra".as_bytes()).unwrap();

        let output = writer.finish();
        assert!(output.ends_with(TRUNCATION_MARKER));
        assert_eq!(
            output.trim_end_matches(TRUNCATION_MARKER).chars().count(),
            MAX_PDF_TEXT_CHARS
        );
    }
}
