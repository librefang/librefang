//! Declarative resource provisioning (#6695).
//!
//! Managed configuration mode locks `config.toml` as a whole: the deployment owns the file and every API route that would persist into it answers `423 Locked`.
//! That contract stops at the kernel configuration, and it is all-or-nothing by design — see `docs/operations/managed-config.md`.
//!
//! This module is the other half of the RFC: resources that live *outside* `config.toml` — agents today — declared in a deployment-owned directory tree, reconciled at boot, and locked individually while everything an operator creates at runtime stays mutable.
//!
//! ```text
//! /etc/librefang/provisioning/
//!   agents/
//!     researcher.toml
//!     triager.toml
//! ```
//!
//! # Why an environment variable rather than a `KernelConfig` field
//!
//! The root is read from `LIBREFANG_PROVISIONING_PATH`, never from `config.toml`, for exactly the reason [`crate::config::config_mode`] is read from the environment: a setting that decides what may be written must not itself be writable through the surface it governs.
//! A `[provisioning] path = …` key would be settable from the dashboard in mutable mode, and a write that turns provisioning off is a write that unlocks every provisioned resource.
//! Keeping it in the environment also means the Kubernetes manifest states the intent where an operator reads it, alongside `LIBREFANG_CONFIG_PATH` and `LIBREFANG_CONFIG_MODE`.
//!
//! Unset means the feature is off, which is what every existing installation gets.
//!
//! # Reconcile semantics
//!
//! Each `*.toml` under `<root>/agents/` is one resource, identified by its manifest's `name` — not by its filename, which is documentation.
//! A reconcile is idempotent: rebooting with an unchanged tree performs no writes and logs nothing beyond a single summary line.
//!
//! Applying a resource that already exists under the same name **adopts** it: the manifest is replaced, and the agent's id, session, history and identity survive.
//! This is `kubectl apply` semantics, and it is deliberately not a conflict — the deployment declaring an agent named `researcher` is the deployment claiming `researcher`, whether or not something created it first.
//! Adoption is logged so the takeover is visible in the boot log.
//!
//! An **orphan** is a resource this daemon provisioned before whose file is no longer in the tree.
//! [`PrunePolicy`] decides what happens to it, and the default keeps data: the agent survives and is simply released back to runtime ownership, so the operator's next dashboard edit works.
//! Releasing rather than remembering is what makes the removal reversible — putting the file back re-adopts the same agent instead of colliding with a tombstone.
//!
//! # What is not here
//!
//! Channels and workflows are named in the RFC's provisioning tree and are not implemented.
//! Both persist through surfaces this reconcile does not model — channels into `config.toml` itself (already covered by the whole-config lock) and workflows into a SQLite-backed registry with its own run state — so neither is a matter of pointing the same scan at another subdirectory.
//! An unrecognised subdirectory under the root is reported as a failure rather than ignored, so a deployment that tries `provisioning/channels/` is told it is unsupported instead of silently provisioning nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Env var that points the daemon at a deployment-owned provisioning tree.
///
/// Unset, empty, or whitespace-only means provisioning is off.
pub const PROVISIONING_PATH_ENV: &str = "LIBREFANG_PROVISIONING_PATH";

/// Env var that selects the [`PrunePolicy`]. Only the exact value `delete` removes anything.
pub const PROVISIONING_PRUNE_ENV: &str = "LIBREFANG_PROVISIONING_PRUNE";

/// Subdirectory of the provisioning root that holds agent manifests.
pub const AGENTS_SUBDIR: &str = "agents";

/// Every subdirectory name the reconcile understands.
///
/// Anything else under the root is reported as a failure — see the module docs.
pub const KNOWN_SUBDIRS: &[&str] = &[AGENTS_SUBDIR];

/// File name of the provisioning state, resolved under `LIBREFANG_HOME`.
///
/// It lives with the mutable runtime state rather than next to the provisioning tree, which is read-only by construction.
pub const STATE_FILE: &str = "provisioning-state.json";

