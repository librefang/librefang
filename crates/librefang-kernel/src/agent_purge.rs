//! Purge every trace of an agent.
//!
//! Written to be shared; the CLI (`librefang purge`) is the only caller
//! today. Deleting an agent removes it from the roster and stops it
//! running, but leaves its workspace directory and any agent-type of the
//! same name on disk, and (for an agent whose roster entry is already gone)
//! its sessions and memories in the database with nothing left pointing at
//! them. This module is the one place that cleans all of it.
//!
//! Deliberately a free function rather than a `Kernel` method: the CLI purges
//! without a running daemon by opening the database directly, and hanging
//! this off the kernel would force it to boot one.

use crate::agent_identity_registry::AgentIdentityRegistry;
use crate::cron::CronScheduler;
use crate::kernel::workspace_setup::resolved_workspace_dir;
use crate::triggers::TriggerEngine;
use librefang_memory::agent_tables::AGENT_SCOPED_TABLES;
use librefang_memory::MemorySubstrate;
use librefang_types::agent::{AgentEntry, AgentId};
use librefang_types::agent_type_store::{agent_type_path_in, validate_agent_type_name};
use librefang_types::config::KernelConfig;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// What a purge actually removed. Every field is what happened, not what was
/// attempted, so a caller can report the truth rather than an intention.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct PurgeReport {
    /// The agent had a roster entry, and it (plus its sessions, memories and
    /// KV rows, via the substrate's cascade) was removed.
    pub roster_entry_removed: bool,
    /// The roster entry was already gone, but sessions, memories and KV rows
    /// survived it (a partial or pre-cascade delete); they were found by
    /// recovering the agent id from its name and removed the same way.
    pub orphaned_data_removed: bool,
    /// The name → canonical-UUID record in `agent_identities.toml` was
    /// dropped, so a future agent with the same name is not pinned to the
    /// purged agent's UUID.
    pub identity_record_removed: bool,
    /// The agent's workspace directory was deleted.
    pub workspace_removed: bool,
    /// The agent left traces behind, but where its workspace directory lived
    /// could not be established, so "no workspace was removed" must not be
    /// read as "there was no workspace".
    ///
    /// A workspace path comes from the agent's manifest (`workspace = ...`,
    /// relative to the workspaces root or absolute inside it) and falls back
    /// to the name-derived directory. An orphan has no manifest left to read,
    /// so when its default directory is not there, both "it never had one"
    /// and "it had one somewhere we cannot see" are consistent with what
    /// survives — and this flag says so instead of picking the flattering one.
    pub workspace_unresolved: bool,
    /// The manifest names a workspace directory that was deliberately left
    /// alone because deleting it would destroy more than this agent: either
    /// it resolves to the workspaces root itself (`workspace = "."`), or
    /// another still-registered agent's manifest resolves to the same
    /// directory (two agents sharing a named workspace by design).
    /// "no workspace was removed" must not be read as "there was nothing to
    /// remove" — there was, and it was left standing on purpose.
    pub workspace_shared: bool,
    /// An agent-type template of the same name was deleted.
    pub agent_type_removed: bool,
    /// Cron jobs owned by the purged agent id(s) were removed from
    /// `<home>/data/cron_jobs.json`.
    ///
    /// Jobs are keyed by `AgentId`, and the default id is derived
    /// deterministically from the agent's name — so without this, a new
    /// agent later spawned under the purged name would inherit the old
    /// agent's cron schedule outright.
    pub cron_jobs_removed: bool,
    /// Event triggers owned by the purged agent id(s) were removed from
    /// `<home>/trigger_jobs.json`, for the same name-collision reason as
    /// [`Self::cron_jobs_removed`].
    pub trigger_jobs_removed: bool,
    /// Channel routing rows (`channel_instance_defaults`,
    /// `conversation_bindings`) naming this agent were removed.
    ///
    /// Both tables key on the agent's *name*, not its id, so this runs
    /// whenever the name matches — regardless of whether a roster entry or
    /// orphaned rows were found for it — for the same reason cron/trigger
    /// residue matters: a new agent spawned under the purged name would
    /// otherwise inherit its channel/conversation routing.
    pub channel_bindings_removed: bool,
    /// Rows exist in agent-scoped tables under an id that is neither a live
    /// roster entry nor attributable to any name this purge recovered —
    /// most likely a child or ephemeral agent (`AgentId::new()`, never
    /// registered) whose roster row is already gone. Purge cannot safely
    /// guess which name they belonged to, so nothing was deleted for them;
    /// this only says they are there, the same honesty
    /// [`Self::workspace_unresolved`] already gives for an unresolved path.
    pub other_orphans_present: bool,
}

impl PurgeReport {
    /// True when nothing at all was found to remove — the caller asked to
    /// purge something that leaves no trace anywhere.
    pub fn is_empty(&self) -> bool {
        !(self.roster_entry_removed
            || self.orphaned_data_removed
            || self.identity_record_removed
            || self.workspace_removed
            || self.agent_type_removed
            || self.cron_jobs_removed
            || self.trigger_jobs_removed
            || self.channel_bindings_removed)
    }
}

/// Read-only preview of what a purge would remove, as computed by
/// [`plan_purge`]. `--dry-run` prints `preview` and stops; the destructive
/// path executes the plan.
#[derive(Debug, Clone, Default)]
pub struct PurgePlan {
    /// What a purge of this agent would remove, computed without writing.
    pub preview: PurgeReport,
    /// Read errors hit while planning. A plan with failures must not be
    /// executed: a roster read failure means live agents cannot be told
    /// apart from orphans, and executing could cascade a live agent's data.
    pub failures: Vec<String>,
    /// The roster entry's id, when the roster still knows this name.
    pub roster_agent_id: Option<AgentId>,
    /// Ids whose substrate rows are orphaned — rows exist, no roster entry
    /// holds the id.
    pub orphan_agent_ids: Vec<AgentId>,
    /// Whether the canonical-UUID registry holds a record for this name.
    pub registry_record: bool,
    /// The workspace directory that would be deleted, when it exists.
    pub workspace: Option<PathBuf>,
    /// The agent-type file that would be deleted, when it exists.
    pub agent_type: Option<PathBuf>,
}

/// What a purge did, alongside everything that went wrong.
///
/// Failures do not abort the run: the roster cascade runs first, so a
/// workspace delete that fails must not leave the caller without the report
/// of what already happened. An outcome with a non-empty `report` and a
/// non-empty `failures` list is a partial purge, and rerunning the command
/// cleans up the rest (the whole module is idempotent by design).
#[derive(Debug, Clone, Default)]
pub struct PurgeOutcome {
    /// What was actually removed.
    pub report: PurgeReport,
    /// Every step that failed, with the reason.
    pub failures: Vec<String>,
}

