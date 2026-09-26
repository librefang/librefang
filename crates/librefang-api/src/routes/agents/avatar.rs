//! Per-agent avatar images: upload, serve, remove (#8339).
//!
//! # The client's filename never reaches the disk
//!
//! The body is the image and nothing else.
//! There is no filename in the path, no filename in a header that this route reads, and no `multipart` part name — because the safest handling of a client-supplied filename is not to have one.
//! What lands on disk is `{agent_id}.{ext}`: the id is a UUID this daemon minted, and the extension is chosen by sniffing the bytes against a fixed four-entry table.
//! A caller that sends `Content-Type: image/svg+xml` with PNG bytes stores a PNG; one that sends PNG headers with a name of `../../etc/cron.d/x` stores nothing different, because the name was never read.
//!
//! This is the first of the three rules #8339 asks for, and satisfying it is what makes the second one — "any client name that is stored or shown passes the shared [`filename_guard`](crate::validation::filename_guard)" — vacuous *here* rather than merely satisfied.
//! There is no name to guard. The guard's first production caller is the shared knowledge base of #8330, which does have to keep the operator's own filenames.
//!
//! # Why the files live in `~/.librefang/avatars/`
//!
//! Not in the agent's workspace: an agent lists its own workspace with `file_list`, so a filename there is text the model reads on any turn that looks at the directory.
//! Not under `~/.librefang/dashboard/`: everything below it is reachable at `/dashboard/…` and `/dashboard/assets/**` is an unauthenticated GET, so that directory is a way to serve chosen bytes from the dashboard's own origin.
//! Not in the shared upload directory: that defaults under the system temp dir and a 24-hour TTL reaper sweeps it, which would delete an agent's avatar the day after it was set.
//! See [`KernelConfig::effective_avatars_dir`](librefang_types::config::KernelConfig::effective_avatars_dir).
//!
//! # The format is decided by the bytes, and read back from the bytes
//!
//! [`image_content_type`](crate::routes::media::image_content_type) matches the magic bytes of PNG, JPEG, GIF and WebP, the same four `librefang_types::media::ALLOWED_IMAGE_TYPES` already lists.
//! SVG is absent on purpose and its absence is load-bearing: an SVG is XML that can carry script, and this daemon serves it back to a browser.
//!
//! Serving re-sniffs rather than mapping the stored extension back to a MIME.
//! The two agree today by construction, but only one of them is evidence: a file renamed on disk by anything else must not be able to change what the daemon claims it is.
//!
//! # The filesystem calls are synchronous on purpose
//!
//! `create_dir_all`, `fs::write` and `fs::read` all run on a Tokio worker thread, and the change that suggests itself — `tokio::fs` — is the wrong one here.
//! Awaiting any of them means holding a value across an `.await` while [`ErrorTranslator`] is alive, and it is `!Send`; that is the trait-bound trap this crate has walked into before, where the compiler's complaint names `Handler<_, _>` instead of the type actually at fault, and the distance between the two is most of the debugging session.
//! `spawn_blocking` after `drop(t)` would work and is the alternative if these ever grow, at the cost of a `JoinError` arm on each call that adds nothing to the error the caller already gets.
//! The bodies are bounded by [`MAX_AVATAR_BYTES`], so the window being blocked on is a few milliseconds.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentId;
use librefang_types::i18n::ErrorTranslator;

use super::AppState;
use crate::middleware::RequestLanguage;

/// Largest avatar accepted, in bytes.
///
/// An avatar is rendered in a list row and a chat header, so the ceiling that matters is "a generously sized square image", not a photograph.
/// Two mebibytes is several times any 512×512 PNG and still far below the 8 MiB global `max_request_body_bytes`, which keeps this route's own two limits the ones that answer rather than a shared layer nobody reading this file would think to check.
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

/// Slack between the handler's cap and the `Bytes` extractor's.
///
/// Setting the extractor to exactly [`MAX_AVATAR_BYTES`] puts both limits on one threshold and the extractor wins: an image a byte over gets a bodiless 413 and the handler's check — the one that says which cap was hit and by how much — becomes dead code.
/// With the slack the handler answers for anything a person plausibly uploaded, and the extractor stays as the memory backstop for a body far past the cap.
const BODY_LIMIT_HEADROOM_BYTES: usize = 64 * 1024;

/// The `DefaultBodyLimit` this route needs on its `MethodRouter`.
///
/// Without it the real ceiling is axum's own 2 MiB `Bytes` default, which sits *below* [`MAX_AVATAR_BYTES`] plus the headroom and would cut first.
pub(crate) const AVATAR_BODY_LIMIT_BYTES: usize = MAX_AVATAR_BYTES + BODY_LIMIT_HEADROOM_BYTES;

