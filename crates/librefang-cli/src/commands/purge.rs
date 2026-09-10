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

pub(crate) fn cmd_purge(
    config: Option<&Path>,
    agent: &str,
    yes: bool,
    dry_run: bool,
    force: bool,
) -> i32 {
    let config = match resolve_config(config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", i18n::t_args("purge-failed-config", &[("error", &e)]));
            return 1;
        }
    };
    let home = config.home_dir.clone();

    // A running daemon keeps this agent alive in memory: it re-persists a
    // resurrected roster entry on its next save, and a later respawn can
    // land on a different UUID once the identity record is gone too. Only
    // the destructive path needs the daemon gone — a dry run touches
    // nothing the daemon could race against, and `--force` is the explicit
    // override for an operator who has already stopped the daemon by other
    // means (or knows `daemon.json` is stale).
    //
    // The probe is behind a closure because it is an HTTP round trip with a
    // 1 s connect and 2 s read timeout: passing it as an argument made
    // `--dry-run` and `--force` wait for a value the guard discards on its
    // first line.
    if let Some(base) = daemon_running_guard(dry_run, force, || {
        crate::commands::common::find_daemon_in_home(&home)
    }) {
        eprintln!(
            "{}",
            i18n::t_args("purge-failed-daemon-running", &[("url", &base)])
        );
        return 1;
    }

    let db = purge_db_path(&config);
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

    purge_with(&substrate, &config, &db, agent, dry_run, |_| {
        // On a non-TTY stdin the prompt reads EOF and answers "no", so --yes
        // is effectively required there — exactly the gate the review asked
        // for.
        yes || prompt_yes_no(&i18n::t("label-confirm-prompt"), false)
    })
}

/// Where the purge target's SQLite database lives.
///
/// Mirrors the resolution `monitoring::cmd_audit_reset` already uses:
/// `[memory] sqlite_path` wins outright, falling back to
/// `data_dir/librefang.db`. Hardcoding `<home>/data/librefang.db` instead
/// would, for an install with a custom `data_dir` or `sqlite_path`, either
/// miss the live database (exit 1) or silently open a stale one at the
/// default path and report success while the live rows survive untouched.
fn purge_db_path(config: &KernelConfig) -> std::path::PathBuf {
    config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| config.data_dir.join("librefang.db"))
}

/// Whether the destructive path must refuse to run because a daemon is
/// holding this home directory.
///
/// A dry run and an explicit `--force` both bypass the refusal — a dry run
/// touches nothing the daemon could race against, and `force` is the
/// operator's explicit override — so `find_daemon` is only called when its
/// answer can change the outcome.
fn daemon_running_guard(
    dry_run: bool,
    force: bool,
    find_daemon: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if dry_run || force {
        return None;
    }
    find_daemon()
}