/// Read-only preview of what [`purge_agent`] would remove for `agent_name`.
/// Never writes — not even the workspaces root, which is why workspace paths
/// are resolved through [`resolved_workspace_dir`] rather than the spawn-side
/// `resolve_workspace_dir` that creates it.
///
/// Confirmation prompts and `--dry-run` show this, so the operator confirms
/// what will actually happen rather than a guess.
pub fn plan_purge(substrate: &MemorySubstrate, cfg: &KernelConfig, agent_name: &str) -> PurgePlan {
    let home = cfg.home_dir.as_path();
    let mut failures = Vec::new();
    if let Err(reason) = validate_purgeable_name(agent_name) {
        failures.push(format!("invalid agent name {agent_name:?}: {reason}"));
        return PurgePlan {
            failures,
            ..PurgePlan::default()
        };
    }

    let entries = match substrate.load_all_agents() {
        Ok(entries) => entries,
        Err(e) => {
            failures.push(format!("read roster: {e}"));
            return PurgePlan {
                failures,
                ..PurgePlan::default()
            };
        }
    };

    // `entries` hydrates every roster row through manifest deserialization,
    // and drops the row on an unparseable UUID, a manifest that fails to
    // deserialize, or a duplicate lowercased name — exactly the damaged-row
    // situations that lead an operator to purge in the first place. A live
    // agent caught by one of those drops must still count as live for the
    // orphan guard below, so that guard reads ids straight from the table
    // instead of through the lossy loader.
    let live_ids: HashSet<String> = match substrate.list_agents() {
        Ok(rows) => rows.into_iter().map(|(id, _, _)| id).collect(),
        Err(e) => {
            failures.push(format!("read agent ids: {e}"));
            return PurgePlan {
                failures,
                ..PurgePlan::default()
            };
        }
    };

    let registry = AgentIdentityRegistry::load(home);
    let registry_record = registry.get(agent_name).is_some();

    let mut preview = PurgeReport::default();
    let mut roster_agent_id = None;
    let mut orphan_agent_ids = Vec::new();

    let roster_entry = entries.iter().find(|e| e.name == agent_name);
    if let Some(entry) = roster_entry {
        preview.roster_entry_removed = true;
        roster_agent_id = Some(entry.id);
    }

    // Recover orphan candidates for this name whether or not a live roster
    // entry currently holds it: residue from an earlier incarnation of the
    // same name (recreated under a different id since) survives a purge
    // that only ever considered the current id. Two sources: the
    // canonical-UUID registry (agents spawned with a random id) and the
    // deterministic name-derived UUID. A candidate any live agent's id
    // matches is never touched — its data belongs to a running agent.
    let mut candidates: Vec<AgentId> = Vec::new();
    if let Some(id) = registry.get(agent_name) {
        candidates.push(id);
    }
    let derived = AgentId::from_name(agent_name);
    if !candidates.contains(&derived) {
        candidates.push(derived);
    }
    for id in candidates {
        if live_ids.contains(&id.0.to_string()) {
            continue;
        }
        match has_agent_rows(substrate, &id) {
            Ok(true) => {
                preview.orphaned_data_removed = true;
                orphan_agent_ids.push(id);
            }
            Ok(false) => {}
            Err(e) => {
                // Cannot verify this candidate is safe to cascade;
                // stop rather than purge on a guess.
                failures.push(e);
                break;
            }
        }
    }
    preview.identity_record_removed = registry_record;

    // Rows can exist under an id this purge could not attribute to any name
    // at all — a child or ephemeral agent (`AgentId::new()`, a random id)
    // whose roster row is already gone and was never recorded in the
    // identity registry either. There is no name to recover those under,
    // so they are reported rather than guessed at: a candidate this purge
    // did not derive from `agent_name` might just as well belong to some
    // other name entirely, and deleting it on a guess is the wrong
    // direction to err in.
    if failures.is_empty() {
        match other_orphan_ids_exist(substrate, &orphan_agent_ids) {
            Ok(present) => preview.other_orphans_present = present,
            // Everything in `failures` is fatal — `purge_agent` refuses to
            // execute a plan that has any — and this scan has not earned
            // that: it deletes nothing, and no other step's safety depends
            // on its answer. A failure here means "could not determine",
            // which must not veto a purge the rest of the plan proved safe
            // and leave the operator no way to clean up.
            Err(e) => {
                tracing::warn!(error = %e, "Purge: orphan-id scan failed; not reported")
            }
        }
    }

    // The workspaces root is configurable (`workspaces_dir`) and an agent can
    // carry its own `workspace` override, so neither the root nor the leaf is
    // safe to spell out here: both come from the same helpers spawn used to
    // create the directory in the first place.
    let agent_workspaces_root = cfg.effective_agent_workspaces_dir();
    let (mut workspace, mut workspace_unresolved) = match roster_entry {
        Some(entry) => match entry_workspace_dir(cfg, entry) {
            Some(dir) => (dir.is_dir().then_some(dir), false),
            // The manifest names a workspace the resolver refuses (outside the
            // root, or with `..` in it). The agent has a workspace and we
            // cannot say where — report that rather than "there was none".
            None => (None, true),
        },
        None => {
            // No roster entry means no manifest, so a `workspace` override is
            // unreadable; only the name-derived default is knowable. The id is
            // needed because that default falls back to it for a name with no
            // filesystem-safe characters left (`研究员` → the agent's UUID).
            let id = orphan_agent_ids
                .first()
                .copied()
                .or_else(|| registry.get(agent_name))
                .unwrap_or_else(|| AgentId::from_name(agent_name));
            let dir = resolved_workspace_dir(&agent_workspaces_root, None, agent_name, id)
                .ok()
                .filter(|d| d.is_dir());
            // An orphan whose default directory is empty is ambiguous: it
            // either never had a workspace or had one under an override we
            // cannot read. Say so, instead of reporting the flattering half.
            let unresolved = dir.is_none() && !orphan_agent_ids.is_empty();
            (dir, unresolved)
        }
    };

    // Never plan to delete the workspaces root itself — `workspace = "."`
    // joins onto it unchanged — or a directory another still-registered
    // agent's manifest also resolves to (two agents can share a named
    // workspace by design). Either would wipe out data this purge has no
    // business touching.
    let mut workspace_shared = false;
    if let Some(dir) = &workspace {
        let this_id = roster_agent_id.or_else(|| orphan_agent_ids.first().copied());
        let is_a_root = *dir == agent_workspaces_root || *dir == cfg.effective_workspaces_dir();
        // Containment, not equality. The direction that destroys data is
        // another agent's directory living *inside* the one about to be
        // `remove_dir_all`d — `workspace = "shared"` here, `"shared/team"`
        // there — which an equality test does not see at all, so the nested
        // agent's workspace goes with the parent and the report calls it a
        // clean removal. The inverse (this workspace being a subtree of
        // somebody else's) is the milder half of the same mistake and costs
        // one more comparison.
        let shared_with_another_agent = entries.iter().any(|other| {
            Some(other.id) != this_id
                && entry_occupied_dirs(cfg, other)
                    .iter()
                    .any(|o| o.starts_with(dir) || dir.starts_with(o))
        });
        if is_a_root || shared_with_another_agent {
            workspace_shared = true;
            workspace = None;
            workspace_unresolved = false;
        }
    }
    if workspace.is_some() {
        preview.workspace_removed = true;
    }
    preview.workspace_unresolved = workspace_unresolved;
    preview.workspace_shared = workspace_shared;

    // Preview cron/trigger/channel residue too. `.load()` only reads, and
    // this preview never calls `.persist()`, so nothing on disk changes —
    // these are throwaway engine instances discarded at the end of the
    // function.
    let purge_ids: Vec<AgentId> = roster_agent_id
        .into_iter()
        .chain(orphan_agent_ids.iter().copied())
        .collect();
    if !purge_ids.is_empty() {
        let cron = CronScheduler::new(home, cfg.max_cron_jobs);
        match cron.load() {
            Ok(_) => {
                preview.cron_jobs_removed =
                    purge_ids.iter().any(|id| !cron.list_jobs(*id).is_empty());
            }
            Err(e) => failures.push(format!("read cron jobs: {e}")),
        }

        let triggers = TriggerEngine::with_config(&cfg.triggers, home);
        match triggers.load() {
            Ok(_) => {
                preview.trigger_jobs_removed = purge_ids
                    .iter()
                    .any(|id| !triggers.list_agent_triggers(*id).is_empty());
            }
            Err(e) => failures.push(format!("read trigger jobs: {e}")),
        }
    }

    // Channel routing rows key on the agent's *name*, not id, so this scan
    // does not depend on `purge_ids` at all.
    match count_channel_bindings_for_agent(substrate, agent_name) {
        Ok(count) => preview.channel_bindings_removed = count > 0,
        Err(e) => failures.push(e),
    }

    // `validate_agent_type_name` governs filenames in the agent-type store, so
    // a name it rejects cannot name a file there. That is not a purge failure:
    // there is simply nothing to look for, and the rest of the purge runs.
    let agent_type = validate_agent_type_name(agent_name)
        .ok()
        .map(|()| agent_type_path_in(home, agent_name))
        .filter(|p| p.is_file());
    if agent_type.is_some() {
        preview.agent_type_removed = true;
    }

    PurgePlan {
        preview,
        failures,
        roster_agent_id,
        orphan_agent_ids,
        registry_record,
        workspace,
        agent_type,
    }
}

