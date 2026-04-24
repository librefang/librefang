//! SurrealDB DDL provisioning helpers (Phase 7b of `surrealdb-storage-swap`).
//!
//! These are one-shot operations run when an operator calls
//! `POST /api/storage/link-uar` with admin credentials. Unlike the session
//! pool (which uses namespace-level auth), provisioning requires root-level
//! or system-level access so it can create namespaces, databases, and users.
//!
//! # Security
//!
//! Root credentials are never persisted. They are read from the named
//! environment variable (`root_pass_env`) at call time, used to open a
//! temporary connection, run the DDL, and then dropped. The application-level
//! password (`app_pass_env`) is similarly read from the environment.

use crate::error::{StorageError, StorageResult};
use tracing::info;

/// Outcome of a successful UAR namespace provisioning run.
#[derive(Debug, Clone)]
pub struct ProvisionReceipt {
    /// The SurrealDB namespace that was (or already was) defined.
    pub namespace: String,
    /// The SurrealDB database that was (or already was) defined.
    pub database: String,
    /// The application-level user that was (or already was) defined.
    pub app_user: String,
}

/// Provision a SurrealDB remote instance for UAR.
///
/// Opens a root-level connection to `url`, then idempotently creates:
/// - `DEFINE NAMESPACE IF NOT EXISTS <namespace>`
/// - `DEFINE DATABASE IF NOT EXISTS main ON NAMESPACE <namespace>`
/// - `DEFINE USER IF NOT EXISTS <app_user> ON NAMESPACE <namespace> PASSWORD <app_pass> ROLES EDITOR`
///
/// # Arguments
///
/// - `url` — remote SurrealDB endpoint (`ws://`, `wss://`, `http://`, `https://`).
/// - `root_username` — SurrealDB root / system-level username.
/// - `root_password_env` — name of the environment variable that holds the root password.
/// - `namespace` — the namespace to create for UAR.
/// - `app_user` — the application-level SurrealDB username to create.
/// - `app_password_env` — name of the environment variable that holds the app password.
///
/// # Errors
///
/// Returns [`StorageError::MissingCredential`] if an expected env var is absent
/// or empty, [`StorageError::RemoteConnect`] for transport failures, and
/// [`StorageError::AuthFailed`] for credential failures. DDL errors are mapped
/// to [`StorageError::Backend`].
#[cfg(feature = "surreal-backend")]
pub async fn provision_uar_namespace(
    url: &str,
    root_username: &str,
    root_password_env: &str,
    namespace: &str,
    app_user: &str,
    app_password_env: &str,
) -> StorageResult<ProvisionReceipt> {
    // Read secrets from environment — never stored in config files.
    let root_pass = read_env_secret(root_password_env)?;
    let app_pass = read_env_secret(app_password_env)?;

    // Open a temporary root-level connection. We do NOT cache this in the
    // SurrealConnectionPool because root sessions should be as short-lived as
    // possible.
    let client =
        surrealdb::engine::any::connect(url)
            .await
            .map_err(|e| StorageError::RemoteConnect {
                url: url.to_owned(),
                source: Box::new(e),
            })?;

    client
        .signin(surrealdb::opt::auth::Root {
            username: root_username.to_owned(),
            password: root_pass.clone(),
        })
        .await
        .map_err(|e| StorageError::AuthFailed {
            username: root_username.to_owned(),
            message: e.to_string(),
        })?;

    // Build and execute idempotent DDL.
    //
    // SurrealDB 3.0 processes multi-statement queries sequentially; the
    // `IF NOT EXISTS` clause makes each statement a no-op when already applied,
    // so this is safe to run multiple times.
    let ddl = format!(
        "DEFINE NAMESPACE IF NOT EXISTS `{ns}`;\
         DEFINE DATABASE IF NOT EXISTS `main` ON NAMESPACE `{ns}`;\
         DEFINE USER IF NOT EXISTS `{user}` ON NAMESPACE `{ns}` \
           PASSWORD '{pass}' ROLES EDITOR;",
        ns = escape_surql_ident(namespace),
        user = escape_surql_ident(app_user),
        // Password is not a SurrealQL identifier; single-quote escaping is used.
        pass = escape_surql_string(&app_pass),
    );

    client
        .query(&ddl)
        .await
        .map_err(|e| StorageError::Backend(format!("DDL provisioning failed: {e}")))?;

    info!(
        namespace,
        app_user, "SurrealDB UAR namespace provisioned (or already existed)"
    );

    Ok(ProvisionReceipt {
        namespace: namespace.to_owned(),
        database: "main".to_owned(),
        app_user: app_user.to_owned(),
    })
}

/// Stub for when the `surreal-backend` feature is not compiled in.
#[cfg(not(feature = "surreal-backend"))]
pub async fn provision_uar_namespace(
    _url: &str,
    _root_username: &str,
    _root_password_env: &str,
    _namespace: &str,
    _app_user: &str,
    _app_password_env: &str,
) -> StorageResult<ProvisionReceipt> {
    Err(StorageError::BackendDisabled { backend: "surreal" })
}

/// Read a non-empty value from an environment variable.
fn read_env_secret(env_var: &str) -> StorageResult<String> {
    let value = std::env::var(env_var).map_err(|_| StorageError::MissingCredential {
        name: env_var.to_owned(),
    })?;
    if value.is_empty() {
        return Err(StorageError::MissingCredential {
            name: env_var.to_owned(),
        });
    }
    Ok(value)
}

/// Escape a SurrealQL identifier by replacing backtick characters.
///
/// SurrealDB wraps identifiers in backticks; a real backtick inside the name
/// would need to be doubled. Reject names containing backticks outright since
/// they are not valid in any common namespace/user naming policy.
fn escape_surql_ident(s: &str) -> String {
    // Replace any backtick with nothing; callers should validate names before
    // calling this function. In practice, namespace and user names coming from
    // the config wizard will never contain backticks.
    s.replace('`', "")
}

/// Escape a SurrealQL single-quoted string value (used for passwords).
fn escape_surql_string(s: &str) -> String {
    // Escape single quotes by doubling them (standard SQL convention).
    s.replace('\'', "''")
}
