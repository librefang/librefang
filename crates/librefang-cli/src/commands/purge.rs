//! `librefang purge --agent <name>` — remove every trace of an agent.
//!
//! Thin wrapper over `librefang_kernel::agent_purge`, the shared
//! implementation (written to be shared; the CLI is the only caller today).
//! Opens the database directly so the command works with no daemon running —
//! which is the usual situation when cleaning up from a previous partial delete.

use crate::commands::common::prompt_yes_no;
use crate::i18n;
use librefang_kernel::agent_purge::PurgeReport;
use librefang_memory::MemorySubstrate;
use librefang_types::config::KernelConfig;
use std::path::Path;

/// Memory decay rate handed to `MemorySubstrate::open` (0.0 = no decay,
/// 1.0 = aggressive decay; the kernel's own default is 0.1). Purge only
/// deletes rows and never runs decay or consolidation, so the value never
/// fires — the substrate just requires one.
const PURGE_DECAY_RATE: f32 = 0.01;

pub(crate) fn cmd_purge(config: Option<&Path>, agent: &str, yes: bool, dry_run: bool) -> i32 {
    // The config decides where the workspaces root is (`workspaces_dir`), so
    // purge has to read it rather than assume `{home}/workspaces/agents`.
    // `load_config` already reports a parse failure on stderr (#5186); falling
    // back to the defaults keeps the command usable on a broken config, which
    // is a state a cleanup command should expect to meet.
    let config = librefang_kernel::config::load_config(config).unwrap_or_else(|e| {
        eprintln!(
            "{}",
            i18n::t_args(
                "common-warning-config-default",
                &[("error", &e.to_string())]
            )
        );
        KernelConfig::default()
    });
    let home = config.home_dir.clone();
    let db = home.join("data").join("librefang.db");
    if !db.exists() {
        eprintln!(
            "{}",
            i18n::t_args(
                "purge-failed-no-database",
                &[("path", &db.display().to_string())]
            )
        );
        return 1;
    }

    let substrate = match MemorySubstrate::open(&db, PURGE_DECAY_RATE) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{}",
                i18n::t_args("purge-failed-open-database", &[("error", &e.to_string())])
            );
            return 1;
        }
    };

    purge_with(&substrate, &config, agent, dry_run, |_| {
        // On a non-TTY stdin the prompt reads EOF and answers "no", so --yes
        // is effectively required there — exactly the gate the review asked
        // for.
        yes || prompt_yes_no(&i18n::t("label-confirm-prompt"), false)
    })
}

/// The command's decisions, with the database and the confirmation both
/// supplied by the caller so a test can drive them.
///
/// `confirm` is called with the planned report, and only when there is
/// something to purge: a prompt over a plan that removes nothing is a
/// question with one honest answer.
fn purge_with(
    substrate: &MemorySubstrate,
    config: &KernelConfig,
    agent: &str,
    dry_run: bool,
    confirm: impl FnOnce(&PurgeReport) -> bool,
) -> i32 {
    // Plan first in both directions. The destructive path used to print a
    // static warning listing everything a purge *can* remove and prompt on
    // that, so the operator confirmed a template rather than what was about to
    // happen — and was prompted even when the answer changed nothing.
    let plan = librefang_kernel::agent_purge::plan_purge(substrate, config, agent);
    let header = if dry_run {
        "purge-dry-run-header"
    } else {
        "purge-confirm-header"
    };
    let code = print_outcome(agent, &plan.preview, &plan.failures, header);
    if dry_run || code != 0 || plan.preview.is_empty() {
        // A plan with failures is one `purge_agent` refuses to execute, and an
        // empty one has nothing to confirm.
        return code;
    }

    eprintln!(
        "{}",
        i18n::t_args("purge-confirm-warning", &[("agent", agent)])
    );
    if !confirm(&plan.preview) {
        eprintln!("{}", i18n::t("label-aborted"));
        return 1;
    }

    let outcome = librefang_kernel::agent_purge::purge_agent(substrate, config, agent);
    print_outcome(
        agent,
        &outcome.report,
        &outcome.failures,
        "purge-purged-header",
    )
}

/// Print the report as localized lines and the failures as localized error
/// lines. `header` picks the dry-run ("would purge"), the confirmation
/// ("about to purge") or the real ("purged") heading; returns the process exit
/// code (0 clean, 1 on any failure).
fn print_outcome(agent: &str, report: &PurgeReport, failures: &[String], header: &str) -> i32 {
    if report.is_empty() && failures.is_empty() {
        println!(
            "{}",
            i18n::t_args("purge-nothing-to-purge", &[("agent", agent)])
        );
        return 0;
    }
    if !report.is_empty() {
        println!("{}", i18n::t_args(header, &[("agent", agent)]));
        if report.roster_entry_removed {
            println!("{}", i18n::t("purge-removed-roster-entry"));
        }
        if report.orphaned_data_removed {
            println!("{}", i18n::t("purge-removed-orphaned-data"));
        }
        if report.identity_record_removed {
            println!("{}", i18n::t("purge-removed-identity-record"));
        }
        if report.workspace_removed {
            println!("{}", i18n::t("purge-removed-workspace"));
        }
        if report.workspace_unresolved {
            println!("{}", i18n::t("purge-workspace-unresolved"));
        }
        if report.agent_type_removed {
            println!("{}", i18n::t("purge-removed-agent-type"));
        }
    }
    for f in failures {
        eprintln!("{}", i18n::t_args("purge-failure-line", &[("error", f)]));
    }
    if failures.is_empty() {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn cfg_for(home: &tempfile::TempDir) -> KernelConfig {
        KernelConfig {
            home_dir: home.path().to_path_buf(),
            ..KernelConfig::default()
        }
    }

    /// The destructive path asks the operator about a plan, so a purge that
    /// would remove nothing asks nothing. It used to print the static warning
    /// and prompt regardless, because it never planned before prompting.
    #[test]
    fn a_purge_that_removes_nothing_never_prompts() {
        let home = tempfile::tempdir().unwrap();
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let code = purge_with(&substrate, &cfg_for(&home), "nobody", false, |_| {
            panic!("prompted over a plan that removes nothing")
        });

        assert_eq!(code, 0);
    }

    /// And when there is something to remove, what reaches the prompt is the
    /// plan itself — not a template listing everything a purge can touch.
    #[test]
    fn the_prompt_is_shown_the_planned_report() {
        let home = tempfile::tempdir().unwrap();
        let types = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        std::fs::create_dir_all(&types).unwrap();
        let agent_type = types.join("alpha.toml");
        std::fs::write(&agent_type, "x").unwrap();
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let seen = Cell::new(false);

        let code = purge_with(&substrate, &cfg_for(&home), "alpha", false, |plan| {
            seen.set(true);
            assert!(plan.agent_type_removed);
            assert!(!plan.roster_entry_removed);
            assert!(!plan.workspace_removed);
            false
        });

        assert!(seen.get(), "the plan never reached the prompt");
        assert_eq!(code, 1, "declining is not success");
        assert!(agent_type.exists(), "declining must not delete anything");
    }
}