fn json_error(status: StatusCode, message: String) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Resolve `{id}` to an agent that exists, or the response to return instead.
fn resolve_agent(
    state: &AppState,
    id: &str,
    t: &ErrorTranslator,
) -> Result<AgentId, Box<axum::response::Response>> {
    // Boxed: an `axum::Response` is 128 bytes, and `clippy::result_large_err`
    // is right that carrying one in every `Ok` is the wrong trade for a
    // three-line helper.
    let agent_id: AgentId = id.parse().map_err(|_| {
        Box::new(json_error(
            StatusCode::BAD_REQUEST,
            t.t("api-error-agent-invalid-id"),
        ))
    })?;
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return Err(Box::new(json_error(
            StatusCode::NOT_FOUND,
            t.t("api-error-agent-not-found"),
        )));
    }
    Ok(agent_id)
}

/// Write `avatar_url` on the stored identity, leaving the other five fields alone.
///
/// Deliberately not `merge_agent_identity`: that exists to give a *request body* PATCH semantics, where `None` has to mean "not provided" and therefore cannot clear a field.
/// Here the whole identity is already in hand, so `None` can mean what it says and removing an avatar really removes the reference rather than storing an empty string.
fn store_avatar_url(state: &AppState, agent_id: AgentId, avatar_url: Option<String>) -> bool {
    let Some(entry) = state.kernel.agent_registry().get(agent_id) else {
        return false;
    };
    let mut identity = entry.identity;
    identity.avatar_url = avatar_url;
    if state
        .kernel
        .agent_registry()
        .update_identity(agent_id, identity)
        .is_err()
    {
        return false;
    }
    if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
        if let Err(e) = state.kernel.memory_substrate().save_agent(&entry) {
            tracing::warn!("Failed to persist agent state: {e}");
        }
    }
    true
}

/// POST /api/agents/{id}/avatar — store an image as this agent's avatar.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/avatar",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(
        content = String,
        content_type = "application/octet-stream",
        description = "The image, raw. PNG, JPEG, GIF or WebP, decided by the bytes — the request's `Content-Type` and any filename it carries are ignored."
    ),
    responses(
        (status = 200, description = "Stored; the body carries the `avatar_url` to keep", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent id", body = crate::types::JsonObject),
        (status = 404, description = "No such agent", body = crate::types::JsonObject),
        (status = 413, description = "Image larger than the cap", body = crate::types::JsonObject),
        (status = 415, description = "Bytes are not a supported image", body = crate::types::JsonObject),
        (status = 423, description = "This agent is provisioned by the deployment; its manifest cannot be changed through the API", body = crate::types::JsonObject)
    )
)]
pub async fn upload_agent_avatar(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    body: Bytes,
) -> axum::response::Response {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id = match resolve_agent(&state, &id, &t) {
        Ok(agent_id) => agent_id,
        Err(response) => return *response,
    };
    // Setting an avatar writes `avatar_url` into the manifest identity, which the next reconcile of a provisioned agent would overwrite (#6695).
    if let Some(refusal) = super::guard_provisioned_agent(&state, agent_id) {
        drop(t);
        return refusal.into_response();
    }

    if body.len() > MAX_AVATAR_BYTES {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "An avatar may be at most {MAX_AVATAR_BYTES} bytes; this one is {}.",
                body.len()
            ),
        );
    }
    // The bytes decide, not the request. A `Content-Type` header and a filename
    // are both things a caller chooses; magic bytes are the thing the browser
    // will actually try to render.
    let Some((content_type, ext)) = crate::routes::media::image_content_type(&body) else {
        return json_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "An avatar must be a PNG, JPEG, GIF or WebP image. SVG is not accepted: it is a document that can carry script, and this daemon serves avatars back to a browser.".to_string(),
        );
    };

    let avatars_dir = state.kernel.config_snapshot().effective_avatars_dir();
    if let Err(error) = std::fs::create_dir_all(&avatars_dir) {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not create the avatar directory: {error}"),
        );
    }
    // Write the bytes through `crate::atomic_write` — a staging file beside
    // the target, `fsync`, then a rename — and only then update the identity
    // and clear the candidates that would shadow it. Every failure the
    // filesystem can report therefore arrives before anything is taken away.
    //
    // That ordering is the fix, not the tidiness. Clearing the slot first —
    // which is what this did — means disk full, `EPERM` or a read-only mount
    // deletes the picture the operator had and then answers 500 with nothing
    // written, so `avatar_url` points at a route that 404s and the picture is
    // simply gone. The staging name carries the process id and a per-process
    // counter, which is the #8349 hardening and the reason this is not a
    // hand-rolled `{uuid}.{ext}.tmp`: that name was derived from the subject
    // alone, so two concurrent uploads for the same agent truncated each
    // other's bytes and whichever renamed last published a splice of the two.
    let id = agent_id.to_string();
    let path = librefang_types::media::avatar_path(&avatars_dir, &id, ext);
    if let Err(error) = crate::atomic_write(&path, &body) {
        // Nothing has been cleared either, so the refusal costs the caller
        // nothing beyond the request itself.
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not store the avatar: {error}"),
        );
    }

    // The identity is written only once the image is in place, and the order is
    // the point rather than the tidiness: recording it first meant a failed
    // rename left `avatar_url` pointing at a route with nothing behind it, which
    // is the same 404-on-every-render the ordering above exists to prevent. It
    // would have moved the damage off the disk and into the manifest, where
    // `PATCH /identity` does not accept the old value back.
    let avatar_url = librefang_types::media::agent_avatar_url(&id);
    if !store_avatar_url(&state, agent_id, Some(avatar_url.clone())) {
        // The image stays on disk, and that is the deliberate half.
        //
        // `store_avatar_url` fails on either of two things — the agent is gone,
        // or the registry refused the identity write — and it does not say
        // which. By this point the rename has already replaced whatever was at
        // this path, so deleting the new file would leave an agent that *had* an
        // avatar with none, which is the state this whole ordering exists to
        // avoid; it would just have arrived through the registry instead of the
        // disk.
        //
        // Keeping it costs less than it looks. When the agent already had one,
        // its `avatar_url` still marks the standard route and that route now
        // serves the new image — the upload effectively landed. When it did not,
        // the file sits unmarked until the next upload sets the marker.
        return json_error(StatusCode::NOT_FOUND, t.t("api-error-agent-not-found"));
    }

    // Only now, with the new file in place, are the other candidates cleared.
    // `find_avatar` probes in a fixed order, so a PNG left beside a new WebP
    // would keep being served and the upload would look like it had done
    // nothing; the file just written is held back, which is why this is not
    // `remove_avatars`.
    librefang_types::media::remove_avatars_except(&avatars_dir, &id, ext);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "avatar_url": avatar_url,
            "content_type": content_type,
            "bytes": body.len(),
        })),
    )
        .into_response()
}