/// Remove every trace of `agent_name`: roster entry (cascading to sessions,
/// memories and KV rows), any orphaned rows left by a previous partial
/// delete, the canonical-UUID registry record, the workspace directory, and
/// any agent-type template with the same name.
///
/// Idempotent by design: purging an agent that is already partly gone cleans
/// up whatever remains and reports it, rather than failing. That is the whole
/// point — the caller is here precisely because a previous delete left
/// something behind.
///
/// Never aborts halfway: every step runs (or is skipped because planning
/// could not prove it safe), and [`PurgeOutcome::failures`] lists everything
/// that went wrong. Rerun the command to finish a partial purge.
pub fn purge_agent(
    substrate: &MemorySubstrate,
    cfg: &KernelConfig,
    agent_name: &str,
) -> PurgeOutcome {
    let home = cfg.home_dir.as_path();
    let plan = plan_purge(substrate, cfg, agent_name);
    if !plan.failures.is_empty() {
        // Planning could not prove what is live; executing could cascade a
        // running agent's data. Surface the failure instead of guessing.
        return PurgeOutcome {
            report: PurgeReport::default(),
            failures: plan.failures,
        };
    }

    let mut report = PurgeReport {
        // Not removals, but the same caveats hold whether or not the run is
        // a dry one, so they travel with the report the caller prints.
        workspace_unresolved: plan.preview.workspace_unresolved,
        workspace_shared: plan.preview.workspace_shared,
        other_orphans_present: plan.preview.other_orphans_present,
        ..PurgeReport::default()
    };
    let mut failures = Vec::new();

    if let Some(id) = plan.roster_agent_id {
        match substrate.remove_agent(id) {
            Ok(()) => report.roster_entry_removed = true,
            Err(e) => failures.push(format!(
                "remove roster entry, sessions and memories for {id}: {e}"
            )),
        }
    }
    for id in &plan.orphan_agent_ids {
        match substrate.remove_agent(*id) {
            Ok(()) => report.orphaned_data_removed = true,
            Err(e) => failures.push(format!("remove orphaned rows for {id}: {e}")),
        }
    }

    // Cron/trigger residue: both stores key on `AgentId`, and the default
    // id is derived deterministically from the agent's name, so leaving
    // these behind means a new agent later spawned under the purged name
    // inherits the old agent's cron schedule and event wiring outright.
    let purge_ids: Vec<AgentId> = plan
        .roster_agent_id
        .into_iter()
        .chain(plan.orphan_agent_ids.iter().copied())
        .collect();
    if !purge_ids.is_empty() {
        let cron = CronScheduler::new(home, cfg.max_cron_jobs);
        match cron.load() {
            Ok(_) => {
                let removed: usize = purge_ids.iter().map(|id| cron.remove_agent_jobs(*id)).sum();
                if removed > 0 {
                    // Rides along with this write: `CronScheduler::load`
                    // disables any persisted job that fails re-validation
                    // (a hand-edited `every_secs = 0` would divide by zero
                    // in the scheduler), so persisting makes that quarantine
                    // durable for jobs belonging to agents this purge never
                    // touched. Not a state the installation was not headed
                    // for anyway — the daemon writes exactly the same thing
                    // at its next boot — and the alternative, merging our
                    // removals into a fresh read of the file, would
                    // resurrect a job that crash-loops the scheduler.
                    match cron.persist() {
                        Ok(()) => report.cron_jobs_removed = true,
                        Err(e) => failures.push(format!(
                            "persist cron jobs after removing {agent_name}'s: {e}"
                        )),
                    }
                }
            }
            Err(e) => failures.push(format!("load cron jobs: {e}")),
        }

        let triggers = TriggerEngine::with_config(&cfg.triggers, home);
        match triggers.load() {
            Ok(_) => {
                let had_any = purge_ids
                    .iter()
                    .any(|id| !triggers.list_agent_triggers(*id).is_empty());
                for id in &purge_ids {
                    triggers.remove_agent_triggers(*id);
                }
                if had_any {
                    match triggers.persist() {
                        Ok(()) => report.trigger_jobs_removed = true,
                        Err(e) => failures.push(format!(
                            "persist trigger jobs after removing {agent_name}'s: {e}"
                        )),
                    }
                }
            }
            Err(e) => failures.push(format!("load trigger jobs: {e}")),
        }
    }

    // Channel routing rows key on the agent's *name*, not id, so this runs
    // unconditionally rather than only alongside a roster or orphan hit —
    // otherwise a new agent spawned under the purged name would inherit
    // its channel/conversation routing.
    match remove_channel_bindings_for_agent(substrate, agent_name) {
        Ok(removed) if removed > 0 => report.channel_bindings_removed = true,
        Ok(_) => {}
        Err(e) => failures.push(e),
    }

    // Drop the name → UUID binding last-but-not-unconditionally: the kernel
    // skips it when the roster row could not be removed (#5117) so the next
    // boot never loads a roster row whose name the registry no longer knows.
    // Same rule here: only unbind when every cascade above succeeded, and
    // only report it removed once it is actually durable on disk —
    // `AgentIdentityRegistry::purge` itself only warns on a persist
    // failure, so re-checking the result here is what turns a read-only
    // home directory into a reported failure instead of a silent no-op.
    if plan.registry_record && failures.is_empty() {
        let registry = AgentIdentityRegistry::load(home);
        if registry.purge(agent_name).is_some() {
            match registry.persist() {
                Ok(()) => report.identity_record_removed = true,
                Err(e) => failures.push(format!(
                    "persist agent_identities.toml after removing {agent_name}: {e}"
                )),
            }
        }
    }

    if let Some(workspace) = &plan.workspace {
        match std::fs::remove_dir_all(workspace) {
            Ok(()) => report.workspace_removed = true,
            Err(e) => failures.push(format!("remove workspace {}: {e}", workspace.display())),
        }
    }
    if let Some(agent_type) = &plan.agent_type {
        match std::fs::remove_file(agent_type) {
            Ok(()) => report.agent_type_removed = true,
            Err(e) => failures.push(format!("remove agent-type {}: {e}", agent_type.display())),
        }
    }

    PurgeOutcome { report, failures }
}

/// Reject a name that would not stay put when joined onto a directory.
///
/// Deliberately *not* `validate_agent_type_name`: that one permits only
/// `[A-Za-z0-9_-]` because it names a file in the agent-type store, whereas
/// the only rule at the agent-registry boundary is `validate_agent_name`,
/// which reserves the `_operator:` prefix and nothing else. `my.bot`,
/// `data agent` and `研究员` are ordinary agent names, and refusing to purge
/// them would leave standing exactly the residue this command exists to
/// remove.
///
/// What purge actually needs is that the name cannot escape a join, so it
/// must be one plain path component.
fn validate_purgeable_name(name: &str) -> Result<(), &'static str> {
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        // `Component::Normal` rules out `..`, `.`, a root and a Windows drive
        // prefix; requiring exactly one of them rules out any separator.
        // Comparing it back to the input rejects the shapes the parser
        // normalises away (`a/`, `./a`) — separators too.
        (Some(Component::Normal(only)), None) if only == OsStr::new(name) => Ok(()),
        _ => Err("must be a single path component, with no separator and no '..'"),
    }
}

