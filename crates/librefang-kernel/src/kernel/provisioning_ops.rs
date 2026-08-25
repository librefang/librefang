//! Boot-time reconcile of the declarative provisioning tree (#6695).
//!
//! The decision table lives in [`crate::provisioning`] as pure functions over plain data; this module is the part that has to touch the registry.
//! Keeping the split means "what would a reconcile do" is unit-testable without a booted kernel, and this file only has to be right about *how* to create, apply and remove an agent.
//!
//! Sibling submodule of `kernel::mod`, so it reaches `LibreFangKernel`'s private fields directly.

use super::*;
use crate::provisioning::{
    checksum_file, plan, provisioning_root, prune_policy, resource_key, scan_agents, state_path,
    unsupported_subdirs, Action, ProvisionedResourceStatus, ProvisioningFailure,
    ProvisioningReport, ProvisioningRuntime, ProvisioningState, ProvisioningStatus, ResourceKind,
    ResourceProvenance,
};

impl LibreFangKernel {
    /// Reconcile the deployment-owned provisioning tree into the live registry.
    ///
    /// A no-op when [`crate::provisioning::PROVISIONING_PATH_ENV`] is unset, which is every existing installation.
    ///
    /// Called once from `boot_with_config_at`, before the "no agents exist, spawn a default assistant" fallback — a managed deployment that declares its own agents must not also receive an `assistant` it never asked for.
    ///
    /// Never fails the boot. A file the reconcile cannot use is recorded as a failure and reported through `GET /api/provisioning/status`; refusing to start over one malformed manifest would turn an operator's typo into an outage, which is the same trade-off the restored-agent loop already makes for a bad `module` path.
    pub fn apply_provisioning(&self) -> ProvisioningReport {
        let Some(root) = provisioning_root() else {
            self.provisioning
                .store(std::sync::Arc::new(ProvisioningRuntime::default()));
            return ProvisioningReport::default();
        };

        let prune = prune_policy();
        let state_file = state_path(&self.home_dir_boot);
        let previous = ProvisioningState::load(&state_file);

        let (desired, mut failures) = scan_agents(&root);
        failures.extend(unsupported_subdirs(&root));

        let desired_keys: Vec<(String, String)> = desired
            .iter()
            .map(|d| {
                (
                    resource_key(ResourceKind::Agent, &d.name),
                    d.checksum.clone(),
                )
            })
            .collect();

        let actions = plan(
            &desired_keys,
            &previous,
            |key| {
                key.strip_prefix("agent/")
                    .map(|name| self.agents.registry.find_by_name(name).is_some())
                    .unwrap_or(false)
            },
            prune,
        );

        let applied_at = chrono::Utc::now().to_rfc3339();
        let mut next = ProvisioningState::default();
        let mut report = ProvisioningReport {
            failed: failures.len(),
            ..Default::default()
        };

        for action in actions {
            let key = action.key().to_string();
            match action {
                Action::Unchanged { .. } => {
                    report.unchanged += 1;
                    // Carry the previous record forward verbatim: re-stamping `applied_at` on
                    // every boot would make an untouched tree look freshly applied.
                    if let Some(prev) = previous.resources.get(&key) {
                        next.resources.insert(key, prev.clone());
                    }
                }
                Action::Create { .. } | Action::Apply { .. } => {
                    let adopted = matches!(action, Action::Apply { adopted: true, .. });
                    let Some(d) = desired
                        .iter()
                        .find(|d| resource_key(ResourceKind::Agent, &d.name) == key)
                    else {
                        continue;
                    };
                    let outcome = match self.agents.registry.find_by_name(&d.name) {
                        Some(entry) => self
                            .update_manifest(entry.id, d.manifest.clone())
                            .map(|()| entry.id),
                        // Deliberately `spawn_agent`, not `spawn_agent_with_source`: recording the
                        // provisioning file as the agent's `source_toml_path` would point
                        // `persist_manifest_to_disk` at it, and the whole premise of a provisioning
                        // tree is that it is deployment-owned and mounted read-only. The daemon
                        // would then retry a doomed write on every manifest touch — the same
                        // failure mode the managed-mode migration write-back was fixed to avoid.
                        //
                        // The tree is an input. The agent's own workspace `agent.toml` stays the
                        // materialised copy, which is also what makes `drifted` on the status
                        // endpoint mean "the declaration moved" rather than "the file we write".
                        None => self.spawn_agent(d.manifest.clone()),
                    };
                    match outcome {
                        Ok(agent_id) => {
                            if matches!(action, Action::Create { .. }) {
                                report.created += 1;
                            } else {
                                report.applied += 1;
                                if adopted {
                                    report.adopted += 1;
                                }
                            }
                            info!(
                                agent = %d.name,
                                agent_id = %agent_id,
                                source = %d.source.display(),
                                adopted,
                                "Provisioned agent from the deployment tree"
                            );
                            next.resources.insert(
                                key,
                                ResourceProvenance {
                                    kind: ResourceKind::Agent,
                                    name: d.name.clone(),
                                    source: d.source.display().to_string(),
                                    checksum: d.checksum.clone(),
                                    applied_at: applied_at.clone(),
                                },
                            );
                        }
                        Err(e) => {
                            report.failed += 1;
                            let error = format!("failed to apply the agent manifest: {e}");
                            tracing::error!(
                                agent = %d.name,
                                source = %d.source.display(),
                                "{error}"
                            );
                            failures.push(ProvisioningFailure {
                                source: d.source.display().to_string(),
                                error,
                            });
                            // No provenance for a resource that was not applied: an unowned
                            // resource stays editable, which is the recoverable state.
                        }
                    }
                }
                Action::Prune { .. } => {
                    let name = key.strip_prefix("agent/").unwrap_or(&key).to_string();
                    match self.agents.registry.find_by_name(&name) {
                        Some(entry) => match self.kill_agent(entry.id) {
                            Ok(()) => {
                                report.pruned += 1;
                                info!(
                                    agent = %name,
                                    "Pruned provisioned agent — its declaration left the tree"
                                );
                            }
                            Err(e) => {
                                // Keep the provenance so the next boot retries. Dropping it here
                                // would silently release a still-running agent that the deployment
                                // asked to have deleted — the outcome `keep` produces, reached by
                                // a failure rather than by the operator's choice.
                                report.failed += 1;
                                let error = format!("failed to prune the agent: {e}");
                                tracing::error!(agent = %name, "{error}");
                                if let Some(prev) = previous.resources.get(&key) {
                                    failures.push(ProvisioningFailure {
                                        source: prev.source.clone(),
                                        error,
                                    });
                                    next.resources.insert(key, prev.clone());
                                }
                            }
                        },
                        None => {
                            report.pruned += 1;
                            tracing::debug!(
                                agent = %name,
                                "Provisioned agent already gone; nothing to prune"
                            );
                        }
                    }
                }
                Action::Release { .. } => {
                    report.released += 1;
                    tracing::warn!(
                        resource = %key,
                        "Declaration left the provisioning tree; releasing the resource to runtime ownership. \
                         It keeps running and becomes editable again. Set {}=delete to remove it instead.",
                        crate::provisioning::PROVISIONING_PRUNE_ENV
                    );
                }
            }
        }

        if next != previous {
            if let Err(e) = next.save(&state_file) {
                tracing::warn!(
                    path = %state_file.display(),
                    "Failed to persist provisioning state ({e}); the next boot will re-adopt every provisioned resource"
                );
            }
        }

        if report.mutated() || report.failed > 0 {
            info!(
                root = %root.display(),
                created = report.created,
                applied = report.applied,
                adopted = report.adopted,
                unchanged = report.unchanged,
                pruned = report.pruned,
                released = report.released,
                failed = report.failed,
                "Reconciled the declarative provisioning tree"
            );
        }

        self.provisioning
            .store(std::sync::Arc::new(ProvisioningRuntime {
                root: Some(root),
                prune,
                state: next,
                failures,
                report: report.clone(),
                applied_at: Some(applied_at),
            }));

        report
    }