/// Resolve the configuration that names the purge target, refusing to guess.
///
/// The config decides which database and which workspaces root the command
/// deletes from, so a config that cannot be loaded is not a degraded mode — it
/// means the target is unknown. `load_config` answers a failure with
/// `KernelConfig::default()`, and for `--config /srv/instance-b/config.toml` a
/// TOML error in that file would therefore retarget the deletion at the default
/// installation and purge its unrelated agent of the same name. Substituting a
/// target is not something a warning can make safe, which is why this uses the
/// strict loader: `try_load_config` returns `Err` for a read failure, a TOML
/// syntax error, a broken `include` chain, a failed migration, a deserialize
/// mismatch and an unknown field under `strict_config`, where `load_config`
/// falls back to defaults for all but the last three.
///
/// The one tolerated absence is a config file that is not there at the *default*
/// location and was not asked for by path. Nothing is substituted in that case:
/// the defaults describe the same default installation the operator meant, and
/// an install that never wrote a `config.toml` still has data worth cleaning up.
/// A default config that exists but cannot be read is a failure like any other —
/// it may well be the file that points `home_dir` somewhere else.
fn resolve_config(explicit: Option<&Path>) -> Result<KernelConfig, String> {
    let path = explicit
        .map(Path::to_path_buf)
        .unwrap_or_else(librefang_kernel::config::default_config_path);
    if explicit.is_none() && !path.exists() {
        return Ok(KernelConfig::default());
    }
    librefang_kernel::config::try_load_config(&path)
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
    db: &Path,
    agent: &str,
    dry_run: bool,
    confirm: impl FnOnce(&PurgeReport) -> bool,
) -> i32 {
    // Name the database before describing the plan. Removal categories say what
    // kind of thing goes; only the path says *whose*, which is the difference
    // between confirming a cleanup and confirming it against the wrong
    // installation.
    //
    // The home directory goes with it because the database no longer names
    // everything the command deletes: cron jobs (`<home>/data/cron_jobs.json`),
    // triggers (`<home>/trigger_jobs.json`), the agent-type template and the
    // workspaces root all hang off `home_dir`, while the database is resolved
    // through `sqlite_path` / `data_dir` — which this command deliberately
    // decoupled. On a relocated install those are two unrelated trees, and one
    // path would have the operator confirm deletions in the other sight unseen.
    println!(
        "{}",
        i18n::t_args(
            "purge-database-line",
            &[("path", &db.display().to_string())]
        )
    );
    println!(
        "{}",
        i18n::t_args(
            "purge-home-line",
            &[("path", &config.home_dir.display().to_string())]
        )
    );

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
    for line in outcome_lines(agent, report, failures, header) {
        println!("{line}");
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

/// The report as the localized stdout lines it prints as, so what the
/// operator is told about a given plan is assertable rather than only
/// observable by running the command and reading the terminal.
fn outcome_lines(
    agent: &str,
    report: &PurgeReport,
    failures: &[String],
    header: &str,
) -> Vec<String> {
    // `workspace_shared` and `other_orphans_present` are caveats, not
    // removals, so `PurgeReport::is_empty()` deliberately excludes them —
    // but that means a report with nothing to *remove* can still have
    // something worth telling the operator, and the "nothing to purge"
    // fast path must not swallow it.
    let has_caveats = report.workspace_shared || report.other_orphans_present;
    if report.is_empty() && !has_caveats {
        // Failures print as their own error lines; claiming there was
        // nothing to purge on top of them would contradict them.
        return if failures.is_empty() {
            vec![i18n::t_args("purge-nothing-to-purge", &[("agent", agent)])]
        } else {
            Vec::new()
        };
    }

    // A caveat is not a removal. `header` announces one ("About to
    // permanently purge 'alpha':", "Purged 'alpha':"), so a plan whose only
    // content is a caveat would promise a deletion, print the note, then
    // prompt for nothing and exit 0 having touched nothing at all. The
    // "nothing to purge" line carries the caveats in that case.
    let announcement = if report.is_empty() && failures.is_empty() {
        "purge-nothing-to-purge"
    } else {
        header
    };
    let mut lines = vec![i18n::t_args(announcement, &[("agent", agent)])];
    for (present, key) in [
        (report.roster_entry_removed, "purge-removed-roster-entry"),
        (report.orphaned_data_removed, "purge-removed-orphaned-data"),
        (
            report.identity_record_removed,
            "purge-removed-identity-record",
        ),
        (report.workspace_removed, "purge-removed-workspace"),
        (report.workspace_unresolved, "purge-workspace-unresolved"),
        (report.workspace_shared, "purge-workspace-shared"),
        (report.agent_type_removed, "purge-removed-agent-type"),
        (report.cron_jobs_removed, "purge-removed-cron-jobs"),
        (report.trigger_jobs_removed, "purge-removed-trigger-jobs"),
        (
            report.channel_bindings_removed,
            "purge-removed-channel-bindings",
        ),
        (report.other_orphans_present, "purge-other-orphans-present"),
    ] {
        if present {
            lines.push(i18n::t(key));
        }
    }
    lines
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

    /// The old code joined `home_dir` and `"data"` directly, ignoring both
    /// `data_dir` and `[memory] sqlite_path`. An installation whose
    /// `data_dir` was moved elsewhere in `config.toml` would then have
    /// purge look at (or create) an empty database at the default path
    /// while the live one, with the agent's actual rows, sat untouched.
    #[test]
    fn db_path_follows_configured_data_dir_not_a_hardcoded_home_join() {
        let cfg = KernelConfig {
            home_dir: std::path::PathBuf::from("/home-dir-is-irrelevant-here"),
            data_dir: std::path::PathBuf::from("/configured/elsewhere"),
            ..KernelConfig::default()
        };
        assert_eq!(
            purge_db_path(&cfg),
            std::path::PathBuf::from("/configured/elsewhere/librefang.db")
        );
    }

    /// `[memory] sqlite_path` is the most specific setting and must win
    /// outright over `data_dir`, matching `monitoring::cmd_audit_reset`.
    #[test]
    fn db_path_prefers_explicit_sqlite_path_over_data_dir() {
        let mut cfg = KernelConfig {
            home_dir: std::path::PathBuf::from("/home"),
            data_dir: std::path::PathBuf::from("/home/data"),
            ..KernelConfig::default()
        };
        cfg.memory.sqlite_path = Some(std::path::PathBuf::from("/custom/db/path.sqlite"));
        assert_eq!(
            purge_db_path(&cfg),
            std::path::PathBuf::from("/custom/db/path.sqlite")
        );
    }

    /// One plan shape has caveats and no removals: nothing is attributable
    /// to the name, but the installation holds orphan rows under an id no
    /// name can be recovered for. `purge_with` then returns early on the
    /// empty preview — so announcing "About to permanently purge 'alpha':"
    /// promises a deletion that is never prompted for and never happens.
    #[test]
    fn a_caveat_only_plan_does_not_announce_a_deletion() {
        let report = PurgeReport {
            other_orphans_present: true,
            ..PurgeReport::default()
        };

        let lines = outcome_lines("alpha", &report, &[], "purge-confirm-header");

        assert_eq!(
            lines,
            vec![
                i18n::t_args("purge-nothing-to-purge", &[("agent", "alpha")]),
                i18n::t("purge-other-orphans-present"),
            ],
            "a caveat must be reported under the line that says nothing was removed"
        );
    }

    /// And the header is still the header when there is a removal under it.
    #[test]
    fn a_plan_with_removals_announces_them_under_the_header() {
        let report = PurgeReport {
            agent_type_removed: true,
            other_orphans_present: true,
            ..PurgeReport::default()
        };

        let lines = outcome_lines("alpha", &report, &[], "purge-confirm-header");

        assert_eq!(
            lines.first().map(String::as_str),
            Some(i18n::t_args("purge-confirm-header", &[("agent", "alpha")]).as_str())
        );
    }

    /// A daemon-detection closure that fails the test if it is ever called.
    fn never_probed() -> Option<String> {
        panic!("the daemon probe ran on a path that discards its answer")
    }

    #[test]
    fn daemon_running_guard_refuses_the_destructive_path_when_a_daemon_is_up() {
        assert_eq!(
            daemon_running_guard(false, false, || Some("http://127.0.0.1:4545".to_string())),
            Some("http://127.0.0.1:4545".to_string())
        );
    }

    /// Both bypasses skip the probe rather than paying for it and throwing
    /// the answer away: it is an HTTP round trip with a 1 s connect and 2 s
    /// read timeout, on the two paths that are supposed to be the quick ones.
    #[test]
    fn daemon_running_guard_allows_a_dry_run_without_probing() {
        assert_eq!(daemon_running_guard(true, false, never_probed), None);
    }

    #[test]
    fn daemon_running_guard_allows_an_explicit_force_override_without_probing() {
        assert_eq!(daemon_running_guard(false, true, never_probed), None);
    }

    #[test]
    fn daemon_running_guard_is_a_noop_when_no_daemon_is_up() {
        assert_eq!(daemon_running_guard(false, false, || None), None);
    }

    /// The destructive path asks the operator about a plan, so a purge that
    /// would remove nothing asks nothing. It used to print the static warning
    /// and prompt regardless, because it never planned before prompting.
    #[test]
    fn a_purge_that_removes_nothing_never_prompts() {
        let home = tempfile::tempdir().unwrap();
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let db = home.path().join("data").join("librefang.db");
        let code = purge_with(&substrate, &cfg_for(&home), &db, "nobody", false, |_| {
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

        let db = home.path().join("data").join("librefang.db");
        let code = purge_with(&substrate, &cfg_for(&home), &db, "alpha", false, |plan| {
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

    /// Serializes the two env-var-mutating tests below.
    /// `LIBREFANG_HOME` is process-wide state, and the whole point of these
    /// tests is what the command resolves it to.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Point `LIBREFANG_HOME` at `home` for the duration of `body`, restoring
    /// the previous value afterwards.
    fn with_librefang_home<T>(home: &Path, body: impl FnOnce() -> T) -> T {
        let previous = std::env::var("LIBREFANG_HOME").ok();
        // SAFETY: every caller holds `env_lock`, so no other thread in this
        // process is reading or writing the variable concurrently.
        unsafe { std::env::set_var("LIBREFANG_HOME", home) };
        let out = body();
        // SAFETY: see above — still under `env_lock`.
        unsafe {
            match previous {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
        }
        out
    }

    /// A default installation carrying an agent-type template for `agent`, and
    /// a database for `cmd_purge` to find. Returns the template's path so a
    /// test can assert it survived.
    fn default_installation_with(home: &Path, agent: &str) -> std::path::PathBuf {
        let types = librefang_types::agent_type_store::agent_types_dir_in(home);
        std::fs::create_dir_all(&types).unwrap();
        let agent_type = types.join(format!("{agent}.toml"));
        std::fs::write(&agent_type, "x").unwrap();
        let data = home.join("data");
        std::fs::create_dir_all(&data).unwrap();
        drop(MemorySubstrate::open(&data.join("librefang.db"), PURGE_DECAY_RATE).unwrap());
        agent_type
    }

    /// A config that cannot be loaded must abort the purge, not silently
    /// retarget it at the default installation. `--config <broken>` used to
    /// warn and fall back to `KernelConfig::default()`, whose `home_dir` is the
    /// default installation — so a TOML error deleted an unrelated agent that
    /// merely shared a name with the intended one.
    #[test]
    fn an_unloadable_explicit_config_purges_nothing_from_the_default_installation() {
        let _guard = env_lock();
        let default_home = tempfile::tempdir().unwrap();
        let agent_type = default_installation_with(default_home.path(), "worker");

        let broken = tempfile::tempdir().unwrap();
        let broken_config = broken.path().join("config.toml");
        std::fs::write(&broken_config, "this is not = = valid toml").unwrap();

        let code = with_librefang_home(default_home.path(), || {
            cmd_purge(Some(&broken_config), "worker", true, false, false)
        });

        assert_eq!(code, 1, "an unloadable config must fail the command");
        assert!(
            agent_type.exists(),
            "purge substituted the default installation as its target"
        );
    }

    /// The same substitution, through the failure mode that survives a naive
    /// fix: `load_config` answers a *missing* file with `Ok(defaults)` rather
    /// than `Err`, so a `--config` pointing at a path that does not exist —
    /// a typo, an unmounted volume — never reaches the error branch at all.
    #[test]
    fn a_missing_explicit_config_purges_nothing_from_the_default_installation() {
        let _guard = env_lock();
        let default_home = tempfile::tempdir().unwrap();
        let agent_type = default_installation_with(default_home.path(), "worker");

        let elsewhere = tempfile::tempdir().unwrap();
        let absent = elsewhere.path().join("never-written.toml");

        let code = with_librefang_home(default_home.path(), || {
            cmd_purge(Some(&absent), "worker", true, false, false)
        });

        assert_eq!(code, 1, "a config that is not there must fail the command");
        assert!(
            agent_type.exists(),
            "purge substituted the default installation as its target"
        );
    }

    /// `purge_db_path`'s own tests pass with `cmd_purge` still joining
    /// `<home>/data/librefang.db`, because a default installation puts both
    /// paths in the same place. This one separates them: `data_dir` points
    /// at a tree that shares nothing with `home_dir`, and the only database
    /// in the installation lives there. Against the old join the command
    /// exits 1 on "no database at ..." and deletes nothing.
    /// A `config.toml` naming both roots, so a `cmd_purge` test can point
    /// the command at a temporary installation through `--config` alone.
    /// `LIBREFANG_HOME` is process-wide and the tests that set it have to
    /// serialize on `env_lock`; a config file needs neither, and leaves no
    /// window in which an unrelated test could read the variable.
    ///
    /// Both roots are spelled out because `data_dir` is `#[serde(default)]`:
    /// naming only `home_dir` would leave the command pointed at the real
    /// installation's database.
    fn config_naming(home: &Path, data_dir: &Path) -> std::path::PathBuf {
        let path = home.join("config.toml");
        // TOML literal strings: a Windows path in a basic string would read
        // its separators as escapes.
        std::fs::write(
            &path,
            format!(
                "home_dir = '{}'\ndata_dir = '{}'\n",
                home.display(),
                data_dir.display()
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn cmd_purge_opens_the_database_data_dir_points_at_not_a_home_join() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();

        let types = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        std::fs::create_dir_all(&types).unwrap();
        let agent_type = types.join("worker.toml");
        std::fs::write(&agent_type, "x").unwrap();
        drop(
            MemorySubstrate::open(&elsewhere.path().join("librefang.db"), PURGE_DECAY_RATE)
                .unwrap(),
        );

        let config_path = config_naming(home.path(), elsewhere.path());
        let code = cmd_purge(Some(&config_path), "worker", true, false, false);

        assert_eq!(code, 0, "the configured database was not found");
        assert!(!agent_type.exists(), "the agent-type template survived");
    }

    /// A socket answering `/api/health` with 200, on an ephemeral port, for
    /// as long as the test process lives. `cmd_purge` reaches the guard
    /// through the real `find_daemon_in_home`, which probes over HTTP, so
    /// pinning the guard's *call site* — as opposed to the helper in
    /// isolation, which stays green when the call site is deleted — needs a
    /// port something can actually connect to.
    fn serve_health() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local_addr").to_string();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                use std::io::{Read as _, Write as _};
                let _ = stream.read(&mut [0u8; 1024]);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            }
        });
        addr
    }

    /// A running daemon holds this installation's data in memory and
    /// re-persists a resurrected roster entry on its next save, so the
    /// destructive path must refuse — and `--force` is the documented way
    /// past it for an operator who stopped the daemon by other means.
    #[test]
    fn cmd_purge_refuses_while_a_daemon_answers_in_this_home_unless_forced() {
        let home = tempfile::tempdir().unwrap();
        let agent_type = default_installation_with(home.path(), "worker");
        std::fs::write(
            home.path().join("daemon.json"),
            format!(
                r#"{{"pid":4242,"listen_addr":"{}","started_at":"1970-01-01T00:00:00Z","version":"0.0.0-test","platform":"test"}}"#,
                serve_health()
            ),
        )
        .unwrap();
        let config_path = config_naming(home.path(), &home.path().join("data"));

        let refused = cmd_purge(Some(&config_path), "worker", true, false, false);
        assert_eq!(refused, 1, "a live daemon must fail the destructive path");
        assert!(
            agent_type.exists(),
            "the refusal must happen before anything is deleted"
        );

        let forced = cmd_purge(Some(&config_path), "worker", true, false, true);
        assert_eq!(forced, 0, "--force is the operator's explicit override");
        assert!(!agent_type.exists(), "--force must actually purge");
    }

    /// A config file that is simply absent from the default location is not a
    /// failure: the defaults describe the very installation the operator meant,
    /// so nothing is substituted and the command still works on an install that
    /// never wrote a `config.toml`.
    #[test]
    fn a_missing_default_config_still_purges_the_default_installation() {
        let _guard = env_lock();
        let default_home = tempfile::tempdir().unwrap();
        let agent_type = default_installation_with(default_home.path(), "worker");

        let code = with_librefang_home(default_home.path(), || {
            cmd_purge(None, "worker", true, false, false)
        });

        assert_eq!(code, 0, "a config-less installation is still purgeable");
        assert!(!agent_type.exists(), "the agent-type template survived");
    }
}
