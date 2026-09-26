//! Per-user avatar images and the identity emoji (#8339).
//!
//! The sibling feature for agents is [`crate::routes::agents::avatar`], and this file is deliberately its twin: the same three handlers, the same validation, the same write ordering.
//! Read that module first — everything it says about *why* an avatar is stored as bytes and served by re-sniffing applies here unchanged.
//! What differs is one property, and it is the whole reason this is a separate file rather than a generic helper.
//!
//! # The user's name comes from the request, so it never becomes a filename
//!
//! An agent is addressed by an id this daemon minted, so `{agent_id}.{ext}` is entirely server-derived.
//! A **user is addressed by a name**, and that name is whatever the operator typed into `[[users]]` — [`super::validate_name`] bounds its length and refuses control characters, and that is all.
//! `../../etc/passwd` is a name a `[[users]]` entry can legally carry; so is `CON`, `nul`, and a 40-character string of `A`.
//!
//! So this module never passes the requested name to [`librefang_types::media::avatar_path`].
//! It passes [`UserId::from_name`], a UUIDv5 this daemon derives from the **configured** name under [`LIBREFANG_USER_NAMESPACE`], and the file that lands on disk is `{uuid}.{ext}` for a uuid the client cannot choose.
//! That keeps the property the shared helpers document — nothing client-supplied reaches the path — true for this second caller instead of quietly weakening it, and it is why [`super::validate_name`] was left alone rather than tightened: a stricter name rule would be a different feature with a different failure mode (refusing names operators already have), and it would still be the wrong fix, because the correct invariant is "the name is not a path component", not "the name is tame".
//!
//! [`LIBREFANG_USER_NAMESPACE`]: librefang_types::agent::LIBREFANG_USER_NAMESPACE
//!
//! # Reads come in two spellings, and the literal one is the default
//!
//! `GET /api/users/me/avatar` resolves the subject from the credential and has no client-controlled segment in its path at all; `GET /api/users/{name}/avatar` names someone else and exists for an Admin reading another person's picture.
//! A name with a space, an accent or a `.` survives the filesystem here but not the dashboard's token-attaching allowlist, which admits only `[A-Za-z0-9_-]+` per segment — so the picture the signed-in user sees is fetched from the literal route, and the by-name one is what a management screen uses.
//! [`serve_my_avatar`] documents why the allowlist must not be widened instead, and the corner the literal creates for a user actually named `me`.
//!
//! # The files live under `~/.librefang/avatars/users/`
//!
//! A subdirectory rather than the avatars root, because an agent avatar and a user avatar are both named `{uuid}.{ext}` for a uuid drawn from a different namespace, so a shared directory would make "which of these is a person" answerable only by running the uuid backwards.
//! `KernelConfig::effective_user_avatars_dir` is the resolver; this module never joins the `users` segment itself.
//!
//! # Only the emoji reaches `config.toml`
//!
//! The image is a file, and whether one exists is answered by probing the directory (`has_avatar` on [`super::UserView`]).
//! Storing an `avatar_url` in the user row as well would be a second copy of that answer, free to disagree with the directory after a restore — which is exactly the failure the agent side had to handle by clearing a dangling reference.
//! The emoji has nowhere else to live, so it goes through [`super::persist_identity_sections`]: comment-preserving `toml_edit` rewrite, backup, `validate_config_for_reload`, kernel reload, audit entry.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::UserId;
use librefang_types::config::UserConfig;
use serde::Deserialize;

use super::AppState;
use super::{err_response, persist_identity_sections, PersistError};
use crate::middleware::AuthenticatedApiUser;

/// Largest user avatar accepted, in bytes.
///
/// The same two mebibytes the agent route uses, and deliberately its own constant rather than an import: the two are independent resources whose sensible ceiling happens to coincide, and the route that owns each one is the module a reader would look in to find it.
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

/// Slack between the handler's cap and the `Bytes` extractor's.
///
/// Setting the extractor to exactly [`MAX_AVATAR_BYTES`] puts both limits on one threshold and the extractor wins: an image a byte over gets a bodiless 413 and the handler's check — the one that says which cap was hit and by how much — becomes dead code.
const BODY_LIMIT_HEADROOM_BYTES: usize = 64 * 1024;