/// GET /api/agents/{id}/avatar — the stored image.
///
/// Authenticated like every other `/api/` route, which is the point: the alternative placement under `~/.librefang/dashboard/` would have been an unauthenticated GET.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/avatar",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "The image", content_type = "image/png"),
        (status = 304, description = "Unchanged since the caller's `If-None-Match`"),
        (status = 400, description = "Invalid agent id", body = crate::types::JsonObject),
        (status = 404, description = "No such agent, or no avatar set", body = crate::types::JsonObject)
    )
)]
pub async fn serve_agent_avatar(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id = match resolve_agent(&state, &id, &t) {
        Ok(agent_id) => agent_id,
        Err(response) => return *response,
    };
    drop(t);

    let avatars_dir = state.kernel.config_snapshot().effective_avatars_dir();
    let Some(path) = librefang_types::media::find_avatar(&avatars_dir, &agent_id.to_string())
    else {
        return json_error(
            StatusCode::NOT_FOUND,
            "This agent has no avatar.".to_string(),
        );
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return json_error(
            StatusCode::NOT_FOUND,
            "This agent has no avatar.".to_string(),
        );
    };
    // Re-derived from the bytes, never mapped back from the stored extension:
    // a file renamed on disk must not be able to change what this route claims
    // it is. A file that no longer sniffs as an image is treated as absent
    // rather than served as `application/octet-stream`.
    let Some((content_type, _)) = crate::routes::media::image_content_type(&bytes) else {
        return json_error(
            StatusCode::NOT_FOUND,
            "This agent has no avatar.".to_string(),
        );
    };

    // A strong validator over the content, so a re-upload is picked up
    // immediately while an unchanged avatar costs one 304 per page load.
    // `no-cache` means "revalidate", not "do not store" — with `avatar_url`
    // being one fixed path per agent, a cache-busting query string would be
    // the alternative, and that would mean putting caller-influenced text back
    // into the one field this change exists to close.
    let etag = format!("\"{:016x}\"", content_hash(&bytes));
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag))
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
        )
            .into_response();
    }

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            // The bytes are an image by content check, but this route serves
            // caller-supplied data back to a browser, so say so out loud.
            (
                header::HeaderName::from_static("x-content-type-options"),
                "nosniff".to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// DELETE /api/agents/{id}/avatar — remove the image and the reference to it.
#[utoipa::path(
    delete,
    path = "/api/agents/{id}/avatar",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Removed", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent id", body = crate::types::JsonObject),
        (status = 404, description = "No such agent", body = crate::types::JsonObject),
        (status = 423, description = "This agent is provisioned by the deployment; its manifest cannot be changed through the API", body = crate::types::JsonObject)
    )
)]
pub async fn delete_agent_avatar(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id = match resolve_agent(&state, &id, &t) {
        Ok(agent_id) => agent_id,
        Err(response) => return *response,
    };
    if let Some(refusal) = super::guard_provisioned_agent(&state, agent_id) {
        drop(t);
        return refusal.into_response();
    }
    drop(t);

    let avatars_dir = state.kernel.config_snapshot().effective_avatars_dir();
    let removed = librefang_types::media::remove_avatars(&avatars_dir, &agent_id.to_string());
    // Clearing the reference is not conditional on a file having been there.
    // A stored `avatar_url` whose file is gone renders as a broken image, and
    // that state is reachable by restoring a backup of the database without
    // the avatars directory.
    store_avatar_url(&state, agent_id, None);

    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "removed": removed })),
    )
        .into_response()
}