/// What happens to a resource this daemon provisioned before whose file has since left the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrunePolicy {
    /// Default. Release the resource: drop its provenance, leave it running and editable, warn once.
    #[default]
    Keep,
    /// Delete the resource. For an agent this is a full `kill_agent`, the same teardown `DELETE /api/agents/{id}` performs.
    Delete,
}

impl PrunePolicy {
    /// The stable wire string used by the API and the docs.
    pub fn as_str(self) -> &'static str {
        match self {
            PrunePolicy::Keep => "keep",
            PrunePolicy::Delete => "delete",
        }
    }
}

/// Resolve the prune policy from the environment.
///
/// Anything other than a case-insensitive `delete` — unset, empty, a typo — resolves to [`PrunePolicy::Keep`].
/// A typo must never be the reason an agent is deleted, so the destructive branch requires the exact word.
pub fn prune_policy() -> PrunePolicy {
    match std::env::var(PROVISIONING_PRUNE_ENV) {
        Ok(v) if v.trim().eq_ignore_ascii_case("delete") => PrunePolicy::Delete,
        _ => PrunePolicy::Keep,
    }
}

/// The provisioning root, or `None` when the feature is off.
pub fn provisioning_root() -> Option<PathBuf> {
    let raw = std::env::var(PROVISIONING_PATH_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Where the provisioning state is persisted for a given LibreFang home.
pub fn state_path(home_dir: &Path) -> PathBuf {
    home_dir.join(STATE_FILE)
}

/// The kind of resource a provisioning file declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    /// An agent manifest, the same shape as `agent.toml`.
    Agent,
}

impl ResourceKind {
    /// The stable wire string, also the first segment of a [`ResourceProvenance::key`].
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceKind::Agent => "agent",
        }
    }
}

/// Who provisioned a resource, from where, and at which revision.
///
/// Persisted to [`STATE_FILE`] so the next boot can tell "this file is unchanged" from "this resource was never provisioned", which is the difference between a no-op and an adoption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceProvenance {
    /// Resource kind. Only [`ResourceKind::Agent`] today.
    pub kind: ResourceKind,
    /// Stable identifier within the kind — the manifest's `name`.
    pub name: String,
    /// Absolute path of the declaring file, as it was at apply time.
    pub source: String,
    /// `sha256:<hex>` over the declaring file's bytes at apply time.
    pub checksum: String,
    /// RFC 3339 timestamp of the reconcile that last created or updated this resource.
    pub applied_at: String,
}

impl ResourceProvenance {
    /// `<kind>/<name>` — the state-file key and the identity the reconcile plans against.
    pub fn key(&self) -> String {
        resource_key(self.kind, &self.name)
    }
}

/// Build the `<kind>/<name>` key without materialising a [`ResourceProvenance`].
pub fn resource_key(kind: ResourceKind, name: &str) -> String {
    format!("{}/{}", kind.as_str(), name)
}

/// One file the reconcile could not turn into a resource.
///
/// A failure is per-file: the rest of the tree still applies, which is the resource-level form of the RFC's requirement that invalid externally supplied configuration never partially replaces what is already in effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvisioningFailure {
    /// Absolute path of the offending file or directory.
    pub source: String,
    /// Operator-facing reason, already formatted.
    pub error: String,
}

/// The persisted record of everything this daemon provisions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisioningState {
    /// Keyed by `<kind>/<name>`. A `BTreeMap` so the file is byte-stable across runs.
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceProvenance>,
}

impl ProvisioningState {
    /// Read the state file, or return an empty state.
    ///
    /// A missing file is the normal first-boot case and is silent.
    /// An unreadable or malformed one warns and returns empty, which degrades to "adopt everything again" — idempotent, and strictly better than refusing to boot over a bookkeeping file.
    pub fn load(path: &Path) -> Self {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    "Failed to read provisioning state ({e}); treating every provisioned resource as new"
                );
                return Self::default();
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    "Provisioning state is malformed ({e}); treating every provisioned resource as new"
                );
                Self::default()
            }
        }
    }

    /// Write the state file, creating the parent directory if needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Whether a resource is currently owned by the deployment.
    pub fn owns(&self, kind: ResourceKind, name: &str) -> bool {
        self.resources.contains_key(&resource_key(kind, name))
    }

    /// The provenance record for a resource, if the deployment owns it.
    pub fn get(&self, kind: ResourceKind, name: &str) -> Option<&ResourceProvenance> {
        self.resources.get(&resource_key(kind, name))
    }
}