/// Whether any agent-scoped substrate row exists for `id` — the check that
/// separates "rows outlived the roster entry" from "the name is simply not
/// in this installation".
///
/// Walks `AGENT_SCOPED_TABLES`, the same constant the substrate's
/// `remove_agent` cascade builds its DELETEs from, so the scan cannot see
/// less than the cascade clears. It used to carry its own four-table list,
/// which left an orphan whose surviving rows sat in any of the other sixteen
/// reported as leaving no trace at all — over rows that were still there.
fn has_agent_rows(substrate: &MemorySubstrate, id: &AgentId) -> Result<bool, String> {
    let conn = substrate
        .pool()
        .get()
        .map_err(|e| format!("acquire database connection: {e}"))?;
    let id = id.0.to_string();
    for (_, table, column) in AGENT_SCOPED_TABLES {
        let exists: bool = conn
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column} = ?1)"),
                rusqlite::params![id],
                |row| row.get(0),
            )
            .map_err(|e| format!("scan {table} for rows of agent {id}: {e}"))?;
        if exists {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `AGENT_SCOPED_TABLES` holds a row under an id that has no roster
/// entry and is not in `attributed` — the ids this purge already recovered
/// for `agent_name`.
///
/// Existence only: an id like this cannot be traced back to any name (a
/// child or ephemeral agent's `AgentId::new()` is neither name-derived nor
/// ever recorded in the identity registry), so nothing here is safe to
/// delete on this purge's behalf — it may belong to a different name
/// entirely. This only makes the operator aware the residue exists, the way
/// [`PurgeReport::workspace_unresolved`] does for an unlocatable directory.
fn other_orphan_ids_exist(
    substrate: &MemorySubstrate,
    attributed: &[AgentId],
) -> Result<bool, String> {
    let conn = substrate
        .pool()
        .get()
        .map_err(|e| format!("acquire database connection: {e}"))?;
    let attributed: Vec<String> = attributed.iter().map(|id| id.0.to_string()).collect();
    let placeholders = vec!["?"; attributed.len()].join(",");
    for (_, table, column) in AGENT_SCOPED_TABLES {
        if *table == "agents" {
            continue; // the roster itself — every row here is a live agent.
        }
        // Both filters belong in SQL. The answer on a healthy installation
        // is `false`, and materialising every distinct id of every
        // agent-scoped table into a `Vec<String>` to reach it is work no
        // answer needs; `EXISTS` stops at the first row that qualifies.
        // Reading the id back out was also the one place a non-TEXT value
        // in an agent-scoping column could raise `InvalidColumnType` —
        // `EXISTS` yields an integer whatever the column holds.
        // `NOT IN ()` is a syntax error, so an empty attributed list drops
        // the clause rather than binding nothing to it.
        let exclusion = if attributed.is_empty() {
            String::new()
        } else {
            format!(" AND {column} NOT IN ({placeholders})")
        };
        let exists: bool = conn
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM {table} \
                     WHERE {column} NOT IN (SELECT id FROM agents){exclusion})"
                ),
                rusqlite::params_from_iter(attributed.iter()),
                |row| row.get(0),
            )
            .map_err(|e| format!("scan {table} for orphan ids: {e}"))?;
        if exists {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Where `entry`'s workspace directory resolves to, following the same rule
/// spawn used when it created it.
///
/// A relative `workspace` override (or none at all) joins under
/// `<workspaces>/agents/`. An absolute one is validated against the whole
/// workspaces root instead: spawn rewrites a hand agent's `workspace` to the
/// absolute, already-resolved `<workspaces>/hands/<hand>/<role>` (see
/// `backfill_workspace_dir`), and validating that against the `agents/`
/// root alone fails `starts_with` even though the manifest and roster entry
/// are both alive.
fn entry_workspace_dir(cfg: &KernelConfig, entry: &AgentEntry) -> Option<PathBuf> {
    let root = match &entry.manifest.workspace {
        Some(p) if p.is_absolute() => cfg.effective_workspaces_dir(),
        _ => cfg.effective_agent_workspaces_dir(),
    };
    resolved_workspace_dir(
        &root,
        entry.manifest.workspace.clone(),
        &entry.name,
        entry.id,
    )
    .ok()
}

/// Every directory under the workspaces tree that `entry` occupies: the one
/// [`entry_workspace_dir`] resolves, plus each directory its manifest's
/// `[workspaces]` table declares.
///
/// Named workspaces are the second way an agent gets a directory, and
/// `ensure_named_workspaces` resolves a `path` entry as
/// `workspaces_root.join(path)` with nothing forbidding `path =
/// "agents/<something>"` — so a sharing guard that consulted only
/// `manifest.workspace` would happily delete one.
///
/// A declaration that does not land inside the workspaces root is dropped
/// rather than trusted. The directory being purged always lives inside that
/// root, so an out-of-tree target can never be nested within it, while
/// honouring one would let a single `mount = "/"` (or a malformed absolute
/// `path`) veto every workspace deletion in the installation.
fn entry_occupied_dirs(cfg: &KernelConfig, entry: &AgentEntry) -> Vec<PathBuf> {
    let root = cfg.effective_workspaces_dir();
    let mut dirs: Vec<PathBuf> = entry_workspace_dir(cfg, entry).into_iter().collect();
    for decl in entry.manifest.workspaces.values() {
        let declared = match (&decl.path, &decl.mount) {
            (Some(rel), None) => root.join(rel),
            (None, Some(mount)) => mount.clone(),
            // Exactly one of the two is required; boot skips anything else.
            _ => continue,
        };
        if declared.starts_with(&root) {
            dirs.push(declared);
        }
    }
    dirs
}

/// Count of channel routing rows (`channel_instance_defaults`,
/// `conversation_bindings`) naming `agent_name`. Both tables store the
/// agent's name rather than its `AgentId`
/// (see `librefang_memory::channel_binding_store`), so this is a name
/// lookup rather than an id-scoped one like [`has_agent_rows`].
fn count_channel_bindings_for_agent(
    substrate: &MemorySubstrate,
    agent_name: &str,
) -> Result<usize, String> {
    let conn = substrate
        .pool()
        .get()
        .map_err(|e| format!("acquire database connection: {e}"))?;
    let mut total = 0usize;
    for table in ["channel_instance_defaults", "conversation_bindings"] {
        let count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE agent_name = ?1"),
                rusqlite::params![agent_name],
                |row| row.get(0),
            )
            .map_err(|e| format!("count {table} rows for {agent_name}: {e}"))?;
        total += count as usize;
    }
    Ok(total)
}

