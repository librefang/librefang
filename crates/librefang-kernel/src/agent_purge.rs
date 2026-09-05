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
use crate::kernel::workspace_setup::resolved_workspace_dir;
use librefang_memory::agent_tables::AGENT_SCOPED_TABLES;
use librefang_memory::MemorySubstrate;
use librefang_types::agent::AgentId;
use librefang_types::agent_type_store::{agent_type_path_in, validate_agent_type_name};
use librefang_types::config::KernelConfig;
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
    /// An agent-type template of the same name was deleted.
    pub agent_type_removed: bool,
}

impl PurgeReport {
    /// True when nothing at all was found to remove — the caller asked to
    /// purge something that leaves no trace anywhere.
    pub fn is_empty(&self) -> bool {
        !(self.roster_entry_removed
            || self.orphaned_data_removed
            || self.identity_record_removed
            || self.workspace_removed
            || self.agent_type_removed)
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

    let registry = AgentIdentityRegistry::load(home);
    let registry_record = registry.get(agent_name).is_some();

    let mut preview = PurgeReport::default();
    let mut roster_agent_id = None;
    let mut orphan_agent_ids = Vec::new();

    let roster_entry = entries.iter().find(|e| e.name == agent_name);
    match roster_entry {
        Some(entry) => {
            preview.roster_entry_removed = true;
            roster_agent_id = Some(entry.id);
        }
        None => {
            // Orphan path — the whole reason this module exists: the roster
            // entry is gone but its rows are not. Recover the agent id from
            // its name. Two sources: the canonical-UUID registry (covers
            // agents spawned with a random id) and the deterministic
            // name-derived UUID. A candidate that any live roster entry
            // holds (an agent renamed since spawn, keeping its id) is never
            // touched — its data belongs to a running agent.
            let mut candidates: Vec<AgentId> = Vec::new();
            if let Some(id) = registry.get(agent_name) {
                candidates.push(id);
            }
            let derived = AgentId::from_name(agent_name);
            if !candidates.contains(&derived) {
                candidates.push(derived);
            }
            for id in candidates {
                if entries.iter().any(|e| e.id == id) {
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
        }
    }
    preview.identity_record_removed = registry_record;

    // The workspaces root is configurable (`workspaces_dir`) and an agent can
    // carry its own `workspace` override, so neither the root nor the leaf is
    // safe to spell out here: both come from the same helpers spawn used to
    // create the directory in the first place.
    let workspaces_root = cfg.effective_agent_workspaces_dir();
    let (workspace, workspace_unresolved) = match roster_entry {
        Some(entry) => match resolved_workspace_dir(
            &workspaces_root,
            entry.manifest.workspace.clone(),
            agent_name,
            entry.id,
        ) {
            Ok(dir) => (dir.is_dir().then_some(dir), false),
            // The manifest names a workspace the resolver refuses (outside the
            // root, or with `..` in it). The agent has a workspace and we
            // cannot say where — report that rather than "there was none".
            Err(_) => (None, true),
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
            let dir = resolved_workspace_dir(&workspaces_root, None, agent_name, id)
                .ok()
                .filter(|d| d.is_dir());
            // An orphan whose default directory is empty is ambiguous: it
            // either never had a workspace or had one under an override we
            // cannot read. Say so, instead of reporting the flattering half.
            let unresolved = dir.is_none() && !orphan_agent_ids.is_empty();
            (dir, unresolved)
        }
    };
    if workspace.is_some() {
        preview.workspace_removed = true;
    }
    preview.workspace_unresolved = workspace_unresolved;

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
        // Not a removal, but the same caveat holds whether or not the run is a
        // dry one, so it travels with the report the caller prints.
        workspace_unresolved: plan.preview.workspace_unresolved,
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

    // Drop the name → UUID binding last-but-not-unconditionally: the kernel
    // skips it when the roster row could not be removed (#5117) so the next
    // boot never loads a roster row whose name the registry no longer knows.
    // Same rule here: only unbind when every cascade above succeeded.
    if plan.registry_record && failures.is_empty() {
        let registry = AgentIdentityRegistry::load(home);
        if registry.purge(agent_name).is_some() {
            report.identity_record_removed = true;
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
}
