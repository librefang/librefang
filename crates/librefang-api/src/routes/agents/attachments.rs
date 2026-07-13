use super::*;

/// Hard cap on inlined text-attachment length (chars). Mirrors the PDF
/// truncation cap so a 5 MB `.log` paste doesn't blow the LLM context.
const MAX_TEXT_ATTACHMENT_CHARS: usize = 200_000;

const TEXT_TRUNCATION_MARKER: &str =
    "\n\n[…file truncated at 200K chars; content continues beyond this point…]";

/// Hard cap on a fetched URL attachment's body (bytes). Bounds peak memory
/// for the streaming download in `resolve_url_attachments`.
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

/// Append `chunk` to `buf` unless doing so would push it past `cap`. Returns
/// `false` (leaving `buf` unchanged) when the cap would be exceeded, so a
/// streaming caller can abort before buffering an over-cap body.
fn push_within_cap(buf: &mut Vec<u8>, chunk: &[u8], cap: usize) -> bool {
    if buf.len().saturating_add(chunk.len()) > cap {
        return false;
    }
    buf.extend_from_slice(chunk);
    true
}

/// Decide whether an attachment looks like a UTF-8 text/code/data file
/// the LLM can read directly. Browsers don't set `content_type` reliably
/// for code files (`.rs`, `.py` typically come through as empty or
/// `application/octet-stream`), so we fall back to extension matching.
fn is_text_like_attachment(content_type: &str, filename: &str) -> bool {
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
        // Plain text & docs
        "txt" | "md" | "markdown" | "rst" | "csv" | "tsv" | "log"
        // Config & data
        | "json" | "yaml" | "yml" | "toml" | "xml" | "ini" | "conf" | "cfg" | "env" | "properties"
        // Web
        | "html" | "htm" | "css" | "scss" | "sass" | "less"
        // JS/TS family
        | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "vue" | "svelte"
        // Other languages
        | "py" | "rs" | "go" | "java" | "kt" | "kts" | "swift" | "scala" | "clj" | "ex" | "exs"
        | "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hh" | "m" | "mm"
        | "rb" | "php" | "pl" | "lua" | "r" | "jl" | "dart" | "zig" | "nim"
        // Shell
        | "sh" | "bash" | "zsh" | "fish" | "ps1"
        // Query / schema
        | "sql" | "graphql" | "gql" | "proto"
        // Notebooks
        | "ipynb"
        // Build files (no extension is rare; keep names like Dockerfile out — accept attribute can't match those)
        | "dockerfile" | "makefile"
    )
}