/// The `DefaultBodyLimit` this route needs on its `MethodRouter`.
///
/// Without it the real ceiling is axum's own 2 MiB `Bytes` default, which sits *below* [`MAX_AVATAR_BYTES`] plus the headroom and would cut first.
pub(crate) const USER_AVATAR_BODY_LIMIT_BYTES: usize = MAX_AVATAR_BYTES + BODY_LIMIT_HEADROOM_BYTES;

/// Resolve the calling credential to its configured `[[users]]` row, if it has one.
///
/// Mirrors [`crate::routes::authz::whoami`], and for the same reason the two must agree: the name a credential resolves to is the key every `/api/users/{name}` route matches on, so an identity that endpoint reports is an identity this one can find.
/// The synthetic root credential — master api key, trusted loopback, `allow_no_auth` — carries no `AuthenticatedApiUser`, so it resolves under the literal name `"root"` and **will** match a deployment that declares a `[[users]]` entry by that name.
/// That is deliberate, not an oversight: `whoami` resolves the same way and for the same reason, and the alternative — treating the synthetic credential as owning nothing while every other route keys on the name — would be the two endpoints disagreeing about who the caller is.
/// It grants nothing a caller did not already have, because the synthetic credential is Owner-equivalent with or without a row behind it.
/// A credential whose name matches no row is `None`, which is the honest answer for an identity that owns nothing.
fn resolve_caller(state: &AppState, api_user: Option<&AuthenticatedApiUser>) -> Option<UserConfig> {
    let name = match api_user {
        Some(u) => u.name.as_str(),
        None => "root",
    };
    state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == name)
        .cloned()
}

/// Resolve `{name}` to a user that exists, or the response to return instead.
///
/// Names are matched exactly and case-sensitively, the same comparison [`super::get_user`] and every other `/api/users/{name}` route performs, so a caller cannot reach a user's avatar through a spelling that would 404 everywhere else.
fn resolve_user(state: &AppState, name: &str) -> Result<UserConfig, Box<axum::response::Response>> {
    // Boxed: an `axum::Response` is 128 bytes, and `clippy::result_large_err`
    // is right that carrying one in every `Ok` is the wrong trade for a
    // three-line helper.
    state
        .kernel
        .config_ref()
        .users
        .iter()
        .find(|u| u.name == name)
        .cloned()
        .ok_or_else(|| {
            Box::new(err_response(
                StatusCode::NOT_FOUND,
                format!("user '{name}' not found"),
            ))
        })
}

/// The server-derived file stem for a user's avatar.
///
/// Takes the **configured** user rather than the requested name so that the value passed to the filesystem is visibly not the string from the URL — see the module docs.
/// The mapping is stable (UUIDv5 under a fixed namespace), so a rename gives the user a new stem — and the file has to be moved to it explicitly, or the old stem keeps the only copy.
/// [`super::update_user`] does that with [`move_user_avatar`] after the row is persisted, which is what stops a later holder of the freed name from inheriting the picture.
fn avatar_id(user: &UserConfig) -> String {
    UserId::from_name(&user.name).to_string()
}