/// One agent the tree declares, parsed and hashed.
#[derive(Debug, Clone)]
pub struct DesiredAgent {
    /// The manifest's `name`, which is the resource identifier.
    pub name: String,
    /// Absolute path of the declaring file.
    pub source: PathBuf,
    /// `sha256:<hex>` over the declaring file's bytes.
    pub checksum: String,
    /// The parsed manifest, ready to spawn or apply.
    pub manifest: librefang_types::agent::AgentManifest,
}

/// `sha256:<hex>` over a byte slice, in the same format [`crate::config::config_provenance`] uses.
pub fn checksum_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// `sha256:<hex>` over a file's current bytes, or `None` when it cannot be read.
pub fn checksum_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| checksum_bytes(&b))
}

/// Read `<root>/agents/*.toml` into desired resources, plus one failure per file that could not be used.
///
/// Files are visited in sorted order so two hosts with the same tree produce the same log and the same duplicate-name winner (#3298's determinism rule applies to anything an operator diffs across machines, not only to prompt bytes).
/// Subdirectories of the agents directory are ignored — an agent is one file.
pub fn scan_agents(root: &Path) -> (Vec<DesiredAgent>, Vec<ProvisioningFailure>) {
    let dir = root.join(AGENTS_SUBDIR);
    let mut desired: Vec<DesiredAgent> = Vec::new();
    let mut failures: Vec<ProvisioningFailure> = Vec::new();

    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (desired, failures),
        Err(e) => {
            failures.push(ProvisioningFailure {
                source: dir.display().to_string(),
                error: format!("cannot read the provisioning agents directory: {e}"),
            });
            return (desired, failures);
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .map(|ext| ext.eq_ignore_ascii_case("toml"))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();

    let mut seen: BTreeSet<String> = BTreeSet::new();
    for path in paths {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                failures.push(ProvisioningFailure {
                    source: path.display().to_string(),
                    error: format!("cannot read the file: {e}"),
                });
                continue;
            }
        };
        let text = match String::from_utf8(bytes.clone()) {
            Ok(text) => text,
            Err(_) => {
                failures.push(ProvisioningFailure {
                    source: path.display().to_string(),
                    error: "the file is not valid UTF-8".to_string(),
                });
                continue;
            }
        };
        let mut manifest: librefang_types::agent::AgentManifest = match toml::from_str(&text) {
            Ok(manifest) => manifest,
            Err(e) => {
                failures.push(ProvisioningFailure {
                    source: path.display().to_string(),
                    error: format!("not a valid agent manifest: {e}"),
                });
                continue;
            }
        };
        // `AgentManifest::name` has a serde default of `"unnamed"`, so a manifest that never
        // declares one deserialises perfectly and would be provisioned as an agent called
        // `unnamed`. Here the name is the resource identity, so it has to be present in the file
        // rather than supplied by the deserialiser — otherwise `nmae = "researcher"` provisions
        // `unnamed`, and a second such typo collides with the first for no visible reason.
        let declared_name = toml::from_str::<toml::Table>(&text)
            .ok()
            .and_then(|t| t.get("name").and_then(|v| v.as_str()).map(str::to_string));
        let name = match declared_name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => {
                failures.push(ProvisioningFailure {
                    source: path.display().to_string(),
                    error:
                        "the manifest declares no `name`, which is the resource identifier — add a \
                         top-level `name = \"…\"`"
                            .to_string(),
                });
                continue;
            }
        };
        if !seen.insert(name.clone()) {
            failures.push(ProvisioningFailure {
                source: path.display().to_string(),
                error: format!(
                    "another file in this directory already declares the agent `{name}`; \
                     the resource identifier is the manifest `name`, not the file name"
                ),
            });
            continue;
        }
        // The resource key is the trimmed name, and `find_by_name` matches on the manifest's
        // own field, so the two must be the same string or a reconcile would create a second
        // agent every boot.
        manifest.name.clone_from(&name);
        desired.push(DesiredAgent {
            name,
            source: path.clone(),
            checksum: checksum_bytes(&bytes),
            manifest,
        });
    }

    (desired, failures)
}

