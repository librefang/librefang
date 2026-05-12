//! `web_fetch_to_file` — fetch a URL directly into a workspace file.
//!
//! Sibling of `web_fetch`: same SSRF protection, DNS pinning, and redirect
//! re-validation, but the response body never enters the agent's context.
//! Instead it streams to a workspace-relative path; the tool result reports
//! only the path, byte count, sha256, and content-type.
//!
//! This is the canonical path for information-gathering agents (research,
//! ingestion, scraping) that need to persist remote documents without burning
//! prompt tokens to re-emit them through the model.

use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::web_fetch::check_ssrf;
use crate::web_search::WebToolsContext;

/// Execute `web_fetch_to_file`. Returns a short human-readable summary on
/// success; the body itself is never returned, only persisted to `dest_path`.
///
/// Caller is responsible for taint scanning the URL / headers / body before
/// invoking this — same contract as `web_fetch` in the tool dispatch arm.
pub async fn tool_web_fetch_to_file(
    input: &Value,
    web_ctx: Option<&WebToolsContext>,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let url = input
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'url' parameter")?;
    let dest_path = input
        .get("dest_path")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'dest_path' parameter")?;
    let method = input
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");
    let headers = input.get("headers").and_then(|v| v.as_object());
    let body = input.get("body").and_then(|v| v.as_str());

    let ctx =
        web_ctx.ok_or("web_fetch_to_file requires the web tool context (Web is not configured)")?;
    let engine = &ctx.fetch;
    let cfg = engine.config();
    let cap = clamp_max_bytes(
        input.get("max_bytes").and_then(|v| v.as_u64()),
        cfg.max_file_bytes,
    );

    // Resolve destination against the workspace sandbox. Mirrors `file_write`:
    // rejects `..`, accepts paths under primary workspace or any RW named
    // workspace prefix, and canonicalises through symlinks.
    let root = workspace_root
        .ok_or("Workspace sandbox not configured: web_fetch_to_file requires a workspace_root")?;
    let resolved =
        crate::workspace_sandbox::resolve_sandbox_path_ext(dest_path, root, additional_roots)?;

    // SSRF check + DNS pinning. Same pipeline as web_fetch; redirect targets
    // are re-validated by the custom redirect policy on the pinned client.
    let resolution = check_ssrf(url, &cfg.ssrf_allowed_hosts)?;
    let client = engine.pinned_client(resolution);

    let method_upper = method.to_uppercase();
    let mut req = match method_upper.as_str() {
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        "DELETE" => client.delete(url),
        _ => client.get(url),
    };
    req = req.header(
        "User-Agent",
        format!("Mozilla/5.0 (compatible; {})", crate::USER_AGENT),
    );
    if let Some(hdrs) = headers {
        for (k, v) in hdrs {
            if let Some(val) = v.as_str() {
                req = req.header(k.as_str(), val);
            }
        }
    }
    if let Some(b) = body {
        if b.trim_start().starts_with('{') || b.trim_start().starts_with('[') {
            req = req.header("Content-Type", "application/json");
        }
        req = req.body(b.to_string());
    }

    let mut resp = req
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {} from {}", status.as_u16(), url));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Fast-path Content-Length check: bail before reading a single byte when
    // the server is honest about size.
    if let Some(len) = resp.content_length() {
        if len > cap {
            return Err(format!(
                "Response too large: Content-Length {len} bytes exceeds cap {cap} bytes"
            ));
        }
    }

    // Stream chunks so a server that omits or lies about Content-Length
    // cannot push past `cap` and exhaust memory.
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len() as u64 + chunk.len() as u64 > cap {
                    return Err(format!(
                        "Response exceeded cap of {cap} bytes (server omitted or misreported Content-Length)"
                    ));
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(format!("Failed to read response body: {e}")),
        }
    }

    // Create parent directory tree (mirrors `tool_file_write`).
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create parent directories: {e}"))?;
    }
    tokio::fs::write(&resolved, &buf)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;

    let mut hasher = Sha256::new();
    hasher.update(&buf);
    let sha_hex = format!("{:x}", hasher.finalize());

    let ct_display = if content_type.is_empty() {
        "unknown"
    } else {
        &content_type
    };
    Ok(format!(
        "Wrote {bytes} bytes to {path} (sha256:{sha_hex}, content-type: {ct_display}, status: {status_code})",
        bytes = buf.len(),
        path = resolved.display(),
        status_code = status.as_u16(),
    ))
}

/// Resolve the effective per-call byte cap. The hard ceiling is always
/// `hard_cap` (from `WebFetchConfig.max_file_bytes`); a smaller agent-supplied
/// `requested` value is honoured, a larger one is silently clamped down.
/// `Some(0)` and `None` both mean "use the hard cap".
fn clamp_max_bytes(requested: Option<u64>, hard_cap: u64) -> u64 {
    match requested {
        Some(n) if n > 0 && n < hard_cap => n,
        _ => hard_cap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_uses_hard_cap_when_request_is_none() {
        assert_eq!(clamp_max_bytes(None, 50_000), 50_000);
    }

    #[test]
    fn clamp_uses_hard_cap_when_request_is_zero() {
        assert_eq!(clamp_max_bytes(Some(0), 50_000), 50_000);
    }

    #[test]
    fn clamp_lowers_to_request_when_under_cap() {
        assert_eq!(clamp_max_bytes(Some(1024), 50_000), 1024);
    }

    #[test]
    fn clamp_keeps_hard_cap_when_request_exceeds_it() {
        assert_eq!(clamp_max_bytes(Some(1_000_000), 50_000), 50_000);
    }

    #[test]
    fn clamp_keeps_hard_cap_when_request_equals_it() {
        // Equal request → use the cap (no benefit to honouring an equal request).
        assert_eq!(clamp_max_bytes(Some(50_000), 50_000), 50_000);
    }
}