/// Move a renamed user's stored image from the old stem to the new one.
///
/// The stem is derived from the name ([`avatar_id`]), so without this a rename
/// leaves the file under the uuid of a name that no longer exists: the renamed
/// user's route serves nothing, and the next holder of the freed name inherits
/// the picture.
/// It is the same residue [`super::delete_user`] sweeps when a row goes away,
/// except that here the row survives and the image moves with it.
/// [`super::update_user`] calls this strictly after the `[[users]]` write, so a
/// rename the config refused cannot have carried the picture away first.
///
/// Each candidate is moved with one `fs::rename` inside one directory, so the
/// move is atomic and the file is never absent from both stems.
/// The normal directory holds a single one, because an upload clears the other
/// extensions; moving all four anyway takes a shadow file left by a crash
/// between an upload's write and its clear out of the freed stem, where it
/// would otherwise become the next holder's image.
/// A missing source is not an error — a user who never uploaded one has
/// nothing to move.
/// A real failure is returned rather than swallowed: the picture has a live
/// owner whose route now serves nothing, and `update_user` reports it.
pub(crate) fn move_user_avatar(
    dir: &std::path::Path,
    old_name: &str,
    new_name: &str,
) -> std::io::Result<()> {
    let old_id = UserId::from_name(old_name).to_string();
    let new_id = UserId::from_name(new_name).to_string();
    for ext in librefang_types::media::AVATAR_EXTENSIONS {
        let old_path = librefang_types::media::avatar_path(dir, &old_id, ext);
        let new_path = librefang_types::media::avatar_path(dir, &new_id, ext);
        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// `PATCH /api/users/{name}/identity` — the emoji that stands for this user.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UserIdentityUpdate {
    /// The glyph to store, or `null` to clear the one already stored. Absent is treated as `null`.
    #[serde(default)]
    pub emoji: Option<String>,
}

/// POST /api/users/{name}/avatar — store an image as this user's avatar.
///
/// `POST` rather than `PUT`, matching `POST /api/agents/{id}/avatar`: the two are one feature on two resources, and a caller reading one route should not have to discover that the other spells the same operation differently.
///
/// Not reachable for a `[[users]]` entry named `me`, whose path is shadowed by the literal read route — see [`serve_my_avatar`].
#[utoipa::path(
    post,
    path = "/api/users/{name}/avatar",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    request_body(
        content = String,
        content_type = "application/octet-stream",
        description = "The image, raw. PNG, JPEG, GIF or WebP, decided by the bytes — the request's `Content-Type` and any filename it carries are ignored."
    ),
    responses(
        (status = 200, description = "Stored", body = crate::types::JsonObject),
        (status = 404, description = "No such user", body = crate::types::JsonObject),
        (status = 413, description = "Image larger than the cap", body = crate::types::JsonObject),
        (status = 415, description = "Bytes are not a supported image", body = crate::types::JsonObject)
    )
)]
pub async fn upload_user_avatar(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Bytes,
) -> axum::response::Response {
    let user = match resolve_user(&state, &name) {
        Ok(user) => user,
        Err(response) => return *response,
    };

    if body.len() > MAX_AVATAR_BYTES {
        return err_response(
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
        return err_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "An avatar must be a PNG, JPEG, GIF or WebP image. SVG is not accepted: it is a document that can carry script, and this daemon serves avatars back to a browser.",
        );
    };

    let dir = state.kernel.config_snapshot().effective_user_avatars_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        return err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not create the avatar directory: {error}"),
        );
    }
    // Write the bytes through `crate::atomic_write` — a staging file beside
    // the target, `fsync`, then a rename — and only then clear the candidates
    // that would shadow it. Every failure the filesystem can report therefore
    // arrives before anything is taken away: clearing the slot first would
    // mean disk full, `EPERM` or a read-only mount deletes the picture the
    // operator had and then answers 500 with nothing written.
    //
    // The staging name carries the process id and a per-process counter, which
    // is the #8349 hardening and the reason this is not a hand-rolled
    // `{uuid}.{ext}.tmp`: that name was derived from the subject alone, so two
    // concurrent uploads for the same user truncated each other's bytes and
    // whichever renamed last published a splice of the two.
    let id = avatar_id(&user);
    let path = librefang_types::media::avatar_path(&dir, &id, ext);
    if let Err(error) = crate::atomic_write(&path, &body) {
        // Nothing has been cleared either, so the refusal costs the caller
        // nothing beyond the request itself.
        return err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not store the avatar: {error}"),
        );
    }

    // Only now, with the new file in place, are the other candidates cleared.
    // `find_avatar` probes in a fixed order, so a PNG left beside a new WebP
    // would keep being served and the upload would look like it had done
    // nothing; the file just written is held back, which is why this is not
    // `remove_avatars`.
    librefang_types::media::remove_avatars_except(&dir, &id, ext);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "content_type": content_type,
            "bytes": body.len(),
        })),
    )
        .into_response()
}