/// Report a failure for every subdirectory of the root the reconcile does not understand.
///
/// The RFC's tree names `channels/` and `workflows/` alongside `agents/`, and neither is implemented.
/// Silently skipping them would let a deployment believe it had provisioned channels; naming them tells the operator the truth at boot.
pub fn unsupported_subdirs(root: &Path) -> Vec<ProvisioningFailure> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .map(|n| (n.to_string(), e.path()))
                .filter(|(n, _)| !KNOWN_SUBDIRS.contains(&n.as_str()))
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|(name, path)| ProvisioningFailure {
            source: path.display().to_string(),
            error: format!(
                "`{name}` is not a supported provisioning resource kind; \
                 this release provisions `{AGENTS_SUBDIR}` only"
            ),
        })
        .collect()
}

/// What the reconcile decided to do with one resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The resource does not exist and will be created from the file.
    Create { key: String },
    /// The resource exists and its declaration changed, or it exists but was never provisioned (an adoption).
    Apply { key: String, adopted: bool },
    /// The resource exists, was provisioned before, and its file is byte-identical. Nothing happens.
    Unchanged { key: String },
    /// The resource was provisioned before, its file is gone, and [`PrunePolicy::Delete`] is in force.
    Prune { key: String },
    /// The resource was provisioned before, its file is gone, and [`PrunePolicy::Keep`] is in force. Ownership is dropped; the resource survives.
    Release { key: String },
}

impl Action {
    /// The `<kind>/<name>` this action concerns.
    pub fn key(&self) -> &str {
        match self {
            Action::Create { key }
            | Action::Apply { key, .. }
            | Action::Unchanged { key }
            | Action::Prune { key }
            | Action::Release { key } => key,
        }
    }
}

/// Decide, without touching anything, what a reconcile would do.
///
/// Split out from the kernel so the whole decision table is testable against plain data: `desired` is `(key, checksum)` in the order the resources should be applied, `previous` is the state file, and `live` answers "does a resource with this key exist right now".
///
/// The returned actions are in desired order followed by orphan handling in key order, so the plan is deterministic for a given input.
pub fn plan(
    desired: &[(String, String)],
    previous: &ProvisioningState,
    live: impl Fn(&str) -> bool,
    prune: PrunePolicy,
) -> Vec<Action> {
    let mut actions = Vec::with_capacity(desired.len() + previous.resources.len());
    let mut declared: BTreeSet<&str> = BTreeSet::new();

    for (key, checksum) in desired {
        declared.insert(key.as_str());
        let owned = previous.resources.get(key);
        let exists = live(key);
        match (exists, owned) {
            // Provisioned before, still here, and the file has not changed.
            (true, Some(prev)) if prev.checksum == *checksum => {
                actions.push(Action::Unchanged { key: key.clone() })
            }
            // Provisioned before and the file changed.
            (true, Some(_)) => actions.push(Action::Apply {
                key: key.clone(),
                adopted: false,
            }),
            // Exists but the deployment never claimed it — `kubectl apply` semantics.
            (true, None) => actions.push(Action::Apply {
                key: key.clone(),
                adopted: true,
            }),
            // Gone, or never existed. Either way the file is the source of truth.
            (false, _) => actions.push(Action::Create { key: key.clone() }),
        }
    }

    for key in previous.resources.keys() {
        if declared.contains(key.as_str()) {
            continue;
        }
        actions.push(match prune {
            PrunePolicy::Delete => Action::Prune { key: key.clone() },
            PrunePolicy::Keep => Action::Release { key: key.clone() },
        });
    }

    actions
}

