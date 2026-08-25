//! Resolution of a workflow step's agent reference (refs #7712).
//!
//! Every workflow driver — the kernel's own `run_workflow`, the operator
//! timeout auto-resolve, the `KernelHandle` workflow runner, the channel
//! bridge, and the API's background/resume/operator drivers — used to carry
//! its own byte-identical `match agent_ref { ById .. ByName .. }` closure.
//! Six copies of one policy is six places to forget a variant, and
//! [`StepAgent::ByType`] is precisely the kind of variant that fails
//! *silently* when a copy is missed: the step resolves to `None` and the run
//! reports a missing agent rather than an unhandled reference.
//!
//! The policy lives here once, in two flavours that differ only in whether
//! they are allowed to mutate:
//!
//! - [`LibreFangKernel::resolve_step_agent`] — used by real runs; find-or-spawn.
//! - [`LibreFangKernel::preview_step_agent`] — used by dry runs; never spawns.

use super::*;
use crate::agent_template::{load_agent_template, TemplateLoadError};

/// Why a [`StepAgent::ByType`] reference could not be resolved to a live agent.
///
/// The workflow engine's resolver signature is `-> Option<...>`, so this error
/// is logged rather than returned to the run; keeping it typed means the log
/// line carries the operator-actionable reason instead of a formatted string
/// assembled at the failure site.
#[derive(Debug)]
pub enum StepAgentResolveError {
    /// The template backing the type could not be loaded.
    Template(TemplateLoadError),
    /// The template loaded but the spawn failed for a reason other than an
    /// agent of that name already existing.
    Spawn { requested: String, detail: String },
    /// The spawn lost a name race, but the winning entry vanished before it
    /// could be re-read. Vanishingly rare (a concurrent `kill_agent`), and
    /// distinct from a spawn failure so it is not misread as a bad manifest.
    RaceLost { requested: String },
}

impl std::fmt::Display for StepAgentResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Template(e) => write!(f, "{e}"),
            Self::Spawn { requested, detail } => {
                write!(f, "failed to spawn agent type '{requested}': {detail}")
            }
            Self::RaceLost { requested } => write!(
                f,
                "agent type '{requested}' lost a spawn race and the winning agent was gone \
                 before it could be reused"
            ),
        }
    }
}

impl std::error::Error for StepAgentResolveError {}

impl StepAgentResolveError {
    /// Short, stable discriminator for structured logs.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Template(e) => e.kind(),
            Self::Spawn { .. } => "spawn_failed",
            Self::RaceLost { .. } => "race_lost",
        }
    }
}

impl From<TemplateLoadError> for StepAgentResolveError {
    fn from(e: TemplateLoadError) -> Self {
        Self::Template(e)
    }
}

impl LibreFangKernel {
    /// Look up a registry agent by name and project it into the resolver tuple.
    fn registry_agent_by_name(&self, name: &str) -> Option<(AgentId, String, bool)> {
        let entry = self.agents.registry.find_by_name(name)?;
        Some((
            entry.id,
            entry.name.clone(),
            entry.manifest.inherit_parent_context,
        ))
    }

    /// Look up a registry agent by its stringified UUID.
    fn registry_agent_by_id(&self, id: &str) -> Option<(AgentId, String, bool)> {
        let agent_id: AgentId = id.parse().ok()?;
        let entry = self.agents.registry.get(agent_id)?;
        Some((
            agent_id,
            entry.name.clone(),
            entry.manifest.inherit_parent_context,
        ))
    }

    /// Resolve a workflow step's agent reference for a real run.
    ///
    /// Returns `(agent id, agent name, inherit_parent_context)`, matching the
    /// `agent_resolver` contract the workflow engine expects.
    /// `None` means the step cannot run; `format_missing_agent_error` renders
    /// the run-visible message and the daemon log carries the specific cause.
    pub fn resolve_step_agent(&self, agent_ref: &StepAgent) -> Option<(AgentId, String, bool)> {
        match agent_ref {
            StepAgent::ById { id } => self.registry_agent_by_id(id),
            StepAgent::ByName { name } => self.registry_agent_by_name(name),
            StepAgent::ByType { template } => match self.find_or_spawn_agent_type(template) {
                Ok(resolved) => Some(resolved),
                Err(e) => {
                    warn!(
                        agent_type = %template,
                        kind = e.kind(),
                        error = %e,
                        "Workflow step agent type could not be resolved"
                    );
                    None
                }
            },
        }
    }