/// GET /api/users/{name}/avatar — the stored image.
///
/// Authenticated like every other `/api/` route, which is the point: the alternative placement under `~/.librefang/dashboard/` would have been an unauthenticated GET.
#[utoipa::path(
    get,
    path = "/api/users/{name}/avatar",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    responses(
        (status = 200, description = "The image", content_type = "image/png"),
        (status = 304, description = "Unchanged since the caller's `If-None-Match`"),
        (status = 404, description = "No such user, or no avatar set", body = crate::types::JsonObject)
    )
)]
pub async fn serve_user_avatar(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let user = match resolve_user(&state, &name) {
        Ok(user) => user,
        Err(response) => return *response,
    };

    let dir = state.kernel.config_snapshot().effective_user_avatars_dir();
    let Some(path) = librefang_types::media::find_avatar(&dir, &avatar_id(&user)) else {
        return err_response(StatusCode::NOT_FOUND, "This user has no avatar.");
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return err_response(StatusCode::NOT_FOUND, "This user has no avatar.");
    };
    // Re-derived from the bytes, never mapped back from the stored extension:
    // a file renamed on disk must not be able to change what this route claims
    // it is. A file that no longer sniffs as an image is treated as absent
    // rather than served as `application/octet-stream`.
    let Some((content_type, _)) = crate::routes::media::image_content_type(&bytes) else {
        return err_response(StatusCode::NOT_FOUND, "This user has no avatar.");
    };

    // A strong validator over the content, so a re-upload is picked up
    // immediately while an unchanged avatar costs one 304 per page load.
    // `no-cache` means "revalidate", not "do not store" — the route path is
    // fixed per user, so a cache-busting query string would be the alternative.
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

/// GET /api/users/me/avatar — the calling credential's own stored image.
///
/// # Why this route exists at all
///
/// The dashboard attaches the bearer token by allowlist (`AUTHENTICATED_IMAGE_PATH_RE` in `dashboard/src/api.ts`), and that allowlist admits only `[A-Za-z0-9_-]+` per path segment — deliberately, so that `/` and `.` cannot appear where an id is expected.
/// A **user name** is not such a segment: `encodeURIComponent("Juan Pérez")` is `Juan%20P%C3%A9rez`, and `%` is not in the class, so the image would simply never load.
/// Widening the class to admit `%XX` would readmit `%2F`, which decodes to `/` — the traversal the allowlist exists to prevent — so the fix belongs here rather than in the regex.
///
/// `me` is a literal, so this path has **no client-controlled segment at all**: the subject comes from the credential, exactly as it does for [`crate::routes::authz::whoami`].
/// That is the same property [`librefang_types::media::avatar_path`] documents for the agent route — no part of the path comes from the request — restored for the one caller that cannot express it by name.
///
/// The `{name}` sibling stays, because an Admin legitimately reads someone else's picture.
///
/// # A user literally named `me`
///
/// `matchit` resolves the static segment first, so `/api/users/me/avatar` never reaches the `{name}` route — and because this node is `GET`-only it owns the whole path, which means the `POST` and `DELETE` registered on the sibling do not answer under it either.
/// A `[[users]]` entry named `me` therefore cannot be fetched by an Admin, and cannot be *given* an avatar through the API at all.
///
/// They are not locked out of their own: this route resolves the credential and then calls the same handler, so a person named `me` still sees their own picture, and every non-avatar route on that name (`PATCH /api/users/me/identity`, `PUT /api/users/me`, the policy and provider-key siblings) is unaffected because none of them has a literal namesake.
///
/// The corner is the price of the read path having no client-controlled segment, and it is recorded here rather than papered over.
/// Widening the literal node to carry the write verbs would remove it, at the cost of giving `/users/me/avatar` a second meaning — "the caller's avatar" for writes while `{name}` keeps it for everyone else — which is a larger change than this route needs.
#[utoipa::path(
    get,
    path = "/api/users/me/avatar",
    tag = "users",
    responses(
        (status = 200, description = "The image", content_type = "image/png"),
        (status = 304, description = "Unchanged since the caller's `If-None-Match`"),
        (status = 404, description = "The credential names no user, or that user has no avatar set", body = crate::types::JsonObject)
    )
)]
pub async fn serve_my_avatar(
    State(state): State<Arc<AppState>>,
    api_user: Option<Extension<AuthenticatedApiUser>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let Some(user) = resolve_caller(&state, api_user.as_ref().map(|e| &e.0)) else {
        return err_response(
            StatusCode::NOT_FOUND,
            "This credential names no user, so it has no avatar.",
        );
    };
    // Delegates rather than re-implementing: the two routes differ only in how
    // the subject is found, and a second copy of the read/sniff/ETag body is a
    // second place for the re-sniffing rule to be lost.
    // Going through the handler instead of the router also means a user named
    // `me` still gets their own picture — see the note above.
    serve_user_avatar(State(state), Path(user.name), headers).await
}

/// DELETE /api/users/{name}/avatar — remove the stored image.
#[utoipa::path(
    delete,
    path = "/api/users/{name}/avatar",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    responses(
        (status = 200, description = "Removed", body = crate::types::JsonObject),
        (status = 404, description = "No such user", body = crate::types::JsonObject)
    )
)]
pub async fn delete_user_avatar(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> axum::response::Response {
    let user = match resolve_user(&state, &name) {
        Ok(user) => user,
        Err(response) => return *response,
    };

    let dir = state.kernel.config_snapshot().effective_user_avatars_dir();
    let removed = librefang_types::media::remove_avatars(&dir, &avatar_id(&user));

    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "removed": removed })),
    )
        .into_response()
}