/// Counts from one reconcile, for the boot log and the status endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProvisioningReport {
    /// Resources created because nothing by that name existed.
    pub created: usize,
    /// Resources whose declaration was applied over an existing resource.
    pub applied: usize,
    /// Of `applied`, how many were taken over from runtime ownership.
    pub adopted: usize,
    /// Resources left alone because their file is byte-identical to the last apply.
    pub unchanged: usize,
    /// Orphans deleted under [`PrunePolicy::Delete`].
    pub pruned: usize,
    /// Orphans released under [`PrunePolicy::Keep`].
    pub released: usize,
    /// Files the reconcile could not use, plus unsupported subdirectories.
    pub failed: usize,
}

impl ProvisioningReport {
    /// True when the reconcile changed anything, so the caller can stay silent otherwise.
    pub fn mutated(&self) -> bool {
        self.created > 0 || self.applied > 0 || self.pruned > 0 || self.released > 0
    }
}

/// Everything the daemon knows about provisioning right now.
///
/// Held on the kernel and swapped wholesale by a reconcile, so a reader never sees a half-applied plan.
#[derive(Debug, Clone, Default)]
pub struct ProvisioningRuntime {
    /// The root, or `None` when [`PROVISIONING_PATH_ENV`] is unset.
    pub root: Option<PathBuf>,
    /// Prune policy in force at the last reconcile.
    pub prune: PrunePolicy,
    /// What the deployment currently owns.
    pub state: ProvisioningState,
    /// Failures from the last reconcile.
    pub failures: Vec<ProvisioningFailure>,
    /// Counts from the last reconcile.
    pub report: ProvisioningReport,
    /// RFC 3339 timestamp of the last reconcile, or `None` when none has run.
    pub applied_at: Option<String>,
}

impl ProvisioningRuntime {
    /// Whether provisioning is switched on at all.
    pub fn enabled(&self) -> bool {
        self.root.is_some()
    }
}

/// One resource as the status endpoint reports it.
///
/// `checksum` is what was applied; `source_checksum` is what the file says now.
/// They differ exactly when the tree has moved on without a reconcile, which is the resource-level analogue of the `checksum/config` annotation an operator compares after a ConfigMap edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvisionedResourceStatus {
    /// Resource kind.
    pub kind: ResourceKind,
    /// Resource identifier.
    pub name: String,
    /// Absolute path of the declaring file.
    pub source: String,
    /// Checksum of the declaration that is in effect.
    pub checksum: String,
    /// RFC 3339 timestamp of the reconcile that applied it.
    pub applied_at: String,
    /// Checksum of the declaring file as it is on disk now, or `None` when the file is gone or unreadable.
    pub source_checksum: Option<String>,
    /// `source_checksum` differs from `checksum`, the file having been edited or removed since the last reconcile.
    pub drifted: bool,
    /// The resource still exists in the daemon.
    ///
    /// `false` means something removed it out of band; the next reconcile recreates it.
    pub present: bool,
}

/// The `GET /api/provisioning/status` body.
#[derive(Debug, Clone, Serialize)]
pub struct ProvisioningStatus {
    /// Whether [`PROVISIONING_PATH_ENV`] is set.
    pub enabled: bool,
    /// The provisioning root, or `None` when disabled.
    pub root: Option<String>,
    /// `"keep"` or `"delete"`.
    pub prune: &'static str,
    /// Every owned resource, in `<kind>/<name>` order.
    pub resources: Vec<ProvisionedResourceStatus>,
    /// Files and directories the last reconcile refused, with reasons.
    pub failures: Vec<ProvisioningFailure>,
    /// Counts from the last reconcile.
    pub report: ProvisioningReport,
    /// RFC 3339 timestamp of the last reconcile.
    pub applied_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance(key_name: &str, checksum: &str) -> ResourceProvenance {
        ResourceProvenance {
            kind: ResourceKind::Agent,
            name: key_name.to_string(),
            source: format!("/etc/librefang/provisioning/agents/{key_name}.toml"),
            checksum: checksum.to_string(),
            applied_at: "2026-08-25T00:00:00+00:00".to_string(),
        }
    }

    fn state(entries: &[(&str, &str)]) -> ProvisioningState {
        let mut state = ProvisioningState::default();
        for (name, checksum) in entries {
            let p = provenance(name, checksum);
            state.resources.insert(p.key(), p);
        }
        state
    }

