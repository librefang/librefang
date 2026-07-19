//! Per-user LLM provider credentials (#6460).
//!
//! In team deployments an organization wants each human user's agents to consume that user's own upstream LLM provider API key, so upstream usage tracking, quota, and chargeback attach to the human owner rather than to a shared daemon-wide credential.
//!
//! This module stores each user's provider key ENCRYPTED in the existing credential vault (`CredentialVault`, AES-256-GCM, keyed by `LIBREFANG_VAULT_KEY` / OS keyring) under a per-user, per-provider namespace, and defines the resolution precedence used at driver-credential lookup time: a user-scoped key wins over the daemon-global credential.
//!
//! The plaintext key is NEVER returned through the API: only provider *names* are listable (see [`LibreFangKernel::list_user_provider_keys`]); the secret value leaves the vault only on the internal driver-resolution path in `resolve_driver_for_owner`.
//!
//! Scope note (#6460 initial slice): the storage + resolution precedence + per-owner metering rollup (the latter already present via `UsageStore::query_user_*`) land here.
//! Threading the human owner of a turn into `resolve_driver_for_owner` from request context — and the HTTP / dashboard surface to manage these keys — are follow-ups tracked in the PR.

use crate::LibreFangKernel;
use librefang_types::agent::UserId;

/// Vault-key namespace for per-user provider credentials.
///
/// Distinct from the MCP-OAuth (`mcp_oauth/…`) and reserved sentinel (`__sentinel__`) namespaces so the three never collide in the shared `vault.enc`.
pub const USER_PROVIDER_KEY_PREFIX: &str = "user_provider_key";

/// Build the vault key under which user `user_id`'s credential for `provider` is stored: `user_provider_key/<user-uuid>/<provider>`.
///
/// [`UserId`] renders as its stable v5 UUID (derived from the configured user name via [`UserId::from_name`]), so the key is stable across restarts and config reloads for the same user.
pub fn user_provider_vault_key(user_id: UserId, provider: &str) -> String {
    format!("{USER_PROVIDER_KEY_PREFIX}/{user_id}/{provider}")
}

/// Credential resolution precedence (#6460): a user-scoped provider key takes precedence over the daemon-global credential.
///
/// Returning the global credential unchanged when no user-scoped key exists is what keeps global-only behaviour byte-identical for every agent that has no per-user key configured.
pub fn resolve_provider_credential(
    user_scoped: Option<String>,
    global: Option<String>,
) -> Option<String> {
    user_scoped.or(global)
}

/// Extract the provider names a user has stored keys for from a flat list of vault keys, filtering to that user's namespace and returning names only — never values.
///
/// Sorted (and de-duplicated) so the listing is deterministic; the vault's own key iteration order is a `HashMap` and must not leak into any output.
pub(crate) fn provider_names_from_keys<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    user_id: UserId,
) -> Vec<String> {
    let prefix = format!("{USER_PROVIDER_KEY_PREFIX}/{user_id}/");
    let mut names: Vec<String> = keys
        .into_iter()
        .filter_map(|k| k.strip_prefix(prefix.as_str()).map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

impl LibreFangKernel {
    /// Store `api_key` as user `user_id`'s own credential for `provider`, encrypted in the vault. Overwrites any existing value for the pair.
    ///
    /// Reuses [`LibreFangKernel::vault_set`], so the vault is lazily created on first write — no separate crypto is introduced.
    pub fn set_user_provider_key(
        &self,
        user_id: UserId,
        provider: &str,
        api_key: &str,
    ) -> Result<(), String> {
        if provider.trim().is_empty() {
            return Err("provider must not be empty".to_string());
        }
        if api_key.is_empty() {
            return Err("api key must not be empty".to_string());
        }
        self.vault_set(&user_provider_vault_key(user_id, provider), api_key)
    }

    /// Read user `user_id`'s stored credential for `provider`, or `None` when no user-scoped key exists (or the vault is unavailable).
    ///
    /// This is the only path that returns the plaintext value, and it is used solely by the internal driver-resolution path — never by an API response.
    /// Kept `pub(crate)` so the plaintext getter cannot be reached from outside the kernel crate (set/list/remove stay `pub`).
    pub(crate) fn get_user_provider_key(&self, user_id: UserId, provider: &str) -> Option<String> {
        self.vault_get(&user_provider_vault_key(user_id, provider))
    }

    /// Remove user `user_id`'s stored credential for `provider`. Returns whether a value was actually removed.
    pub fn remove_user_provider_key(
        &self,
        user_id: UserId,
        provider: &str,
    ) -> Result<bool, String> {
        let key = user_provider_vault_key(user_id, provider);
        let handle = self.vault_handle()?;
        let mut guard = handle.write().unwrap_or_else(|e| e.into_inner());
        if !guard.is_unlocked() {
            // Vault file never materialised — nothing to remove.
            return Ok(false);
        }
        guard
            .remove(&key)
            .map_err(|e| format!("Vault remove failed: {e}"))
    }

    /// List the provider names user `user_id` has stored a credential for, sorted and without exposing any secret value.
    pub fn list_user_provider_keys(&self, user_id: UserId) -> Vec<String> {
        let handle = match self.vault_handle() {
            Ok(h) => h,
            Err(_) => return Vec::new(),
        };
        let guard = handle.read().unwrap_or_else(|e| e.into_inner());
        if !guard.is_unlocked() {
            return Vec::new();
        }
        provider_names_from_keys(guard.list_keys(), user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_key_is_namespaced_stable_and_isolated() {
        let alice = UserId::from_name("alice");
        let key = user_provider_vault_key(alice, "openai");

        assert!(key.starts_with("user_provider_key/"));
        assert!(key.ends_with("/openai"));
        // Stable for the same (user, provider) pair.
        assert_eq!(
            key,
            user_provider_vault_key(UserId::from_name("alice"), "openai")
        );
        // Isolated across users and across providers.
        assert_ne!(
            key,
            user_provider_vault_key(UserId::from_name("bob"), "openai")
        );
        assert_ne!(key, user_provider_vault_key(alice, "gemini"));
    }

    #[test]
    fn resolution_prefers_user_scoped_over_global() {
        // User-scoped key wins.
        assert_eq!(
            resolve_provider_credential(Some("user".into()), Some("global".into())),
            Some("user".to_string()),
        );
        // No user key → fall back to global (global-only behaviour unchanged).
        assert_eq!(
            resolve_provider_credential(None, Some("global".into())),
            Some("global".to_string()),
        );
        // User key with no global still resolves.
        assert_eq!(
            resolve_provider_credential(Some("user".into()), None),
            Some("user".to_string()),
        );
        // Neither → nothing.
        assert_eq!(resolve_provider_credential(None, None), None);
    }

    #[test]
    fn provider_names_filter_by_user_sort_and_exclude_other_namespaces() {
        let alice = UserId::from_name("alice");
        let bob = UserId::from_name("bob");
        let owned = [
            user_provider_vault_key(alice, "openai"),
            user_provider_vault_key(alice, "anthropic"),
            user_provider_vault_key(bob, "gemini"),
            "__sentinel__".to_string(),
            "mcp_oauth/https://example/access_token".to_string(),
        ];
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();

        let names = provider_names_from_keys(refs, alice);
        // Alice's providers only, sorted; Bob's and the reserved namespaces excluded.
        assert_eq!(names, vec!["anthropic".to_string(), "openai".to_string()]);
    }
}