/// PATCH /api/users/{name}/identity — store or clear this user's emoji.
#[utoipa::path(
    patch,
    path = "/api/users/{name}/identity",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    request_body = UserIdentityUpdate,
    responses(
        (status = 200, description = "Identity updated", body = crate::routes::users::UserView),
        (status = 400, description = "Validation error", body = crate::types::JsonObject),
        (status = 404, description = "No such user", body = crate::types::JsonObject),
        (status = 423, description = "The config file is deployment-managed", body = crate::types::JsonObject)
    )
)]
pub async fn update_user_identity(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    caller: Option<Extension<AuthenticatedApiUser>>,
    Json(req): Json<UserIdentityUpdate>,
) -> axum::response::Response {
    let emoji = match librefang_types::config::validate_emoji(req.emoji.as_deref()) {
        Ok(emoji) => emoji,
        Err(error) => return err_response(StatusCode::BAD_REQUEST, error),
    };

    let target = name.clone();
    let caller_uid = caller.as_ref().map(|c| c.0.user_id);
    let outcome = persist_identity_sections(
        &state,
        caller_uid,
        "user identity updated",
        move |users, _groups| -> Result<UserConfig, PersistError> {
            let idx = users
                .iter()
                .position(|u| u.name == target)
                .ok_or_else(|| PersistError::NotFound(format!("user '{target}' not found")))?;
            // Only the emoji is touched: every other field on the row is
            // carried through untouched, so a glyph edit cannot reset a
            // rename the operator did not make, a role they set, or a policy
            // the permission matrix owns.
            users[idx].emoji = emoji.clone();
            Ok(users[idx].clone())
        },
    )
    .await;

    match outcome {
        Ok(final_cfg) => {
            let dir = state.kernel.config_snapshot().effective_user_avatars_dir();
            (
                StatusCode::OK,
                Json(super::UserView::from_config(&final_cfg, &dir)),
            )
                .into_response()
        }
        Err(PersistError::BadRequest(m)) => err_response(StatusCode::BAD_REQUEST, m),
        Err(PersistError::Conflict(m)) => err_response(StatusCode::CONFLICT, m),
        Err(PersistError::NotFound(m)) => err_response(StatusCode::NOT_FOUND, m),
        Err(PersistError::Internal(m)) => super::internal_err_response(m),
        Err(PersistError::Managed) => {
            crate::routes::managed_config_response(state.kernel.config_path())
        }
    }
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

    #[test]
    fn the_extractor_limit_leaves_room_above_the_handler_cap() {
        // If these were equal the extractor would answer first and the
        // handler's 413 — the one that names the cap — would be unreachable.
        const {
            assert!(
                USER_AVATAR_BODY_LIMIT_BYTES > MAX_AVATAR_BYTES,
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
            USER_AVATAR_BODY_LIMIT_BYTES < global_default,
            "avatar cap {USER_AVATAR_BODY_LIMIT_BYTES} must stay under the global {global_default}"
        );
    }

    /// The stem handed to the filesystem is derived, never the configured name.
    ///
    /// This is the property the whole module rests on, so it is asserted on the
    /// two shapes of name that would be dangerous if it ever stopped holding:
    /// a traversal, and the Windows device names that a `.png` suffix does not
    /// make safe.
    #[test]
    fn the_file_stem_is_a_uuid_and_never_the_name() {
        for name in [
            "../../etc/cron.d/evil",
            "CON",
            "nul",
            "Alice",
            "a/b\\c",
            "..",
        ] {
            let user = UserConfig {
                name: name.to_string(),
                ..Default::default()
            };
            let id = avatar_id(&user);
            assert_eq!(
                id,
                UserId::from_name(name).to_string(),
                "{name:?} must map to its stable derived id"
            );
            assert!(
                uuid::Uuid::parse_str(&id).is_ok(),
                "{name:?} produced {id:?}, which is not a uuid"
            );
            // Parse alone would accept a hyphenated hex string that came from
            // somewhere else; the equality above is what pins the namespace.
            assert!(
                !id.contains('/') && !id.contains('\\') && !id.contains(".."),
                "{name:?} leaked into the file stem {id:?}"
            );
        }
    }

    /// The stem is the *documented* derivation, not merely some uuid.
    ///
    /// Pinned explicitly because the mapping is the thing that has to stay
    /// stable: `UserId::from_name` promises "same name, same id, across
    /// restarts and reloads", and a swap of the namespace constant would keep
    /// this module compiling and every traversal assertion passing while
    /// orphaning every avatar already on disk.
    #[test]
    fn the_file_stem_uses_the_frozen_user_namespace() {
        let name = "Alice";
        assert_eq!(
            UserId::from_name(name).0,
            uuid::Uuid::new_v5(
                &librefang_types::agent::LIBREFANG_USER_NAMESPACE,
                name.as_bytes()
            ),
            "the stem must come from LIBREFANG_USER_NAMESPACE, which is frozen"
        );
        assert_eq!(
            UserId::from_name(name).0.get_version_num(),
            5,
            "v5 is what makes the mapping reproducible rather than random"
        );
    }

    /// A rename moves every candidate to the new stem and frees the old one.
    ///
    /// Both files exist here because a crash between an upload's write and its
    /// clear can leave a shadow candidate; if it stayed under the old stem, the
    /// next user to take the freed name would inherit a stale format.
    #[test]
    fn moving_an_avatar_carries_every_candidate_to_the_new_stem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = UserId::from_name("Alice").to_string();
        let new = UserId::from_name("Alicia").to_string();
        std::fs::write(
            librefang_types::media::avatar_path(dir.path(), &old, "png"),
            b"png",
        )
        .expect("write png");
        std::fs::write(
            librefang_types::media::avatar_path(dir.path(), &old, "gif"),
            b"gif",
        )
        .expect("write gif");

        move_user_avatar(dir.path(), "Alice", "Alicia").expect("move");

        assert_eq!(
            std::fs::read(librefang_types::media::avatar_path(dir.path(), &new, "png"))
                .expect("png at the new stem"),
            b"png"
        );
        assert_eq!(
            std::fs::read(librefang_types::media::avatar_path(dir.path(), &new, "gif"))
                .expect("gif at the new stem"),
            b"gif"
        );
        assert!(
            librefang_types::media::find_avatar(dir.path(), &old).is_none(),
            "nothing may be left under the old stem"
        );
    }

    /// A user who never uploaded one has nothing to move, and that is not an
    /// error: every candidate being absent is the ordinary case for a rename.
    #[test]
    fn moving_an_absent_avatar_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        move_user_avatar(dir.path(), "Alice", "Alicia").expect("nothing to move is not a failure");
    }

    /// A real disk failure is propagated rather than swallowed, because the
    /// caller reports it: the picture has an owner whose route would otherwise
    /// serve nothing.
    #[test]
    fn a_failed_move_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = UserId::from_name("Alice").to_string();
        let new = UserId::from_name("Alicia").to_string();
        std::fs::write(
            librefang_types::media::avatar_path(dir.path(), &old, "png"),
            b"png",
        )
        .expect("write png");
        // A directory squatting on the target makes `fs::rename` fail with a
        // kind that is not `NotFound` — the shape of every real failure this
        // helper must not mistake for absence.
        std::fs::create_dir(librefang_types::media::avatar_path(dir.path(), &new, "png"))
            .expect("squat the target");

        let error = move_user_avatar(dir.path(), "Alice", "Alicia").expect_err("must fail");
        assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn the_user_avatar_directory_is_a_subdirectory_of_the_agent_one() {
        let config = librefang_types::config::KernelConfig {
            home_dir: std::path::PathBuf::from("/srv/librefang"),
            ..librefang_types::config::KernelConfig::default()
        };
        let avatars = config.effective_avatars_dir();
        let users = config.effective_user_avatars_dir();
        assert!(
            users.starts_with(&avatars) && users != avatars,
            "user avatars must live below the avatars root, not in it: {users:?}"
        );
        // And therefore outside every tree the parent was chosen to avoid.
        assert!(!users.starts_with(config.effective_workspaces_dir()));
        assert!(!users.starts_with(config.home_dir.join("dashboard")));
    }

    #[test]
    fn the_etag_tracks_the_content() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert_eq!(content_hash(png), content_hash(png));
        let mut changed = png.to_vec();
        changed.push(0);
        assert_ne!(content_hash(png), content_hash(&changed));
    }
}