/// Delete channel routing rows naming `agent_name`. Returns the number of
/// rows removed. See [`count_channel_bindings_for_agent`] for why this is a
/// name lookup, not an id-scoped one.
fn remove_channel_bindings_for_agent(
    substrate: &MemorySubstrate,
    agent_name: &str,
) -> Result<usize, String> {
    let conn = substrate
        .pool()
        .get()
        .map_err(|e| format!("acquire database connection: {e}"))?;
    let mut total = 0usize;
    for table in ["channel_instance_defaults", "conversation_bindings"] {
        let removed = conn
            .execute(
                &format!("DELETE FROM {table} WHERE agent_name = ?1"),
                rusqlite::params![agent_name],
            )
            .map_err(|e| format!("remove {table} rows for {agent_name}: {e}"))?;
        total += removed;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::agent::{AgentEntry, AgentState};
    use librefang_types::agent_type_store::agent_types_dir_in;

    fn home_with(agents: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for a in agents {
            seed_workspace(&dir.path().join("workspaces").join("agents").join(a));
            let types = agent_types_dir_in(dir.path());
            std::fs::create_dir_all(&types).unwrap();
            std::fs::write(types.join(format!("{a}.toml")), "x").unwrap();
        }
        dir
    }

    /// A workspace directory as spawn leaves it, at whatever path the caller
    /// resolved — which is the point: the location is not a fixed literal.
    fn seed_workspace(dir: &Path) {
        std::fs::create_dir_all(dir.join(".identity")).unwrap();
        std::fs::write(dir.join("agent.toml"), "x").unwrap();
    }

    /// The config a purge runs against. `workspaces_dir` unset means the
    /// default `{home}/workspaces/agents`, which is what `home_with` seeds.
    fn cfg_for(home: &tempfile::TempDir) -> KernelConfig {
        KernelConfig {
            home_dir: home.path().to_path_buf(),
            ..KernelConfig::default()
        }
    }

    /// Seed a full agent footprint: roster entry, a session, a KV row and a
    /// memory row, all under `id`.
    fn seed_agent_rows(substrate: &MemorySubstrate, name: &str, id: AgentId) {
        let entry = AgentEntry {
            id,
            name: name.to_string(),
            state: AgentState::Running,
            ..Default::default()
        };
        substrate.save_agent(&entry).unwrap();
        substrate.create_session(id).unwrap();
        substrate
            .structured_set(id, "purge-test", serde_json::json!("seeded"))
            .unwrap();
        let conn = substrate.pool().get().unwrap();
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, created_at, accessed_at) \
             VALUES ('purge-test-memory', ?1, 'remembered', 'test', datetime('now'), datetime('now'))",
            rusqlite::params![id.0.to_string()],
        )
        .unwrap();
    }

    /// Simulate the legacy partial delete this module exists for: the roster
    /// row goes, every other row stays.
    fn delete_roster_row_only(substrate: &MemorySubstrate, id: AgentId) {
        let conn = substrate.pool().get().unwrap();
        conn.execute(
            "DELETE FROM agents WHERE id = ?1",
            rusqlite::params![id.0.to_string()],
        )
        .unwrap();
    }

    fn orphan_row_count(substrate: &MemorySubstrate, id: AgentId) -> i64 {
        let conn = substrate.pool().get().unwrap();
        ["sessions", "memories", "kv_store"]
            .iter()
            .map(|table| {
                conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE agent_id = ?1"),
                    rusqlite::params![id.0.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
            })
            .sum()
    }

    #[test]
    fn it_removes_the_workspace_and_the_agent_type() {
        let home = home_with(&["alpha", "beta"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.workspace_removed);
        assert!(outcome.report.agent_type_removed);
        assert!(!home.path().join("workspaces/agents/alpha").exists());
        assert!(!agent_type_path_in(home.path(), "alpha").exists());
    }

    #[test]
    fn it_leaves_every_other_agent_alone() {
        let home = home_with(&["alpha", "beta"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(home
            .path()
            .join("workspaces/agents/beta/agent.toml")
            .exists());
        assert!(agent_type_path_in(home.path(), "beta").exists());
    }

    #[test]
    fn purging_something_already_gone_is_not_an_error() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let outcome = purge_agent(&substrate, &cfg_for(&home), "nobody");

        assert!(outcome.failures.is_empty());
        assert!(outcome.report.is_empty());
    }

    #[test]
    fn a_path_shaped_name_never_reaches_the_filesystem() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        for evil in ["../../etc", "a/b", "..", ""] {
            assert!(
                purge_agent(&substrate, &cfg_for(&home), evil)
                    .failures
                    .iter()
                    .any(|f| f.contains("invalid agent name")),
                "{evil:?} must be rejected before any join"
            );
        }
        assert!(home.path().join("workspaces/agents/alpha").exists());
    }

    /// THE headline case: the roster entry is already gone but its session,
    /// memory and KV rows are not. Purge-by-name must find the id and
    /// cascade the orphans away, not report "nothing to purge".
    #[test]
    fn purge_by_name_cleans_rows_that_outlived_the_roster_entry() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("alpha");
        seed_agent_rows(&substrate, "alpha", id);
        delete_roster_row_only(&substrate, id);
        assert!(orphan_row_count(&substrate, id) > 0, "seed failed");

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.orphaned_data_removed);
        assert_eq!(
            orphan_row_count(&substrate, id),
            0,
            "orphaned rows survived"
        );
    }

    /// The deterministic name derivation is not the only id source: an agent
    /// spawned with a random UUID leaves a registry record behind. That
    /// record must lead the purge back to the orphaned rows too.
    #[test]
    fn orphan_rows_are_found_through_the_identity_registry_even_for_random_ids() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::new();
        seed_agent_rows(&substrate, "alpha", id);
        delete_roster_row_only(&substrate, id);
        let registry = AgentIdentityRegistry::load(home.path());
        registry.register_if_absent("alpha", id);

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.orphaned_data_removed);
        assert!(outcome.report.identity_record_removed);
        assert_eq!(
            orphan_row_count(&substrate, id),
            0,
            "orphaned rows survived"
        );
        assert!(AgentIdentityRegistry::load(home.path())
            .get("alpha")
            .is_none());
    }

    /// An agent spawned as "alpha" and later renamed to "beta" keeps its id.
    /// Purging the stale name "alpha" must not cascade the live agent's rows.
    #[test]
    fn a_live_agents_id_is_never_treated_as_an_orphan() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("alpha");
        seed_agent_rows(&substrate, "beta", id);

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(!outcome.report.orphaned_data_removed);
        assert!(outcome.report.is_empty());
        assert_eq!(
            orphan_row_count(&substrate, id),
            3,
            "live agent's rows must survive"
        );
    }

    #[test]
    fn dry_run_plan_matches_what_the_real_purge_removes() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        seed_agent_rows(&substrate, "alpha", AgentId::from_name("alpha"));

        let plan = plan_purge(&substrate, &cfg_for(&home), "alpha");
        assert!(plan.failures.is_empty(), "{:?}", plan.failures);
        assert!(plan.preview.roster_entry_removed);
        assert!(plan.preview.workspace_removed);
        assert!(plan.preview.agent_type_removed);
        assert!(!plan.preview.identity_record_removed);
        assert!(plan.roster_agent_id.is_some());
        assert!(plan.workspace.is_some());
        assert!(plan.agent_type.is_some());

        // Planning itself must not have touched anything.
        assert!(substrate
            .load_all_agents()
            .unwrap()
            .iter()
            .any(|e| e.name == "alpha"));
        assert!(agent_type_path_in(home.path(), "alpha").exists());

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");
        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert_eq!(outcome.report, plan.preview);
    }

    fn row_count(substrate: &MemorySubstrate, table: &str, column: &str, id: AgentId) -> i64 {
        let conn = substrate.pool().get().unwrap();
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?1"),
            rusqlite::params![id.0.to_string()],
            |row| row.get(0),
        )
        .unwrap()
    }

    /// An orphan whose only surviving rows are in a table the scan used to
    /// miss. `usage_events` and `goal_runs` are both cleared by the cascade
    /// and neither was in the old four-table list, so purge reported
    /// "left no trace in this installation" over rows that were still there —
    /// and, because the report was empty, never called the cascade that would
    /// have removed them.
    ///
    /// Not a hypothetical: the cascade grew over time (`pending_approvals` in
    /// v26, `goal_runs` in v42), and time-based retention diverges between
    /// `sessions` and `usage_events` on a current build, so residue outlives
    /// the four scanned tables by ordinary means.
    #[test]
    fn orphan_rows_outside_the_old_four_table_scan_are_found_and_purged() {
        for (table, seed) in [
            ("usage_events", "INSERT INTO usage_events (id, agent_id, timestamp, model, input_tokens, output_tokens, cost_usd, tool_calls) \
                               VALUES ('purge-test-usage', ?1, datetime('now'), 'm', 0, 0, 0.0, 0)"),
            ("goal_runs", "INSERT INTO goal_runs (goal_id, agent_id, phase, started_at, updated_at) \
                           VALUES ('purge-test-goal', ?1, 'finished', datetime('now'), datetime('now'))"),
        ] {
            let home = home_with(&[]);
            let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
            let id = AgentId::from_name("alpha");
            substrate
                .pool()
                .get()
                .unwrap()
                .execute(seed, rusqlite::params![id.0.to_string()])
                .unwrap();
            assert_eq!(row_count(&substrate, table, "agent_id", id), 1, "seed failed");
            // Nothing in the four tables the scan used to look at.
            assert_eq!(orphan_row_count(&substrate, id), 0);

            let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

            assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
            assert!(
                outcome.report.orphaned_data_removed,
                "{table} residue must count as a trace"
            );
            assert_eq!(
                row_count(&substrate, table, "agent_id", id),
                0,
                "{table} rows survived the purge"
            );
        }
    }

    /// The registry boundary reserves the `_operator:` prefix and nothing
    /// else, so these are ordinary agent names. The agent-type charset is not
    /// the rule for them, and refusing them left the residue standing.
    ///
    /// Both cases also exercise the default workspace derivation, which is
    /// `safe_path_component(name, agent_id)` — it strips the characters the
    /// agent-type validator rejects (`my.bot` → `mybot`) and falls back to the
    /// UUID when nothing is left (`研究员`). Joining the raw name would have
    /// missed both directories.
    #[test]
    fn names_outside_the_agent_type_charset_are_purged_not_refused() {
        for name in ["my.bot", "data agent", "研究员"] {
            let home = home_with(&[]);
            let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
            let id = AgentId::from_name(name);
            seed_agent_rows(&substrate, name, id);
            let workspace = home.path().join("workspaces").join("agents").join(
                crate::kernel::workspace_setup::safe_path_component(name, &id.to_string()),
            );
            seed_workspace(&workspace);

            let outcome = purge_agent(&substrate, &cfg_for(&home), name);

            assert!(
                outcome.failures.is_empty(),
                "{name}: {:?}",
                outcome.failures
            );
            assert!(outcome.report.roster_entry_removed, "{name}");
            assert!(outcome.report.workspace_removed, "{name}");
            assert!(!workspace.exists(), "{name}: workspace survived");
            assert_eq!(orphan_row_count(&substrate, id), 0, "{name}: rows survived");
        }
    }

    /// `workspaces_dir` moves the root. Purge must follow it rather than
    /// assume `{home}/workspaces/agents`, or it deletes nothing and reports
    /// success.
    #[test]
    fn the_workspace_follows_a_configured_workspaces_dir() {
        let home = home_with(&[]);
        let elsewhere = tempfile::tempdir().unwrap();
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        seed_agent_rows(&substrate, "alpha", AgentId::from_name("alpha"));
        let workspace = elsewhere.path().join("agents").join("alpha");
        seed_workspace(&workspace);
        let cfg = KernelConfig {
            workspaces_dir: Some(elsewhere.path().to_path_buf()),
            ..cfg_for(&home)
        };

        let outcome = purge_agent(&substrate, &cfg, "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.workspace_removed);
        assert!(!workspace.exists(), "relocated workspace survived");
    }

    /// A per-agent `workspace` override in the manifest moves the leaf. The
    /// roster entry carries the manifest, so purge reads it instead of
    /// guessing the name-derived directory.
    #[test]
    fn a_manifest_workspace_override_is_honoured() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("alpha");
        let mut entry = AgentEntry {
            id,
            name: "alpha".to_string(),
            state: AgentState::Running,
            ..Default::default()
        };
        entry.manifest.workspace = Some(PathBuf::from("shared/alpha-home"));
        substrate.save_agent(&entry).unwrap();
        let workspace = home
            .path()
            .join("workspaces")
            .join("agents")
            .join("shared")
            .join("alpha-home");
        seed_workspace(&workspace);
        // The name-derived directory exists too and must be left alone: the
        // manifest, not the name, says where this agent's workspace is.
        let by_name = home.path().join("workspaces").join("agents").join("alpha");
        seed_workspace(&by_name);

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.workspace_removed);
        assert!(!workspace.exists(), "the manifest's workspace survived");
        assert!(
            by_name.exists(),
            "the name-derived directory is not this agent's"
        );
    }

    /// An orphan has no manifest left, so an override is unreadable. When the
    /// default directory is not there, "nothing was removed" is ambiguous —
    /// the report says the location was not resolvable rather than claiming
    /// there was no workspace.
    #[test]
    fn an_orphans_missing_workspace_is_reported_as_unresolved_not_absent() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("alpha");
        seed_agent_rows(&substrate, "alpha", id);
        delete_roster_row_only(&substrate, id);

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.report.orphaned_data_removed);
        assert!(!outcome.report.workspace_removed);
        assert!(outcome.report.workspace_unresolved);

        // A name with no trace at all is not ambiguous — there is nothing to
        // have had a workspace, so the caveat stays quiet.
        let quiet = purge_agent(&substrate, &cfg_for(&home), "nobody");
        assert!(!quiet.report.workspace_unresolved);
        assert!(quiet.report.is_empty());
    }

    /// THE stale-name headline case: the name is live again under a fresh
    /// id, but rows an *earlier* incarnation of the same name left behind
    /// (roster gone, rows not) sit under a different id. Purging the
    /// live name must still find and remove that older residue, not just
    /// the current holder's own data.
    #[test]
    fn stale_orphans_from_an_earlier_incarnation_are_found_even_when_the_name_is_live_again() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let old_id = AgentId::from_name("alpha");
        seed_agent_rows(&substrate, "alpha", old_id);
        delete_roster_row_only(&substrate, old_id);
        assert!(orphan_row_count(&substrate, old_id) > 0, "seed failed");

        let new_id = AgentId::new();
        substrate
            .save_agent(&AgentEntry {
                id: new_id,
                name: "alpha".to_string(),
                state: AgentState::Running,
                ..Default::default()
            })
            .unwrap();

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.roster_entry_removed);
        assert!(
            outcome.report.orphaned_data_removed,
            "the earlier incarnation's rows, left under a different id, must be found too"
        );
        assert_eq!(
            orphan_row_count(&substrate, old_id),
            0,
            "stale rows from the earlier incarnation survived"
        );
    }

    /// `load_all_agents` drops a row whose manifest blob fails to
    /// deserialize — exactly the damaged-row situation that leads an
    /// operator to run purge. The live-agent guard must read ids straight
    /// from the table, or a purge for some other name that happens to
    /// derive the same id cascades the still-running agent's rows away.
    #[test]
    fn a_live_agent_with_a_corrupt_manifest_is_never_treated_as_an_orphan() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        // Purging "alpha" derives exactly this id; the rows actually live
        // under the current name "beta" (renamed since spawn, same id).
        let id = AgentId::from_name("alpha");
        seed_agent_rows(&substrate, "beta", id);

        {
            let conn = substrate.pool().get().unwrap();
            conn.execute(
                "UPDATE agents SET manifest = ?1 WHERE id = ?2",
                rusqlite::params![b"not valid msgpack".to_vec(), id.0.to_string()],
            )
            .unwrap();
        } // drop the pooled connection before the calls below acquire their own —
          // the in-memory substrate's pool is sized for one connection at a time.
          // Confirm the corruption actually reproduces the gap this fix
          // closes: the hydrating loader drops the row, the raw table still
          // has it.
        assert!(!substrate
            .load_all_agents()
            .unwrap()
            .iter()
            .any(|e| e.id == id));
        assert!(substrate
            .list_agents()
            .unwrap()
            .iter()
            .any(|(row_id, _, _)| *row_id == id.0.to_string()));

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(
            !outcome.report.orphaned_data_removed,
            "a live agent's rows must never be cascaded"
        );
        assert_eq!(
            orphan_row_count(&substrate, id),
            3,
            "the live agent's rows must survive"
        );
    }

    /// An id with rows in agent-scoped tables but no roster entry and no
    /// name this purge can derive it from (a child/ephemeral agent's
    /// random `AgentId::new()`, never registered) cannot be safely
    /// attributed to `agent_name`. It must be reported, not deleted — it
    /// might belong to a completely different name.
    #[test]
    fn unattributable_orphan_rows_are_reported_but_never_touched() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let orphan_id = AgentId::new();
        seed_agent_rows(&substrate, "irrelevant", orphan_id);
        delete_roster_row_only(&substrate, orphan_id);
        assert!(orphan_row_count(&substrate, orphan_id) > 0, "seed failed");

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(
            outcome.report.is_empty(),
            "nothing attributable to 'alpha' exists"
        );
        assert!(
            outcome.report.other_orphans_present,
            "an orphan id this purge could not attribute to any name must be surfaced"
        );
        assert_eq!(
            orphan_row_count(&substrate, orphan_id),
            3,
            "an id this purge cannot attribute to a name must never be deleted"
        );
    }

    /// `workspace = "."` joins onto the workspaces root unchanged.
    /// Honouring it would delete every agent's workspace directory.
    #[test]
    fn a_workspace_override_pointing_at_the_root_itself_is_never_deleted() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("alpha");
        let mut entry = AgentEntry {
            id,
            name: "alpha".to_string(),
            state: AgentState::Running,
            ..Default::default()
        };
        entry.manifest.workspace = Some(PathBuf::from("."));
        substrate.save_agent(&entry).unwrap();
        let root = home.path().join("workspaces").join("agents");
        // Proof that honouring "." would take out another agent's data too.
        seed_workspace(&root.join("someone-else"));

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(
            !outcome.report.workspace_removed,
            "the workspaces root must never be deleted"
        );
        assert!(
            outcome.report.workspace_shared,
            "the collision must be reported, not silently skipped"
        );
        assert!(
            root.join("someone-else").exists(),
            "another agent's workspace must survive"
        );
    }

    /// Two agents can share a named workspace by design (`workspace =
    /// "shared/team"` on both manifests). Purging one must not delete the
    /// directory a still-registered agent is also pointed at.
    #[test]
    fn a_workspace_shared_with_another_live_agent_is_never_deleted() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let shared = PathBuf::from("shared/team");
        for (id, name) in [
            (AgentId::from_name("alpha"), "alpha"),
            (AgentId::from_name("beta"), "beta"),
        ] {
            let mut entry = AgentEntry {
                id,
                name: name.to_string(),
                state: AgentState::Running,
                ..Default::default()
            };
            entry.manifest.workspace = Some(shared.clone());
            substrate.save_agent(&entry).unwrap();
        }
        let workspace = home.path().join("workspaces/agents/shared/team");
        seed_workspace(&workspace);

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.roster_entry_removed);
        assert!(
            !outcome.report.workspace_removed,
            "a workspace shared with a live agent must never be deleted"
        );
        assert!(outcome.report.workspace_shared);
        assert!(
            workspace.exists(),
            "beta's still-live shared workspace must survive"
        );
    }

    /// The same collision one directory level up, which is the shape that
    /// actually loses data: `alpha` owns `shared`, `beta` owns
    /// `shared/team`. Nothing exotic is configured — two ordinary
    /// `workspace` overrides — and a guard that compares the two resolved
    /// directories for *equality* sees no collision at all, so
    /// `remove_dir_all` on alpha's takes the live agent's subtree with it
    /// and the report calls it a clean removal.
    #[test]
    fn a_nested_workspace_of_another_live_agent_is_never_deleted() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        for (name, ws) in [("alpha", "shared"), ("beta", "shared/team")] {
            let mut entry = AgentEntry {
                id: AgentId::from_name(name),
                name: name.to_string(),
                state: AgentState::Running,
                ..Default::default()
            };
            entry.manifest.workspace = Some(PathBuf::from(ws));
            substrate.save_agent(&entry).unwrap();
        }
        let alpha_ws = home.path().join("workspaces/agents/shared");
        let beta_ws = alpha_ws.join("team");
        seed_workspace(&alpha_ws);
        seed_workspace(&beta_ws);

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.roster_entry_removed);
        assert!(
            !outcome.report.workspace_removed,
            "a workspace containing a live agent's must never be deleted"
        );
        assert!(outcome.report.workspace_shared);
        assert!(
            beta_ws.join("agent.toml").exists(),
            "beta's nested workspace must survive alpha's purge"
        );
    }

    /// `[workspaces]` in a manifest is the second way an agent gets a
    /// directory, resolved by `ensure_named_workspaces` against the
    /// workspaces root — and nothing forbids one under `agents/`. A guard
    /// that reads only `manifest.workspace` never sees beta's declaration,
    /// so purging alpha deletes a directory beta is actively pointed at.
    #[test]
    fn another_agents_named_workspace_under_this_one_is_never_deleted() {
        use librefang_types::agent::WorkspaceDecl;

        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        substrate
            .save_agent(&AgentEntry {
                id: AgentId::from_name("alpha"),
                name: "alpha".to_string(),
                state: AgentState::Running,
                ..Default::default()
            })
            .unwrap();
        let mut beta = AgentEntry {
            id: AgentId::from_name("beta"),
            name: "beta".to_string(),
            state: AgentState::Running,
            ..Default::default()
        };
        beta.manifest.workspaces.insert(
            "library".to_string(),
            WorkspaceDecl {
                path: Some(PathBuf::from("agents/alpha/library")),
                ..WorkspaceDecl::default()
            },
        );
        substrate.save_agent(&beta).unwrap();
        let alpha_ws = home.path().join("workspaces/agents/alpha");
        let shared_library = alpha_ws.join("library");
        seed_workspace(&alpha_ws);
        std::fs::create_dir_all(&shared_library).unwrap();
        std::fs::write(shared_library.join("notes.md"), "beta's").unwrap();

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.roster_entry_removed);
        assert!(
            !outcome.report.workspace_removed,
            "a directory a live agent declares as a named workspace must survive"
        );
        assert!(outcome.report.workspace_shared);
        assert!(shared_library.join("notes.md").exists());
    }

    /// The unattributable-orphan scan is a diagnostic: it deletes nothing,
    /// and no other step's safety depends on its answer. Letting its error
    /// into `plan.failures` therefore made a *report* line able to veto the
    /// whole command, with no flag to get past it.
    ///
    /// A BLOB in an agent-scoping column is the cheap way to provoke that:
    /// TEXT affinity rewrites an integer to text on insert, but never a
    /// BLOB, so reading the id back out as a `String` raises
    /// `InvalidColumnType` — and the scan no longer reads it back out.
    #[test]
    fn a_failing_orphan_scan_never_vetoes_the_purge() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        // The name-derived id, so the candidate loop finds it live and
        // `has_agent_rows` — which walks the same tables — is never reached.
        seed_agent_rows(&substrate, "alpha", AgentId::from_name("alpha"));
        substrate
            .pool()
            .get()
            .unwrap()
            .execute(
                "INSERT INTO memories (id, agent_id, content, source, created_at, accessed_at) \
                 VALUES ('blob-agent-id', x'deadbeef', 'x', 'test', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(
            outcome.failures.is_empty(),
            "a diagnostic scan must not veto the purge: {:?}",
            outcome.failures
        );
        assert!(
            outcome.report.roster_entry_removed,
            "the agent's own data must still be removed"
        );
        assert!(
            outcome.report.other_orphans_present,
            "the unattributable row is still there and must be reported"
        );
    }

    /// Spawn rewrites a hand agent's `workspace` to the absolute,
    /// already-resolved `<workspaces>/hands/<hand>/<role>` path. Validating
    /// that against the `agents/` root alone (rather than the whole
    /// workspaces root) used to fail `starts_with` and report "no manifest
    /// survives" for a hand agent whose manifest and roster entry are both
    /// alive.
    #[test]
    fn a_hand_agents_absolute_workspace_override_is_resolved_not_reported_missing() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("researcher");
        let hand_workspace = home.path().join("workspaces/hands/team-a/researcher");
        let mut entry = AgentEntry {
            id,
            name: "researcher".to_string(),
            state: AgentState::Running,
            ..Default::default()
        };
        entry.manifest.workspace = Some(hand_workspace.clone());
        substrate.save_agent(&entry).unwrap();
        seed_workspace(&hand_workspace);

        let outcome = purge_agent(&substrate, &cfg_for(&home), "researcher");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(
            outcome.report.workspace_removed,
            "a hand agent's absolute workspace path must resolve"
        );
        assert!(!outcome.report.workspace_unresolved);
        assert!(!hand_workspace.exists());
    }

    /// Cron jobs and event triggers key on `AgentId`, and the default id is
    /// derived deterministically from the agent's name — so leaving them
    /// behind means a new agent later spawned under the purged name
    /// inherits the old agent's schedule and event wiring outright.
    #[test]
    fn cron_and_trigger_residue_is_removed_so_a_recreated_agent_does_not_inherit_it() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("alpha");
        seed_agent_rows(&substrate, "alpha", id);
        let cfg = cfg_for(&home);

        let cron = CronScheduler::new(home.path(), cfg.max_cron_jobs);
        cron.add_job(
            librefang_types::scheduler::CronJob {
                id: librefang_types::scheduler::CronJobId::new(),
                agent_id: id,
                name: "test-job".into(),
                enabled: true,
                schedule: librefang_types::scheduler::CronSchedule::Every { every_secs: 3600 },
                action: librefang_types::scheduler::CronAction::SystemEvent {
                    text: "ping".into(),
                },
                delivery: librefang_types::scheduler::CronDelivery::None,
                delivery_targets: Vec::new(),
                peer_id: None,
                session_mode: None,
                created_at: chrono::Utc::now(),
                last_run: None,
                next_run: None,
                owner: None,
            },
            false,
        )
        .unwrap();
        cron.persist().unwrap();

        let triggers = TriggerEngine::with_config(&cfg.triggers, home.path());
        triggers
            .register(
                id,
                crate::triggers::TriggerPattern::System,
                "wake up".into(),
                0,
            )
            .unwrap();
        triggers.persist().unwrap();

        let outcome = purge_agent(&substrate, &cfg, "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(
            outcome.report.cron_jobs_removed,
            "cron residue must be reported removed"
        );
        assert!(
            outcome.report.trigger_jobs_removed,
            "trigger residue must be reported removed"
        );

        let reloaded_cron = CronScheduler::new(home.path(), cfg.max_cron_jobs);
        reloaded_cron.load().unwrap();
        assert!(
            reloaded_cron.list_jobs(id).is_empty(),
            "a recreated 'alpha' must not inherit the purged agent's cron jobs"
        );

        let reloaded_triggers = TriggerEngine::with_config(&cfg.triggers, home.path());
        reloaded_triggers.load().unwrap();
        assert!(
            reloaded_triggers.list_agent_triggers(id).is_empty(),
            "a recreated 'alpha' must not inherit the purged agent's triggers"
        );
    }

    /// `channel_instance_defaults` / `conversation_bindings` key on the
    /// agent's *name*, so a re-created agent under the purged name would
    /// otherwise inherit its channel/conversation routing.
    #[test]
    fn channel_binding_residue_is_removed_by_name() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        substrate
            .channel_bindings()
            .seed_instance_default("tg-bot", "alpha")
            .unwrap();
        substrate
            .channel_bindings()
            .set_conversation_binding("tg-bot", "peer-1", "alpha", "user:admin")
            .unwrap();

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.channel_bindings_removed);
        assert_eq!(
            substrate
                .channel_bindings()
                .instance_default("tg-bot")
                .unwrap(),
            None
        );
        assert_eq!(
            substrate
                .channel_bindings()
                .conversation_binding("tg-bot", "peer-1")
                .unwrap(),
            None
        );
    }

    /// `AgentIdentityRegistry::purge` itself only warns on a persist
    /// failure and still returns `Some` — this pins that `purge_agent`
    /// re-checks the persist result itself, rather than trusting that
    /// return value, so a write failure is a reported failure and
    /// `identity_record_removed` stays false instead of claiming success.
    #[cfg(unix)]
    #[test]
    fn identity_persist_failure_is_reported_not_silently_swallowed() {
        use std::os::unix::fs::PermissionsExt;

        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::new();
        substrate
            .save_agent(&AgentEntry {
                id,
                name: "alpha".to_string(),
                state: AgentState::Running,
                ..Default::default()
            })
            .unwrap();
        AgentIdentityRegistry::load(home.path()).register_if_absent("alpha", id);
        assert!(
            AgentIdentityRegistry::load(home.path())
                .get("alpha")
                .is_some(),
            "seed failed"
        );

        // Strip write permission on the home directory so persist()'s
        // tmp-file create (or the rename) fails, while the read that
        // `plan_purge` performs a moment later still succeeds — reading an
        // existing file by name needs no write permission on its parent.
        //
        // `persist_tmp_path` mixes a timestamp into the temporary file's
        // name and the destination has to stay readable for the plan, so
        // permissions are the only lever here — and root (or anything
        // holding CAP_DAC_OVERRIDE, which is how `Dockerfile.rust-dev`
        // runs) ignores the write bit and would make every assertion below
        // pass against a persist that quietly succeeded. Confirm the
        // sabotage actually bites rather than reporting a green that
        // measured nothing.
        let original_perms = std::fs::metadata(home.path()).unwrap().permissions();
        std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let probe = home.path().join("write-bit-probe");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            std::fs::set_permissions(home.path(), original_perms).unwrap();
            eprintln!(
                "skipped: this process writes to a 0o500 directory, so a persist failure \
                 cannot be provoked here"
            );
            return;
        }

        let outcome = purge_agent(&substrate, &cfg_for(&home), "alpha");

        // Restore before any assertion can panic and skip cleanup.
        std::fs::set_permissions(home.path(), original_perms).unwrap();

        assert!(
            !outcome.failures.is_empty(),
            "a persist failure must be surfaced, not swallowed"
        );
        assert!(!outcome.report.identity_record_removed);
        assert_eq!(
            AgentIdentityRegistry::load(home.path()).get("alpha"),
            Some(id),
            "the on-disk record must survive an unpersisted purge"
        );
    }
}