    /// Resolve a workflow step's agent reference **without** side effects.
    ///
    /// `dry_run_workflow` is documented as side-effect free, so a `ByType`
    /// step must not spawn the template there: a preview that mints an agent
    /// per invocation turns "show me what this would do" into "do part of it".
    /// An already-registered instance is reported as-is; otherwise the
    /// template is loaded read-only and the id the real run *would* use is
    /// reported, without registering anything.
    pub fn preview_step_agent(&self, agent_ref: &StepAgent) -> Option<(AgentId, String, bool)> {
        match agent_ref {
            StepAgent::ById { id } => self.registry_agent_by_id(id),
            StepAgent::ByName { name } => self.registry_agent_by_name(name),
            StepAgent::ByType { template } => {
                if let Some(resolved) = self.registry_agent_by_name(template) {
                    return Some(resolved);
                }
                match load_agent_template(self.home_dir(), template) {
                    Ok((manifest, _)) => {
                        // The canonical UUID the real run would land on:
                        // an already-registered identity if one exists, else
                        // the same name-derived id `spawn_agent_inner` would
                        // derive (#4614). `get` never writes, so the preview
                        // stays non-mutating even for a never-spawned type.
                        let id = self
                            .agents
                            .agent_identities
                            .get(template)
                            .unwrap_or_else(|| AgentId::from_name(template));
                        Some((id, template.to_string(), manifest.inherit_parent_context))
                    }
                    Err(e) => {
                        warn!(
                            agent_type = %template,
                            kind = e.kind(),
                            error = %e,
                            "Workflow dry run could not preview step agent type"
                        );
                        None
                    }
                }
            }
        }
    }

    /// Find-or-spawn the agent backing an agent *type*.
    ///
    /// 1. Reuse the registered agent named after the type, if there is one.
    /// 2. Otherwise load its template manifest and spawn it top-level, which
    ///    binds it to the canonical name-derived UUID via `agent_identities`
    ///    (#4614) so the same type keeps the same id and session history
    ///    across restarts.
    ///
    /// The spawn is top-level (`parent = None`) on purpose: a workflow step
    /// agent outlives the run that first needed it and is shared by every
    /// later run of every workflow referencing that type, so parenting it to
    /// whatever happened to run first would give it that agent's lifetime and
    /// capability ceiling.
    pub fn find_or_spawn_agent_type(
        &self,
        requested: &str,
    ) -> Result<(AgentId, String, bool), StepAgentResolveError> {
        if let Some(resolved) = self.registry_agent_by_name(requested) {
            return Ok(resolved);
        }

        let (manifest, source_path) = load_agent_template(self.home_dir(), requested)?;
        // `load_agent_template` guarantees `manifest.name == requested`, which
        // is what makes the duplicate-race reuse below safe: the entry we
        // re-read under `requested` is the same agent this spawn was building.
        debug_assert_eq!(manifest.name, requested);
        let inherit = manifest.inherit_parent_context;

        match self.spawn_agent_with_source(manifest, Some(source_path)) {
            Ok(id) => {
                info!(
                    agent_type = %requested,
                    agent_id = %id,
                    "Spawned agent from template for a workflow step agent type"
                );
                Ok((id, requested.to_string(), inherit))
            }
            // A concurrent resolver for the same type won between our
            // `find_by_name` miss and this register. Reusing the winner is
            // correct — and is the ONLY spawn failure where reuse is correct.
            // Any other error (rejected module path, reserved name,
            // tool_exec override) means this manifest was never registered,
            // so an entry that happens to hold the name belongs to something
            // else and binding the step to it is the silent mis-binding this
            // resolution path exists to avoid.
            Err(KernelError::LibreFang(LibreFangError::AgentAlreadyExists(_))) => self
                .registry_agent_by_name(requested)
                .ok_or_else(|| StepAgentResolveError::RaceLost {
                    requested: requested.to_string(),
                }),
            Err(e) => Err(StepAgentResolveError::Spawn {
                requested: requested.to_string(),
                detail: e.to_string(),
            }),
        }
    }
}