/// FNV-1a over the bytes, for the `ETag`.
///
/// A validator only has to change when the content does; it is not a security
/// claim about the bytes, so this avoids pulling a cryptographic digest into
/// the request path for a value the client only compares for equality.
fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-pixel PNG, used wherever a test needs bytes that really sniff as one.
    pub(crate) const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89,
    ];

    #[test]
    fn the_extractor_limit_leaves_room_above_the_handler_cap() {
        // If these were equal the extractor would answer first and the
        // handler's 413 — the one that names the cap — would be unreachable.
        const {
            assert!(
                AVATAR_BODY_LIMIT_BYTES > MAX_AVATAR_BYTES,
                "the extractor must not cut at the same threshold the handler checks"
            );
        }
    }

    #[test]
    fn the_route_cap_stays_under_the_default_global_request_body_limit() {
        // The global `RequestBodyLimitLayer` wraps this route. If the route's
        // ceiling ever passed the global default the rejection would come from
        // a shared layer with no body, from a file nobody reading this one
        // would think to open.
        let global_default =
            librefang_types::config::KernelConfig::default().max_request_body_bytes;
        assert!(
            AVATAR_BODY_LIMIT_BYTES < global_default,
            "avatar cap {AVATAR_BODY_LIMIT_BYTES} must stay under the global {global_default}"
        );
    }

    #[test]
    fn the_etag_tracks_the_content() {
        assert_eq!(content_hash(TINY_PNG), content_hash(TINY_PNG));
        let mut changed = TINY_PNG.to_vec();
        changed.push(0);
        assert_ne!(content_hash(TINY_PNG), content_hash(&changed));
    }

    /// Every extension the store can produce must be one the finder probes.
    #[test]
    fn every_sniffable_extension_is_findable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let id = "11111111-1111-1111-1111-111111111111";
        for ext in librefang_types::media::AVATAR_EXTENSIONS {
            let path = librefang_types::media::avatar_path(dir.path(), id, ext);
            std::fs::write(&path, TINY_PNG).expect("write");
            assert_eq!(
                librefang_types::media::find_avatar(dir.path(), id).as_deref(),
                Some(path.as_path()),
                "an avatar stored as .{ext} must be findable"
            );
            std::fs::remove_file(&path).expect("remove");
        }
    }

    /// A format change must not leave the old file behind for the finder to return.
    #[test]
    fn removing_clears_every_candidate_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let id = "22222222-2222-2222-2222-222222222222";
        for ext in librefang_types::media::AVATAR_EXTENSIONS {
            std::fs::write(
                librefang_types::media::avatar_path(dir.path(), id, ext),
                TINY_PNG,
            )
            .expect("write");
        }
        assert_eq!(
            librefang_types::media::remove_avatars(dir.path(), id),
            librefang_types::media::AVATAR_EXTENSIONS.len()
        );
        assert!(librefang_types::media::find_avatar(dir.path(), id).is_none());
    }

    /// The avatar directory must not be anywhere an agent or the public can reach.
    #[test]
    fn avatars_live_outside_the_workspace_and_dashboard_trees() {
        let config = librefang_types::config::KernelConfig {
            home_dir: std::path::PathBuf::from("/srv/librefang"),
            ..librefang_types::config::KernelConfig::default()
        };
        let avatars = config.effective_avatars_dir();
        assert!(!avatars.starts_with(config.effective_workspaces_dir()));
        assert!(!avatars.starts_with(config.home_dir.join("dashboard")));
        assert!(avatars.starts_with(&config.home_dir));
    }
}