    #[test]
    fn unchanged_declaration_on_a_live_resource_is_a_no_op() {
        let actions = plan(
            &[("agent/a".into(), "sha256:aa".into())],
            &state(&[("a", "sha256:aa")]),
            |_| true,
            PrunePolicy::Keep,
        );
        assert_eq!(
            actions,
            vec![Action::Unchanged {
                key: "agent/a".into()
            }]
        );
    }

    #[test]
    fn changed_declaration_on_an_owned_resource_applies_without_adopting() {
        let actions = plan(
            &[("agent/a".into(), "sha256:bb".into())],
            &state(&[("a", "sha256:aa")]),
            |_| true,
            PrunePolicy::Keep,
        );
        assert_eq!(
            actions,
            vec![Action::Apply {
                key: "agent/a".into(),
                adopted: false
            }]
        );
    }

    #[test]
    fn an_existing_but_unowned_resource_is_adopted_rather_than_duplicated() {
        let actions = plan(
            &[("agent/a".into(), "sha256:aa".into())],
            &ProvisioningState::default(),
            |_| true,
            PrunePolicy::Keep,
        );
        assert_eq!(
            actions,
            vec![Action::Apply {
                key: "agent/a".into(),
                adopted: true
            }]
        );
    }

    #[test]
    fn an_owned_resource_deleted_out_of_band_is_recreated() {
        let actions = plan(
            &[("agent/a".into(), "sha256:aa".into())],
            &state(&[("a", "sha256:aa")]),
            |_| false,
            PrunePolicy::Keep,
        );
        assert_eq!(
            actions,
            vec![Action::Create {
                key: "agent/a".into()
            }]
        );
    }

    #[test]
    fn an_orphan_is_released_under_the_default_policy() {
        let actions = plan(
            &[],
            &state(&[("gone", "sha256:aa")]),
            |_| true,
            PrunePolicy::Keep,
        );
        assert_eq!(
            actions,
            vec![Action::Release {
                key: "agent/gone".into()
            }]
        );
    }

    #[test]
    fn an_orphan_is_deleted_only_under_the_delete_policy() {
        let actions = plan(
            &[],
            &state(&[("gone", "sha256:aa")]),
            |_| true,
            PrunePolicy::Delete,
        );
        assert_eq!(
            actions,
            vec![Action::Prune {
                key: "agent/gone".into()
            }]
        );
    }

    #[test]
    fn orphan_handling_follows_declared_resources_in_key_order() {
        let actions = plan(
            &[("agent/keep".into(), "sha256:aa".into())],
            &state(&[
                ("keep", "sha256:aa"),
                ("z", "sha256:zz"),
                ("b", "sha256:bb"),
            ]),
            |_| true,
            PrunePolicy::Delete,
        );
        let keys: Vec<&str> = actions.iter().map(|a| a.key()).collect();
        assert_eq!(keys, vec!["agent/keep", "agent/b", "agent/z"]);
    }

    #[test]
    fn the_destructive_prune_policy_is_never_reached_by_a_typo() {
        // Pins the resolution rule rather than the env read, which is process-global and
        // would force this file's tests serial for one assertion.
        let resolve = |raw: &str| {
            if raw.trim().eq_ignore_ascii_case("delete") {
                PrunePolicy::Delete
            } else {
                PrunePolicy::Keep
            }
        };
        assert_eq!(resolve("delete"), PrunePolicy::Delete);
        assert_eq!(resolve(" DELETE "), PrunePolicy::Delete);
        for typo in ["", " ", "del", "deleted", "true", "prune"] {
            assert_eq!(resolve(typo), PrunePolicy::Keep, "input {typo:?}");
        }
        assert_eq!(PrunePolicy::default(), PrunePolicy::Keep);
    }

    #[test]
    fn scan_reads_manifests_in_sorted_order_and_keys_them_by_manifest_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents = dir.path().join(AGENTS_SUBDIR);
        std::fs::create_dir_all(&agents).expect("mkdir");
        // File name deliberately unrelated to the manifest name.
        std::fs::write(
            agents.join("02-second.toml"),
            "name = \"beta\"\nmodule = \"builtin:chat\"\n",
        )
        .expect("write");
        std::fs::write(
            agents.join("01-first.toml"),
            "name = \"alpha\"\nmodule = \"builtin:chat\"\n",
        )
        .expect("write");
        // Non-TOML files are not resources.
        std::fs::write(agents.join("README.md"), "ignore me").expect("write");