/// Resolve uploaded file attachments into content blocks.
///
/// Reads each file from the upload directory and produces blocks the
/// agent loop can consume:
///   - `image/*` → `ContentBlock::Image` (base64-encoded inline)
///   - `application/pdf` → `ContentBlock::Text` with a `[Attached PDF: <filename>]`
///     header followed by extracted plain text (truncated at 200K chars).
///     Scanned/image-only PDFs surface as a text note explaining no text
///     was extractable, so the LLM at least sees the attachment exists.
///   - text-like files (any `text/*`, `application/json|xml|yaml|toml|…`,
///     plus common code/data extensions) → `ContentBlock::Text` with a
///     `[Attached file: <filename>]` header. Read as UTF-8 lossy and
///     truncated at 200K chars.
///   - everything else → skipped with a warn log.
pub fn resolve_attachments(
    state: &AppState,
    attachments: &[AttachmentRef],
) -> Vec<librefang_types::message::ContentBlock> {
    use base64::Engine;

    let upload_dir = state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    let mut blocks = Vec::new();

    for att in attachments {
        // Look up metadata from the upload registry
        let meta = UPLOAD_REGISTRY.get(&att.file_id);
        let (raw_content_type, filename) = if let Some(ref m) = meta {
            (m.content_type.clone(), m.filename.clone())
        } else if !att.content_type.is_empty() {
            (att.content_type.clone(), att.file_id.clone())
        } else {
            continue; // Skip unknown attachments
        };

        // Normalize MIME for downstream branching: drop parameters
        // (`application/pdf; charset=binary`) and lowercase. Without this,
        // a `Content-Type: Application/PDF` header would skip the PDF branch
        // and silently drop the attachment.
        let content_type = librefang_types::media::mime_base(&raw_content_type);

        // Validate file_id is a UUID to prevent path traversal
        if uuid::Uuid::parse_str(&att.file_id).is_err() {
            continue;
        }

        let file_path = upload_dir.join(&att.file_id);

        if content_type.starts_with("image/") {
            match std::fs::read(&file_path) {
                Ok(data) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                    tracing::info!(
                        file_id = %att.file_id,
                        filename = %filename,
                        content_type = %content_type,
                        size_bytes = data.len(),
                        "Resolved image attachment into Image block"
                    );
                    blocks.push(librefang_types::message::ContentBlock::Image {
                        media_type: content_type,
                        data: b64,
                    });
                }
                Err(e) => {
                    tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read image upload");
                }
            }
        } else if content_type == "application/pdf" {
            match std::fs::read(&file_path) {
                Ok(data) => {
                    let header = format!("[Attached PDF: {} ({} bytes)]", filename, data.len());
                    let body = match librefang_kernel::pdf_text::extract_text_from_pdf(&data) {
                        Ok(text) => text,
                        Err(e) => {
                            tracing::warn!(
                                file_id = %att.file_id,
                                filename = %filename,
                                error = %e,
                                "PDF text extraction failed; surfacing as note to LLM"
                            );
                            format!("[Could not extract text: {e}]")
                        }
                    };
                    tracing::info!(
                        file_id = %att.file_id,
                        filename = %filename,
                        size_bytes = data.len(),
                        extracted_chars = body.chars().count(),
                        "Resolved PDF attachment into Text block"
                    );
                    blocks.push(librefang_types::message::ContentBlock::Text {
                        text: format!("{header}\n\n{body}"),
                        provider_metadata: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read PDF upload");
                }
            }
        } else if is_text_like_attachment(&content_type, &filename) {
            match std::fs::read(&file_path) {
                Ok(data) => {
                    let raw = String::from_utf8_lossy(&data);
                    let total_chars = raw.chars().count();
                    let (body, truncated) = if total_chars > MAX_TEXT_ATTACHMENT_CHARS {
                        let mut s: String = raw.chars().take(MAX_TEXT_ATTACHMENT_CHARS).collect();
                        s.push_str(TEXT_TRUNCATION_MARKER);
                        (s, true)
                    } else {
                        (raw.into_owned(), false)
                    };
                    let suffix = if truncated { ", truncated" } else { "" };
                    let header = format!(
                        "[Attached file: {} ({} bytes{})]",
                        filename,
                        data.len(),
                        suffix
                    );
                    tracing::info!(
                        file_id = %att.file_id,
                        filename = %filename,
                        content_type = %content_type,
                        size_bytes = data.len(),
                        kept_chars = body.chars().count(),
                        truncated,
                        "Resolved text attachment into Text block"
                    );
                    blocks.push(librefang_types::message::ContentBlock::Text {
                        text: format!("{header}\n\n{body}"),
                        provider_metadata: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read text upload");
                }
            }
        } else {
            tracing::warn!(
                file_id = %att.file_id,
                content_type = %content_type,
                filename = %filename,
                "Attachment type not yet wired into the agent loop; skipping"
            );
        }
    }

    blocks
}

/// Pre-insert attachment content blocks (image / extracted-text-from-PDF /
/// text files) into an agent's session so the LLM can see them.
///
/// Injects a single user-role message containing all blocks BEFORE the
/// kernel adds the user's text message, so the LLM receives:
/// `[..., User(attach_blocks), User(text)]`. session_repair will merge
/// those two consecutive user-role messages into one for the wire format.
///
/// **Cross-chat isolation (2026-05-20 incident).** This helper MUST land
/// the attachment blocks in the SAME session the subsequent text-part
/// dispatch will land in — otherwise images leak across chats. The
/// session id is therefore resolved with the same priority as
/// `send_message_streaming_with_incognito` /
/// `send_message_with_incognito`:
///
/// 1. Explicit `session_id_override` from the caller (multi-tab UIs).
/// 2. `SessionId::for_sender_scope(agent, channel, chat_id)` when a
///    `sender_context` with a non-empty `channel` is present AND the
///    sender isn't asking for the canonical session.
/// 3. The agent's persistent `entry.session_id` as a last resort.
///
/// Falling back to "agent default session" without going through the
/// resolver is the very bug this signature fixes — see
/// `crates/librefang-kernel-handle/src/lib.rs` `SessionWriter` doc.
///
/// Delegates to [`SessionWriter::inject_attachment_blocks`] so this call
/// site does not need to import the concrete `LibreFangKernel` type (#3744).
pub fn inject_attachments_into_session(
    kernel: &dyn SessionWriter,
    agent_id: AgentId,
    sender_context: Option<&librefang_channels::types::SenderContext>,
    session_id_override: Option<librefang_types::agent::SessionId>,
    fallback_session_id: librefang_types::agent::SessionId,
    attachment_blocks: Vec<librefang_types::message::ContentBlock>,
) {
    let session_id = resolve_attachment_session_id(
        agent_id,
        sender_context,
        session_id_override,
        fallback_session_id,
    );
    kernel.inject_attachment_blocks(agent_id, session_id, attachment_blocks);
}

/// Resolve URL-based attachments into image content blocks.
///
/// Downloads each attachment URL, base64-encodes images, and returns
/// content blocks ready to inject into a session. Non-image attachments
/// and download failures are skipped with a warning.
///
/// SSRF defence: every URL is run through
/// [`crate::webhook_store::validate_webhook_url_resolved`] before the
/// fetch — this rejects loopback, RFC 1918, link-local, IPv6 ULA, the
/// cloud-metadata literals, and any hostname whose DNS resolves to one
/// of those families. For domain URLs we then pin reqwest to the
/// validated `SocketAddr` via `.resolve(host, addr)` so a DNS-rebind
/// flip between validation and the eventual HTTP connect cannot reroute
/// the fetch onto an internal IP. Mirrors the webhook fire-time pattern
/// at `webhooks.rs:738-744` (issue #3701).
pub async fn resolve_url_attachments(
    attachments: &[librefang_types::comms::Attachment],
) -> Vec<librefang_types::message::ContentBlock> {
    use base64::Engine;

    let mut blocks = Vec::new();

    for att in attachments {
        // Determine MIME type from explicit field or guess from URL extension
        let content_type = if let Some(ref ct) = att.content_type {
            ct.clone()
        } else {
            mime_from_url(&att.url).unwrap_or_default()
        };

        // Only process image types
        if !content_type.starts_with("image/") {
            tracing::debug!(url = %att.url, content_type, "Skipping non-image attachment");
            continue;
        }

        // SSRF guard: validate the URL (cheap scheme + literal checks)
        // and resolve its hostname against the SSRF blocklist BEFORE we
        // make any outbound request. `None` means the URL was an IP
        // literal (already covered by the cheap pre-check); `Some` means
        // we got back a validated `SocketAddr` we must pin reqwest to.
        let pinned_host = match crate::webhook_store::validate_webhook_url_resolved(&att.url).await
        {
            Ok(host) => host,
            Err(e) => {
                tracing::warn!(
                    url = %att.url,
                    error = %e,
                    "Refusing attachment URL — failed SSRF validation"
                );
                continue;
            }
        };

        // Build a per-attachment client and pin DNS to the IP we just
        // validated. Without the pin, reqwest performs its own
        // independent lookup before connecting — a low-TTL record can
        // flip to a private IP between our validation and reqwest's
        // resolver call (DNS rebind, #3701).
        let mut builder = librefang_kernel::http_client::proxied_client_builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none());
        if let Some((ref host, addr)) = pinned_host {
            builder = builder.resolve(host, addr);
        }
        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(url = %att.url, error = %e, "Failed to build HTTP client for attachment");
                continue;
            }
        };

        match client.get(&att.url).send().await {
            Ok(resp) if resp.status().is_success() => {
                // Pre-reject an advertised Content-Length over the cap so we
                // never begin buffering a declared-huge body.
                if let Some(len) = resp.content_length() {
                    if len > MAX_ATTACHMENT_BYTES as u64 {
                        tracing::warn!(url = %att.url, size = len, "Attachment too large (Content-Length), skipping");
                        continue;
                    }
                }
                // Stream chunk-by-chunk with a running cap. `resp.bytes()`
                // fully buffers the body into memory *before* any size check,
                // so a lying/absent Content-Length streaming an unbounded body
                // could OOM the daemon (bounded only by the 30s timeout, i.e.
                // multiple GB at high bandwidth). Accumulating with an in-loop
                // cap bounds peak memory to MAX_ATTACHMENT_BYTES regardless of
                // the server's send rate (mirrors the channels
                // `fetch_url_bytes_unchecked` defence).
                let mut resp = resp;
                let mut data: Vec<u8> = Vec::new();
                let collected = loop {
                    match resp.chunk().await {
                        Ok(Some(chunk)) => {
                            if !push_within_cap(&mut data, &chunk, MAX_ATTACHMENT_BYTES) {
                                tracing::warn!(url = %att.url, "Attachment exceeds size cap while streaming, skipping");
                                break None;
                            }
                        }
                        Ok(None) => break Some(data),
                        Err(e) => {
                            tracing::warn!(url = %att.url, error = %e, "Failed to read attachment body");
                            break None;
                        }
                    }
                };
                let Some(data) = collected else { continue };
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                blocks.push(librefang_types::message::ContentBlock::Image {
                    media_type: content_type,
                    data: b64,
                });
            }
            Ok(resp) => {
                tracing::warn!(url = %att.url, status = %resp.status(), "Attachment download failed");
            }
            Err(e) => {
                tracing::warn!(url = %att.url, error = %e, "Failed to fetch attachment URL");
            }
        }
    }

    blocks
}

/// Guess MIME type from a URL file extension.
fn mime_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    let ext = path.rsplit('.').next()?;
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg".into()),
        "png" => Some("image/png".into()),
        "gif" => Some("image/gif".into()),
        "webp" => Some("image/webp".into()),
        "svg" => Some("image/svg+xml".into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the streaming attachment fetch must bound peak memory by
    /// refusing a chunk that would exceed the cap (previously `resp.bytes()`
    /// buffered the whole body first, enabling remote OOM).
    #[test]
    fn push_within_cap_rejects_over_cap_chunk() {
        let mut buf = Vec::new();
        assert!(push_within_cap(&mut buf, &[0u8; 8], 10));
        assert_eq!(buf.len(), 8);
        // A chunk that would push past the cap is refused and buf is unchanged.
        assert!(!push_within_cap(&mut buf, &[0u8; 8], 10));
        assert_eq!(buf.len(), 8, "over-cap chunk must not be appended");
        // Exactly filling the cap is allowed.
        assert!(push_within_cap(&mut buf, &[0u8; 2], 10));
        assert_eq!(buf.len(), 10);
        // Zero-length chunk at the cap is fine.
        assert!(push_within_cap(&mut buf, &[], 10));
    }
}