    /// Provenance for a resource the deployment owns, or `None` when it is runtime-owned.
    ///
    /// This is the lookup every write guard performs, so it must stay cheap: one `ArcSwap` load and a `BTreeMap` hit, with no filesystem access.
    pub fn provisioned_resource(
        &self,
        kind: ResourceKind,
        name: &str,
    ) -> Option<ResourceProvenance> {
        self.provisioning.load().state.get(kind, name).cloned()
    }

    /// The `GET /api/provisioning/status` body.
    ///
    /// Re-hashes each declaring file so `drifted` answers "has the tree moved on since the last reconcile", which is what an operator checks after editing a ConfigMap and before deciding whether the rollout landed.
    pub fn provisioning_status(&self) -> ProvisioningStatus {
        let runtime = self.provisioning.load();
        let resources = runtime
            .state
            .resources
            .values()
            .map(|p| {
                let source_checksum = checksum_file(std::path::Path::new(&p.source));
                let present = match p.kind {
                    ResourceKind::Agent => self.agents.registry.find_by_name(&p.name).is_some(),
                };
                ProvisionedResourceStatus {
                    kind: p.kind,
                    name: p.name.clone(),
                    source: p.source.clone(),
                    checksum: p.checksum.clone(),
                    applied_at: p.applied_at.clone(),
                    drifted: source_checksum.as_deref() != Some(p.checksum.as_str()),
                    source_checksum,
                    present,
                }
            })
            .collect();

        ProvisioningStatus {
            enabled: runtime.enabled(),
            root: runtime.root.as_ref().map(|r| r.display().to_string()),
            prune: runtime.prune.as_str(),
            resources,
            failures: runtime.failures.clone(),
            report: runtime.report.clone(),
            applied_at: runtime.applied_at.clone(),
        }
    }
}