        let (desired, failures) = scan_agents(dir.path());
        assert!(failures.is_empty(), "{failures:?}");
        let names: Vec<&str> = desired.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert!(desired[0].checksum.starts_with("sha256:"));
    }

    #[test]
    fn scan_reports_one_failure_per_unusable_file_and_keeps_the_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents = dir.path().join(AGENTS_SUBDIR);
        std::fs::create_dir_all(&agents).expect("mkdir");
        std::fs::write(
            agents.join("good.toml"),
            "name = \"good\"\nmodule = \"builtin:chat\"\n",
        )
        .expect("write");
        std::fs::write(agents.join("broken.toml"), "name = \"oops\n").expect("write");
        std::fs::write(agents.join("nameless.toml"), "module = \"builtin:chat\"\n").expect("write");

        let (desired, failures) = scan_agents(dir.path());
        assert_eq!(
            desired.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["good"],
            "one bad file must not take the good ones down with it"
        );
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(failures.iter().any(|f| f.source.ends_with("broken.toml")));
        assert!(failures
            .iter()
            .any(|f| f.source.ends_with("nameless.toml") && f.error.contains("`name`")));
    }

    #[test]
    fn two_files_declaring_the_same_agent_name_keep_the_first_and_report_the_second() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents = dir.path().join(AGENTS_SUBDIR);
        std::fs::create_dir_all(&agents).expect("mkdir");
        std::fs::write(
            agents.join("a.toml"),
            "name = \"dup\"\nmodule = \"builtin:chat\"\ndescription = \"first\"\n",
        )
        .expect("write");
        std::fs::write(
            agents.join("b.toml"),
            "name = \"dup\"\nmodule = \"builtin:chat\"\ndescription = \"second\"\n",
        )
        .expect("write");

        let (desired, failures) = scan_agents(dir.path());
        assert_eq!(desired.len(), 1);
        assert_eq!(desired[0].manifest.description, "first");
        assert_eq!(failures.len(), 1);
        assert!(failures[0].source.ends_with("b.toml"));
    }

    #[test]
    fn a_missing_agents_directory_is_not_a_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (desired, failures) = scan_agents(dir.path());
        assert!(desired.is_empty());
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn an_unsupported_subdirectory_is_named_rather_than_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(AGENTS_SUBDIR)).expect("mkdir");
        std::fs::create_dir_all(dir.path().join("channels")).expect("mkdir");
        std::fs::create_dir_all(dir.path().join("workflows")).expect("mkdir");

        let failures = unsupported_subdirs(dir.path());
        let sources: Vec<&str> = failures.iter().map(|f| f.source.as_str()).collect();
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(sources.iter().any(|s| s.ends_with("channels")));
        assert!(sources.iter().any(|s| s.ends_with("workflows")));
        assert!(
            !sources.iter().any(|s| s.ends_with(AGENTS_SUBDIR)),
            "the supported kind must not be reported"
        );
    }

    #[test]
    fn state_round_trips_through_the_file_and_survives_a_malformed_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = state_path(dir.path());
        assert_eq!(ProvisioningState::load(&path), ProvisioningState::default());

        let original = state(&[("a", "sha256:aa"), ("b", "sha256:bb")]);
        original.save(&path).expect("save");
        assert_eq!(ProvisioningState::load(&path), original);
        assert!(original.owns(ResourceKind::Agent, "a"));
        assert!(!original.owns(ResourceKind::Agent, "nope"));

        std::fs::write(&path, b"{not json").expect("write");
        assert_eq!(
            ProvisioningState::load(&path),
            ProvisioningState::default(),
            "a malformed state file must degrade to re-adoption, not to a boot failure"
        );
    }

    #[test]
    fn checksums_match_the_config_provenance_format() {
        assert_eq!(
            checksum_bytes(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
