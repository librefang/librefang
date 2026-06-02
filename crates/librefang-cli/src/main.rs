//! LibreFang CLI — command-line interface for the LibreFang Agent OS.
//!
//! When a daemon is running (`librefang start`), the CLI talks to it over HTTP.
//! Otherwise, commands boot an in-process kernel (single-shot mode).

// The in-process agent loop's deeply-nested async future chain — now
// carrying the per-task held-agent-lock `scope` layer (#5125/#5126) —
// exceeds the default type-recursion limit when this binary crate is
// monomorphised. Matches the `librefang-kernel` / `librefang-api` crate
// roots.
#![recursion_limit = "256"]

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod acp;
mod cli;
mod commands;
mod desktop_install;
pub mod doctor;
mod http_client;
pub mod i18n;
mod launcher;
mod log_filter;
mod mcp;
pub mod progress;
pub mod table;
mod templates;
mod tui;
mod ui;

use clap::Parser;
// All other shared symbols (cli defs, common helpers, command groups, and the
// std/external short names) come through the command prelude glob, which is
// exempt from unused-import warnings as `main.rs` keeps shrinking.
use commands::prelude::*;
#[cfg(windows)]
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Global flag set by the Ctrl+C handler.
static CTRLC_PRESSED: AtomicBool = AtomicBool::new(false);
const INIT_DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../templates/init_default_config.toml");
const LOG_RETENTION_DAYS: u64 = 7;

/// Install a Ctrl+C handler that force-exits the process.
/// On Windows/MINGW, the default handler doesn't reliably interrupt blocking
/// `read_line` calls, so we explicitly call `process::exit`.
pub(crate) fn install_ctrlc_handler() {
    #[cfg(windows)]
    {
        extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
            if CTRLC_PRESSED.swap(true, Ordering::SeqCst) {
                // Second press: hard exit
                std::process::exit(130);
            }
            // First press: print message and exit cleanly
            let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\nInterrupted.\n");
            std::process::exit(0);
        }
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }

    #[cfg(not(windows))]
    {
        // On Unix, the default SIGINT handler already interrupts read_line
        // and terminates the process.
        let _ = &CTRLC_PRESSED;
    }
}

/// Wraps an inner `FormatEvent` impl so every emitted log line carries a
/// `trace_id=<32-hex>` suffix whenever the current tracing span is part of
/// an OpenTelemetry-traced flow (i.e. the OTel reload layer has been swapped
/// in by `init_otel_tracing` and the span has a valid trace context).
///
/// The trace_id sits at the **end** of the line as a logfmt-style structured
/// suffix rather than at the front. This keeps the human-readable
/// timestamp/level/message portion at the start of the line where readers
/// expect it, matching the convention that structured key=value fields
/// follow the unstructured prose of a log entry.
///
/// When telemetry is compiled out, the wrapper still exists but the
/// `cfg(feature = "telemetry")` block is empty — every call delegates to
/// the inner formatter unchanged, so non-telemetry builds see no behaviour
/// change. When telemetry is compiled in but no OTel context is active
/// (e.g. an early boot log before the reload swap, a CLI subcommand that
/// never started the API), the trace context is invalid and the suffix is
/// omitted — again the inner formatter's output is passed through verbatim.
///
/// The suffix uses bare logfmt `trace_id=<hex>` (no quotes) — the matching
/// `derivedFields` regex in `deploy/grafana/provisioning/datasources/loki.yml`
/// is `trace_id="?([0-9a-f]{32})"?`, which is anchored on the literal
/// `trace_id=` token rather than line position, so the suffix placement
/// resolves the same clickable trace link as a prefix would.
struct WithTraceId<F>(F);

impl<S, N, F> tracing_subscriber::fmt::format::FormatEvent<S, N> for WithTraceId<F>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
    F: tracing_subscriber::fmt::format::FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        #[allow(unused_mut)] mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        #[cfg(feature = "telemetry")]
        {
            use opentelemetry::trace::TraceContextExt;
            use tracing_opentelemetry::OpenTelemetrySpanExt;
            // Bind `cx` and the span via separate `let` bindings: `cx.span()`
            // returns a `SpanRef` that borrows from `cx`, and `span_context()`
            // returns a reference into the `SpanRef`'s inner state. Inlining
            // either one drops a temporary while a later borrow still needs
            // it (E0716 — verified with rustc 1.90 on this branch).
            let cx = tracing::Span::current().context();
            let span_ref = cx.span();
            let span_cx = span_ref.span_context();
            if span_cx.is_valid() {
                // Capture the inner formatter's output into a buffer so we
                // can append the trace_id suffix before the trailing newline.
                // The inner formatter writes its own `\n`; we strip it,
                // append ` trace_id=<hex>`, then re-emit a single newline.
                // Allocates one String per traced log event — acceptable,
                // and the no-OTel path below avoids the alloc entirely.
                let mut buf = String::new();
                self.0.format_event(
                    ctx,
                    tracing_subscriber::fmt::format::Writer::new(&mut buf),
                    event,
                )?;
                let trimmed = buf.trim_end_matches('\n');
                return writeln!(writer, "{trimmed} trace_id={:032x}", span_cx.trace_id());
            }
        }
        self.0.format_event(ctx, writer, event)
    }
}

fn init_tracing_stderr(log_level: &str) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    // One-shot CLI commands (status, stop, doctor, …) load config.toml as a
    // side effect. librefang_kernel::config emits INFO on every load and WARN
    // for every unknown field; in a CLI context those lines leak into the
    // user's stdout flow and make basic commands look broken. Keep them out
    // of the default stderr budget — users who set RUST_LOG explicitly still
    // see everything, and daemon/foreground boots route through a different
    // initialiser where the full log is expected.
    let user_set_rust_log = std::env::var("RUST_LOG").is_ok();
    // Per-target overrides applied unconditionally on top of the user-visible
    // level (and reapplied on every hot-reload via `install_with_baseline` —
    // see Codex P2-1 #3200). Stored as strings so the filter installer can
    // reparse them after a `log_level` swap; without that, a dashboard
    // "give me debug" toggle would silently drop these and flood operators
    // with kernel/runtime DEBUG noise that boot specifically masked.
    let baseline_directives: Vec<String> = if user_set_rust_log {
        // RUST_LOG is the explicit "I want full control" knob — don't layer
        // any opinionated overrides on top of it, and don't carry any across
        // reloads either.
        Vec::new()
    } else {
        vec![
            "librefang_kernel=warn".to_string(),
            "librefang_runtime=warn".to_string(),
            "librefang_extensions=warn".to_string(),
            "librefang_kernel::config=error".to_string(),
            "librefang_runtime::registry_sync=error".to_string(),
        ]
    };
    let mut env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));
    for d in &baseline_directives {
        // Per-string parse keeps the boot-time directive list and the
        // reload-time directive list literally identical.
        env_filter = env_filter.add_directive(d.parse().expect("baseline directive must parse"));
    }

    // Compact stderr format: in a one-shot CLI context the user cares about
    // the WARN/ERROR text, not the timestamp or the fully-qualified target.
    // One-shot CLI runs are transient — stderr is the only sink; the daemon
    // has its own file appender under `logs/daemon.log`.
    //
    // `.with_filter(env_filter)` applies the user-visible log filter to the
    // fmt layer ONLY. A registry-level filter would also suppress span
    // CREATION, which would starve the OTel exporter layer attached below
    // (`librefang_kernel`/`librefang_runtime` downgraded to WARN means all
    // INFO-level `#[instrument]` spans are filtered out before OTel ever
    // sees them). Per-layer filtering keeps stderr terse while OTel
    // receives the full span tree.
    //
    // The filter is wrapped in `ReloadableEnvFilter` so the daemon can swap
    // it at runtime when `KernelConfig::log_level` changes via hot-reload.
    // `install_with_baseline` hands the per-target directives above to the
    // filter installer so a dashboard `log_level` edit reapplies them after
    // the swap — i.e. the kernel/runtime overrides survive reloads instead
    // of being silently dropped. `RUST_LOG` itself is *not* re-read on
    // reload (it's a boot-time knob); operators wanting env-driven
    // filtering after a config edit need to restart.
    //
    // Force stderr explicitly: machine-readable subcommands like
    // `doctor --json` expect a clean stdout stream. The fmt layer's
    // default writer is stdout, which would interleave tracing output
    // with the JSON payload and corrupt downstream parsers.
    //
    // Build the inner format separately so we can wrap it in `WithTraceId`,
    // which appends the OTel `trace_id` as a logfmt suffix on every line when
    // an OTel context is active. The wrapper is unconditional but no-ops
    // without the `telemetry` feature; see `WithTraceId` doc above.
    let inner_format = tracing_subscriber::fmt::format()
        .without_time()
        .with_target(false)
        .compact();
    let reloadable_filter =
        log_filter::ReloadableEnvFilter::install_with_baseline(env_filter, baseline_directives);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .event_format(WithTraceId(inner_format))
        .with_filter(reloadable_filter);

    // Register a no-op reload slot so `init_otel_tracing` can swap a real
    // OTel layer in later without needing to claim the global dispatcher.
    // The slot is stacked **first** (directly on Registry) so its boxed
    // `Layer<Registry>` trait object matches the innermost subscriber type.
    // No filter is attached to this layer on purpose — see comment above.
    #[cfg(feature = "telemetry")]
    let registry =
        tracing_subscriber::registry().with(librefang_api::telemetry::install_otel_reload_layer());
    #[cfg(not(feature = "telemetry"))]
    let registry = tracing_subscriber::registry();

    registry.with(fmt_layer).init();
}

/// Redirect tracing to a log file so it doesn't corrupt the ratatui TUI.
fn init_tracing_file(log_level: &str, custom_log_dir: Option<&std::path::Path>) {
    // `custom_log_dir` is already a log directory (typically `daemon.log_dir`
    // from config); use it as-is. Otherwise default to `<home>/logs/`.
    let log_dir = custom_log_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| cli_librefang_home().join("logs"));
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("tui.log");

    match std::fs::File::create(&log_path) {
        Ok(file) => {
            // Same `WithTraceId` wrapper as `init_tracing_stderr` so the TUI
            // log file carries `trace_id=<hex>` suffixes when OTel is on.
            // We have to build the subscriber by hand here (rather than the
            // `tracing_subscriber::fmt()` builder shortcut) because the
            // builder owns its formatter and doesn't expose `event_format`.
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;

            let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));
            let inner_format = tracing_subscriber::fmt::format();
            let fmt_layer = tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .event_format(WithTraceId(inner_format));
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
        Err(_) => {
            // Fallback: suppress all output rather than corrupt the TUI
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::ERROR)
                .with_writer(std::io::sink)
                .init();
        }
    }
}

fn load_language_from_config() -> Option<String> {
    let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: toml::Value = toml::from_str(&content).ok()?;
    config.get("language")?.as_str().map(|s| s.to_string())
}

/// Load just the `log_level` field from config.toml without fully deserializing.
/// Returns the configured level (e.g. "debug", "warn") or falls back to "info".
fn load_log_level_from_config() -> String {
    let level = (|| -> Option<String> {
        let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
        let content = std::fs::read_to_string(&config_path).ok()?;
        let config: toml::Value = toml::from_str(&content).ok()?;
        config.get("log_level")?.as_str().map(|s| s.to_string())
    })();
    level.unwrap_or_else(|| "info".to_string())
}

/// Load just the `log_dir` field from config.toml without fully deserializing.
/// Returns the configured custom log directory, or `None` to use the default.
fn load_log_dir_from_config() -> Option<PathBuf> {
    let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: toml::Value = toml::from_str(&content).ok()?;
    config.get("log_dir")?.as_str().map(PathBuf::from)
}

fn main() {
    // Initialize rustls crypto provider FIRST, before any async/TLS operations
    // This is required because rustls 0.23 needs explicit crypto provider initialization
    {
        use rustls::crypto::aws_lc_rs;
        let _ = aws_lc_rs::default_provider().install_default();
    }

    // Load ~/.librefang/.env into process environment (system env takes priority).
    dotenv::load_dotenv();

    let language = load_language_from_config().unwrap_or_else(|| "en".to_string());
    i18n::init(&language);

    let cli = Cli::parse();

    // Determine if this invocation launches a ratatui TUI.
    // TUI modes must NOT install the Ctrl+C handler (it calls process::exit
    // which bypasses ratatui::restore and leaves the terminal in raw mode).
    // TUI modes also need file-based tracing (stderr output corrupts the TUI).
    let is_launcher = cli.command.is_none() && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let is_tui_mode = is_launcher
        || matches!(cli.command, Some(Commands::Tui))
        || matches!(cli.command, Some(Commands::Chat { .. }))
        || matches!(
            cli.command,
            Some(Commands::Agent(AgentCommands::Chat { .. }))
        );

    let log_level = load_log_level_from_config();
    let custom_log_dir = load_log_dir_from_config();

    if is_tui_mode {
        init_tracing_file(&log_level, custom_log_dir.as_deref());
    } else {
        // CLI subcommands: install Ctrl+C handler for clean interrupt of
        // blocking read_line calls, and trace to stderr.
        install_ctrlc_handler();
        init_tracing_stderr(&log_level);
    }

    match cli.command {
        None => {
            if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                // Piped: fall back to text help
                use clap::CommandFactory;
                Cli::command().print_help().unwrap();
                println!();
                return;
            }
            match launcher::run(cli.config.clone()) {
                launcher::LauncherChoice::GetStarted => cmd_init(false),
                launcher::LauncherChoice::Chat => cmd_quick_chat(cli.config, None),
                launcher::LauncherChoice::Dashboard => cmd_dashboard(),
                launcher::LauncherChoice::DesktopApp => launcher::launch_desktop_app(),
                launcher::LauncherChoice::TerminalUI => tui::run(cli.config),
                launcher::LauncherChoice::ShowHelp => {
                    use clap::CommandFactory;
                    Cli::command().print_help().unwrap();
                    println!();
                }
                launcher::LauncherChoice::Quit => {}
            }
        }
        Some(Commands::Tui) => tui::run(cli.config),
        Some(Commands::Init { quick, upgrade }) => {
            if upgrade {
                cmd_init_upgrade();
            } else {
                cmd_init(quick);
            }
        }
        Some(Commands::Start {
            tail,
            foreground,
            spawned,
        }) => cmd_start(cli.config, tail, spawned, foreground),
        Some(Commands::Restart { tail, foreground }) => cmd_restart(cli.config, tail, foreground),
        Some(Commands::Spawn(args)) => cmd_spawn_alias(
            cli.config,
            args.target,
            args.template,
            args.name,
            args.dry_run,
        ),
        Some(Commands::Agents { json }) => cmd_agent_list(cli.config, json),
        Some(Commands::Kill { agent_id }) => cmd_agent_kill(cli.config, &agent_id),
        Some(Commands::Update {
            check,
            version,
            channel,
        }) => cmd_update(check, version, channel),
        Some(Commands::Stop) => cmd_stop(cli.config),
        Some(Commands::Agent(sub)) => match sub {
            AgentCommands::New { template } => cmd_agent_new(cli.config, template),
            AgentCommands::Spawn(args) => {
                cmd_agent_spawn(cli.config, args.manifest, args.name, args.dry_run)
            }
            AgentCommands::List { json } => cmd_agent_list(cli.config, json),
            AgentCommands::Chat { agent_id } => cmd_agent_chat(cli.config, &agent_id),
            AgentCommands::Kill { agent_id } => cmd_agent_kill(cli.config, &agent_id),
            AgentCommands::Delete { name, yes } => cmd_agent_delete(cli.config, &name, yes),
            AgentCommands::ResetUuid { name, yes } => cmd_agent_reset_uuid(cli.config, &name, yes),
            AgentCommands::MergeHistory { name, from } => cmd_agent_merge_history(&name, &from),
            AgentCommands::Set {
                agent_id,
                field,
                value,
            } => cmd_agent_set(&agent_id, &field, &value),
        },
        Some(Commands::Workflow(sub)) => match sub {
            WorkflowCommands::List => cmd_workflow_list(),
            WorkflowCommands::Create { file } => cmd_workflow_create(file),
            WorkflowCommands::Run { workflow_id, input } => cmd_workflow_run(&workflow_id, &input),
        },
        Some(Commands::Trigger(sub)) => match sub {
            TriggerCommands::List { agent_id } => cmd_trigger_list(agent_id.as_deref()),
            TriggerCommands::Get { trigger_id } => cmd_trigger_get(&trigger_id),
            TriggerCommands::Create {
                agent_id,
                pattern_json,
                prompt,
                max_fires,
                target_agent,
                cooldown,
                session_mode,
            } => cmd_trigger_create(
                &agent_id,
                &pattern_json,
                &prompt,
                max_fires,
                target_agent.as_deref(),
                cooldown,
                session_mode.as_deref(),
            ),
            TriggerCommands::Update {
                trigger_id,
                pattern,
                prompt,
                enabled,
                max_fires,
                cooldown,
                clear_cooldown,
                session_mode,
                clear_session_mode,
                target_agent,
                clear_target_agent,
            } => cmd_trigger_update(
                &trigger_id,
                pattern.as_deref(),
                prompt.as_deref(),
                enabled,
                max_fires,
                cooldown,
                clear_cooldown,
                session_mode.as_deref(),
                clear_session_mode,
                target_agent.as_deref(),
                clear_target_agent,
            ),
            TriggerCommands::Enable { trigger_id } => cmd_trigger_set_enabled(&trigger_id, true),
            TriggerCommands::Disable { trigger_id } => cmd_trigger_set_enabled(&trigger_id, false),
            TriggerCommands::Delete { trigger_id } => cmd_trigger_delete(&trigger_id),
        },
        Some(Commands::Migrate(args)) => cmd_migrate(args),
        Some(Commands::Skill(sub)) => match sub {
            SkillCommands::Install { source, hand } => cmd_skill_install(&source, hand.as_deref()),
            SkillCommands::List { hand } => cmd_skill_list(hand.as_deref()),
            SkillCommands::Remove { name, hand } => cmd_skill_remove(&name, hand.as_deref()),
            SkillCommands::Search { query } => cmd_skill_search(&query),
            SkillCommands::Test { path, tool, input } => cmd_skill_test(path, tool, input),
            SkillCommands::Publish {
                path,
                repo,
                tag,
                output,
                dry_run,
            } => cmd_skill_publish(path, repo, tag, output, dry_run),
            SkillCommands::Create => cmd_skill_create(),
            SkillCommands::Evolve(sub) => cmd_skill_evolve(sub),
            SkillCommands::Pending(sub) => cmd_skill_pending(sub),
        },
        Some(Commands::Channel(sub)) => match sub {
            ChannelCommands::List => cmd_channel_list(),
            ChannelCommands::Reload => cmd_channel_reload(),
            ChannelCommands::Setup { name } => cmd_channel_setup(name.as_deref()),
            ChannelCommands::Rm { name } => cmd_channel_rm(&name),
        },
        Some(Commands::Hand(sub)) => match sub {
            HandCommands::List => cmd_hand_list(),
            HandCommands::Active => cmd_hand_active(),
            HandCommands::Status { id } => cmd_hand_status(id.as_deref()),
            HandCommands::Install { path } => cmd_hand_install(&path),
            HandCommands::Activate { id } => cmd_hand_activate(&id),
            HandCommands::Deactivate { id } => cmd_hand_deactivate(&id),
            HandCommands::Info { id } => cmd_hand_info(&id),
            HandCommands::CheckDeps { id } => cmd_hand_check_deps(&id),
            HandCommands::InstallDeps { id } => cmd_hand_install_deps(&id),
            HandCommands::Pause { id } => cmd_hand_pause(&id),
            HandCommands::Resume { id } => cmd_hand_resume(&id),
            HandCommands::Settings { id } => cmd_hand_settings(&id),
            HandCommands::Set { id, key, value } => cmd_hand_set(&id, &key, &value),
            HandCommands::Reload => cmd_hand_reload(),
            HandCommands::Chat { id } => cmd_hand_chat(&id),
        },
        Some(Commands::Config(sub)) => match sub {
            ConfigCommands::Show => cmd_config_show(),
            ConfigCommands::Edit => cmd_config_edit(),
            ConfigCommands::Get { key } => cmd_config_get(&key),
            ConfigCommands::Set { key, value } => cmd_config_set(&key, &value),
            ConfigCommands::Unset { key } => cmd_config_unset(&key),
            ConfigCommands::SetKey { provider } => cmd_config_set_key(&provider),
            ConfigCommands::DeleteKey { provider } => cmd_config_delete_key(&provider),
            ConfigCommands::TestKey { provider } => cmd_config_test_key(&provider),
        },
        Some(Commands::Chat { agent }) => cmd_quick_chat(cli.config, agent),
        Some(Commands::Status {
            json,
            verbose,
            quiet,
            watch,
        }) => cmd_status(cli.config, json, verbose, quiet, watch),
        Some(Commands::Doctor { json, repair }) => cmd_doctor(json, repair),
        Some(Commands::Dashboard) => cmd_dashboard(),
        Some(Commands::Completion { shell }) => cmd_completion(shell),
        Some(Commands::Mcp { command }) => match command {
            None => mcp::run_mcp_server(cli.config),
            Some(McpCommands::List) => cmd_mcp_list(),
            Some(McpCommands::Catalog { query }) => cmd_mcp_catalog(query.as_deref()),
            Some(McpCommands::Add { name, key }) => cmd_mcp_add(&name, key.as_deref()),
            Some(McpCommands::Remove { name }) => cmd_mcp_remove(&name),
        },
        Some(Commands::Acp { agent }) => acp::run_acp_server(cli.config, agent),
        Some(Commands::Auth(sub)) => match sub {
            AuthCommands::Chatgpt { device_auth } => cmd_auth_chatgpt(device_auth),
            AuthCommands::Pool(sub) => match sub {
                AuthPoolCommands::List { json } => cmd_auth_pool_list(cli.config, json),
                AuthPoolCommands::Add {
                    provider,
                    env_var,
                    label,
                    priority,
                } => cmd_auth_pool_add(cli.config, &provider, &env_var, &label, priority),
                AuthPoolCommands::Remove { provider, env_var } => {
                    cmd_auth_pool_remove(cli.config, &provider, &env_var)
                }
                AuthPoolCommands::Strategy { provider, strategy } => {
                    cmd_auth_pool_strategy(cli.config, &provider, &strategy)
                }
            },
        },
        Some(Commands::Vault(sub)) => match sub {
            VaultCommands::Init => cmd_vault_init(),
            VaultCommands::Set { key } => cmd_vault_set(&key),
            VaultCommands::List => cmd_vault_list(),
            VaultCommands::Remove { key } => cmd_vault_remove(&key),
            VaultCommands::RotateKey { from_stdin } => cmd_vault_rotate_key(from_stdin),
        },
        Some(Commands::New { kind }) => cmd_scaffold(kind),
        // ── New commands ────────────────────────────────────────────────
        Some(Commands::Models(sub)) => match sub {
            ModelsCommands::List { provider, json } => cmd_models_list(provider.as_deref(), json),
            ModelsCommands::Aliases { json } => cmd_models_aliases(json),
            ModelsCommands::Providers { json } => cmd_models_providers(json),
            ModelsCommands::Set { model } => cmd_models_set(model),
        },
        Some(Commands::Gateway(sub)) => match sub {
            GatewayCommands::Start { tail, foreground } => {
                cmd_start(cli.config, tail, false, foreground)
            }
            GatewayCommands::Restart { tail, foreground } => {
                cmd_restart(cli.config, tail, foreground)
            }
            GatewayCommands::Stop => cmd_stop(cli.config),
            GatewayCommands::Status { json } => cmd_status(cli.config, json, false, false, None),
        },
        Some(Commands::Approvals(sub)) => match sub {
            ApprovalsCommands::List { json } => cmd_approvals_list(json),
            ApprovalsCommands::Approve { id } => cmd_approvals_respond(&id, true),
            ApprovalsCommands::Reject { id } => cmd_approvals_respond(&id, false),
        },
        Some(Commands::Cron(sub)) => match sub {
            CronCommands::List { json } => cmd_cron_list(json),
            CronCommands::Create {
                agent,
                spec,
                prompt,
                name,
            } => cmd_cron_create(&agent, &spec, &prompt, name.as_deref()),
            CronCommands::Delete { id } => cmd_cron_delete(&id),
            CronCommands::Enable { id } => cmd_cron_toggle(&id, true),
            CronCommands::Disable { id } => cmd_cron_toggle(&id, false),
        },
        Some(Commands::Sessions {
            agent,
            json,
            active,
        }) => cmd_sessions(agent.as_deref(), json, active),
        Some(Commands::Logs { lines, follow }) => cmd_logs(cli.config, lines, follow),
        Some(Commands::Health { json }) => cmd_health(json),
        Some(Commands::Security(sub)) => match sub {
            SecurityCommands::Status { json } => cmd_security_status(json),
            SecurityCommands::Audit { limit, json } => cmd_security_audit(limit, json),
            SecurityCommands::Verify => cmd_security_verify(),
            SecurityCommands::AuditReset { confirm } => cmd_audit_reset(cli.config, confirm),
        },
        Some(Commands::Memory(sub)) => match sub {
            MemoryCommands::List { agent, json } => cmd_memory_list(&agent, json),
            MemoryCommands::Get { agent, key, json } => cmd_memory_get(&agent, &key, json),
            MemoryCommands::Set { agent, key, value } => cmd_memory_set(&agent, &key, &value),
            MemoryCommands::Delete { agent, key } => cmd_memory_delete(&agent, &key),
        },
        Some(Commands::Devices(sub)) => match sub {
            DevicesCommands::List { json } => cmd_devices_list(json),
            DevicesCommands::Pair => cmd_devices_pair(),
            DevicesCommands::Remove { id } => cmd_devices_remove(&id),
        },
        Some(Commands::Qr) => cmd_devices_pair(),
        Some(Commands::Webhooks(sub)) => match sub {
            WebhooksCommands::List { json } => cmd_webhooks_list(json),
            WebhooksCommands::Create { agent, url } => cmd_webhooks_create(&agent, &url),
            WebhooksCommands::Delete { id } => cmd_webhooks_delete(&id),
            WebhooksCommands::Test { id } => cmd_webhooks_test(&id),
        },
        Some(Commands::Onboard { quick, upgrade }) | Some(Commands::Setup { quick, upgrade }) => {
            if upgrade {
                cmd_init_upgrade();
            } else {
                cmd_init(quick);
            }
        }
        Some(Commands::Configure) => cmd_init(false),
        Some(Commands::Message {
            agent,
            text,
            json,
            incognito,
        }) => cmd_message(&agent, &text, json, incognito),
        Some(Commands::System(sub)) => match sub {
            SystemCommands::Info { json } => cmd_system_info(json),
            SystemCommands::Version { json } => cmd_system_version(json),
        },
        Some(Commands::Service(sub)) => match sub {
            ServiceCommands::Install => cmd_service_install(),
            ServiceCommands::Uninstall => cmd_service_uninstall(),
            ServiceCommands::Status => cmd_service_status(),
        },
        Some(Commands::Reset { confirm }) => cmd_reset(confirm),
        Some(Commands::Uninstall {
            confirm,
            keep_config,
        }) => cmd_uninstall(confirm, keep_config),
        Some(Commands::HashPassword { password }) => cmd_hash_password(password),
    }
}

struct PreparedAgentManifest {
    manifest: AgentManifest,
    manifest_toml: String,
    source_label: String,
}

fn cmd_agent_spawn(
    config: Option<PathBuf>,
    manifest_path: PathBuf,
    name_override: Option<String>,
    dry_run: bool,
) {
    let prepared = prepared_agent_manifest_from_path(&manifest_path, name_override.as_deref());
    if dry_run {
        preview_agent_manifest(&prepared);
        return;
    }
    spawn_prepared_agent(config, prepared);
}

fn cmd_spawn_alias(
    config: Option<PathBuf>,
    target: Option<String>,
    template_path: Option<PathBuf>,
    name_override: Option<String>,
    dry_run: bool,
) {
    if template_path.is_some() && target.is_some() {
        ui::error_with_fix(
            "Choose either a positional target or `--template`, not both.",
            "Use `librefang spawn coder` or `librefang spawn --template agents/custom/my-agent.toml`.",
        );
        std::process::exit(1);
    }

    if target.is_none() && template_path.is_none() {
        if name_override.is_some() {
            ui::error_with_fix(
                "`--name` requires a template name or manifest path.",
                "Use `librefang spawn coder --name backend-coder` or `librefang spawn --template path/to/agent.toml --name backend-coder`.",
            );
            std::process::exit(1);
        }
        if dry_run {
            ui::error_with_fix(
                "Dry run needs a template name or manifest path.",
                "Use `librefang spawn coder --dry-run` or `librefang spawn --template path/to/agent.toml --dry-run`.",
            );
            std::process::exit(1);
        }
        cmd_agent_new(config, None);
        return;
    }

    if let Some(path) = template_path {
        let prepared = prepared_agent_manifest_from_path(&path, name_override.as_deref());
        if dry_run {
            preview_agent_manifest(&prepared);
        } else {
            spawn_prepared_agent(config, prepared);
        }
        return;
    }

    let target = target.expect("target checked above");
    let manifest_path = PathBuf::from(&target);
    if manifest_path.exists() {
        let prepared = prepared_agent_manifest_from_path(&manifest_path, name_override.as_deref());
        if dry_run {
            preview_agent_manifest(&prepared);
        } else {
            spawn_prepared_agent(config, prepared);
        }
        return;
    }

    let templates = templates::load_all_templates();
    let template = templates
        .iter()
        .find(|t| t.name == target)
        .unwrap_or_else(|| {
            ui::error_with_fix(
                &format!("Template or manifest path not found: {target}"),
                "Run `librefang agent new` to browse templates, or pass a valid manifest path.",
            );
            std::process::exit(1);
        });
    if dry_run {
        let prepared = prepared_agent_manifest_from_template(template, name_override.as_deref());
        preview_agent_manifest(&prepared);
    } else {
        spawn_template_agent(config, template, name_override.as_deref());
    }
}

fn prepared_agent_manifest_from_path(
    manifest_path: &std::path::Path,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    if !manifest_path.exists() {
        ui::error_with_fix(
            &i18n::t_args(
                "manifest-not-found",
                &[("path", &manifest_path.display().to_string())],
            ),
            &i18n::t("manifest-not-found-fix"),
        );
        std::process::exit(1);
    }

    let contents = std::fs::read_to_string(manifest_path).unwrap_or_else(|e| {
        eprintln!(
            "{}",
            i18n::t_args("error-reading-manifest", &[("error", &e.to_string())])
        );
        std::process::exit(1);
    });

    prepared_agent_manifest_from_contents(
        &contents,
        manifest_path.display().to_string(),
        name_override,
    )
}

fn prepared_agent_manifest_from_template(
    template: &templates::AgentTemplate,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    prepared_agent_manifest_from_contents(
        &template.content,
        format!("template:{}", template.name),
        name_override,
    )
}

fn prepared_agent_manifest_from_contents(
    contents: &str,
    source_label: String,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    let mut manifest: AgentManifest = toml::from_str(contents).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Failed to parse agent manifest from {source_label}: {e}"),
            "Check the manifest TOML syntax and required fields.",
        );
        std::process::exit(1);
    });

    if let Some(name) = name_override {
        manifest.name = name.to_string();
    }

    let manifest_toml = if name_override.is_some() {
        toml::to_string_pretty(&manifest).unwrap_or_else(|e| {
            ui::error(&format!("Failed to serialize updated manifest: {e}"));
            std::process::exit(1);
        })
    } else {
        contents.to_string()
    };

    PreparedAgentManifest {
        manifest,
        manifest_toml,
        source_label,
    }
}

fn preview_agent_manifest(prepared: &PreparedAgentManifest) {
    ui::section("Agent Dry Run");
    ui::kv("Source", &prepared.source_label);
    ui::kv("Name", &prepared.manifest.name);
    ui::kv("Version", &prepared.manifest.version);
    ui::kv("Module", &prepared.manifest.module);
    ui::kv(
        "Model",
        &format!(
            "{}/{}",
            prepared.manifest.model.provider, prepared.manifest.model.model
        ),
    );
    ui::kv(
        "Tools",
        &prepared.manifest.capabilities.tools.len().to_string(),
    );
    ui::kv("Skills", &prepared.manifest.skills.len().to_string());
    if !prepared.manifest.tags.is_empty() {
        ui::kv("Tags", &prepared.manifest.tags.join(", "));
    }
    if !prepared.manifest.description.is_empty() {
        ui::kv("Description", &prepared.manifest.description);
    }
    ui::success("Manifest parsed successfully. No agent was spawned.");
}

fn spawn_prepared_agent(config: Option<PathBuf>, prepared: PreparedAgentManifest) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": prepared.manifest_toml}))
                .send(),
        );
        if body.get("agent_id").is_some() {
            println!("{}", i18n::t("agent-spawn-success"));
            println!("  ID:   {}", body["agent_id"].as_str().unwrap_or("?"));
            println!(
                "  Name: {}",
                body["name"]
                    .as_str()
                    .unwrap_or(prepared.manifest.name.as_str())
            );
        } else {
            eprintln!(
                "{}",
                i18n::t_args(
                    "agent-spawn-agent-failed",
                    &[("error", body["error"].as_str().unwrap_or("Unknown error"))]
                )
            );
            std::process::exit(1);
        }
    } else {
        let agent_name = prepared.manifest.name.clone();
        let kernel = boot_kernel(config);
        match kernel.spawn_agent_with_source(prepared.manifest, None) {
            Ok(id) => {
                println!("{}", i18n::t("agent-spawn-inprocess-mode"));
                println!("  ID:   {id}");
                println!("  Name: {agent_name}");
                println!("\n  {}", i18n::t("agent-note-lost"));
                println!("  {}", i18n::t("agent-note-persistent"));
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-spawn-agent-failed", &[("error", &e.to_string())])
                );
                std::process::exit(1);
            }
        }
    }
}

fn cmd_agent_list(config: Option<PathBuf>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/agents")).send());

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }

        let agents = body
            .get("items")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array());

        match agents {
            Some(agents) if agents.is_empty() => println!("{}", i18n::t("agent-no-agents")),
            Some(agents) => {
                // Render via the shared Table builder so column widths
                // self-size to the actual content (instead of hard-coded
                // {:<38} which truncates / over-pads), and so piped output
                // automatically falls back to ASCII (#3306).
                let mut t = crate::table::Table::new(&["ID", "NAME", "STATE", "PROVIDER", "MODEL"]);
                for a in agents {
                    t.add_row(&[
                        a["id"].as_str().unwrap_or("?"),
                        a["name"].as_str().unwrap_or("?"),
                        a["state"].as_str().unwrap_or("?"),
                        a["model_provider"].as_str().unwrap_or("?"),
                        a["model_name"].as_str().unwrap_or("?"),
                    ]);
                }
                t.print();
            }
            None => println!("{}", i18n::t("agent-no-agents")),
        }
    } else {
        let kernel = boot_kernel(config);
        let agents = kernel.agent_registry_ref().list();

        if json {
            let list: Vec<serde_json::Value> = agents
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "name": e.name,
                        "state": format!("{:?}", e.state),
                        "created_at": e.created_at.to_rfc3339(),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&list).unwrap_or_default()
            );
            return;
        }

        if agents.is_empty() {
            println!("{}", i18n::t("agent-no-agents"));
            return;
        }

        let mut t = crate::table::Table::new(&["ID", "NAME", "STATE", "CREATED"]);
        for entry in agents {
            let id = entry.id.to_string();
            let state = format!("{:?}", entry.state);
            let created = entry.created_at.format("%Y-%m-%d %H:%M").to_string();
            t.add_row(&[
                id.as_str(),
                entry.name.as_str(),
                state.as_str(),
                created.as_str(),
            ]);
        }
        t.print();
    }
}

fn cmd_agent_chat(config: Option<PathBuf>, agent_id_str: &str) {
    ensure_initialized(&config);
    tui::chat_runner::run_chat_tui(config, Some(agent_id_str.to_string()));
}

fn cmd_agent_kill(config: Option<PathBuf>, agent_id_str: &str) {
    if let Some(base) = find_daemon() {
        let agent_id = resolve_agent_id(&base, agent_id_str);
        let client = daemon_client();
        // Refs #4614: explicit `librefang agent kill <id>` IS the user's
        // confirmation. The API requires `?confirm=true` on DELETE so the
        // canonical UUID is purged on the kill (matching the issue's
        // "explicit delete" semantics). Internal lifecycle resets call
        // `kernel.kill_agent` directly and skip this path.
        let body = daemon_json(
            client
                .delete(format!("{base}/api/agents/{agent_id}?confirm=true"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("{}", i18n::t_args("agent-killed", &[("id", &agent_id)]));
        } else {
            eprintln!(
                "{}",
                i18n::t_args(
                    "agent-kill-failed",
                    &[("error", body["error"].as_str().unwrap_or("Unknown error"))]
                )
            );
            std::process::exit(1);
        }
    } else {
        let agent_id: AgentId = agent_id_str.parse().unwrap_or_else(|_| {
            eprintln!(
                "{}",
                i18n::t_args("agent-invalid-id", &[("id", agent_id_str)])
            );
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        // Direct-kernel path (no daemon): mirror the API's confirmed-delete
        // semantics so behavior matches whether the daemon is running or not.
        match kernel.kill_agent_with_purge(agent_id, true) {
            Ok(()) => println!(
                "{}",
                i18n::t_args("agent-killed", &[("id", &agent_id.to_string())])
            ),
            Err(e) => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-kill-failed", &[("error", &e.to_string())])
                );
                std::process::exit(1);
            }
        }
    }
}

/// Refs #4614 — `librefang agent delete <name>` with confirmation prompt.
///
/// Looks up the canonical UUID for `name` via `GET /api/agents/identities`
/// (or directly from the kernel registry when no daemon is running),
/// prints the destructive-action warning, and either prompts `[y/N]` or
/// proceeds immediately when `--yes` is set. Then issues the confirmed
/// DELETE. This is the long-form companion to `librefang agent kill <id>`
/// — useful when the operator only knows the agent's name.
fn cmd_agent_delete(config: Option<PathBuf>, name: &str, yes: bool) {
    eprintln!("WARNING: Deleting agent \"{name}\" will permanently remove its canonical UUID");
    eprintln!("    and all associated memories and sessions.");
    eprintln!("    This action cannot be undone.");
    if !yes && !prompt_yes_no("Confirm?", false) {
        eprintln!("Aborted.");
        std::process::exit(1);
    }

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        // Resolve name → UUID via the identity registry endpoint.
        let canonical_uuid = match lookup_canonical_uuid(&base, name) {
            Some(id) => id,
            None => {
                eprintln!(
                    "No canonical UUID recorded for agent name '{name}' — nothing to delete."
                );
                std::process::exit(1);
            }
        };
        let body = daemon_json(
            client
                .delete(format!("{base}/api/agents/{canonical_uuid}?confirm=true"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("Agent \"{name}\" deleted (canonical UUID purged).");
        } else {
            eprintln!(
                "Failed to delete agent: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
            std::process::exit(1);
        }
    } else {
        let kernel = boot_kernel(config);
        let canonical_uuid = match kernel.identities_ref().get(name) {
            Some(id) => id,
            None => {
                eprintln!(
                    "No canonical UUID recorded for agent name '{name}' — nothing to delete."
                );
                std::process::exit(1);
            }
        };
        match kernel.kill_agent_with_purge(canonical_uuid, true) {
            Ok(()) => println!("Agent \"{name}\" deleted (canonical UUID purged)."),
            Err(e) => {
                eprintln!("Failed to delete agent: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// Refs #4614 — `librefang agent reset-uuid <name>` with confirmation.
///
/// Drops the canonical UUID binding without killing a running agent. The
/// next spawn under `name` re-derives a fresh UUID and registers it as
/// the new canonical binding; prior sessions / memories tied to the old
/// UUID are orphaned. `--yes` skips the prompt.
fn cmd_agent_reset_uuid(config: Option<PathBuf>, name: &str, yes: bool) {
    eprintln!("WARNING: Resetting the canonical UUID for \"{name}\" will orphan all sessions");
    eprintln!("    and memories tied to its current UUID. The next spawn under this");
    eprintln!("    name will start with a fresh UUID. This action cannot be undone.");
    if !yes && !prompt_yes_no("Confirm?", false) {
        eprintln!("Aborted.");
        std::process::exit(1);
    }

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!(
                    "{base}/api/agents/identities/{}/reset",
                    percent_encode_path_segment(name)
                ))
                .query(&[("confirm", "true")])
                .send(),
        );
        if body.get("status").is_some() {
            println!(
                "Canonical UUID for \"{name}\" reset (was {}).",
                body["previous_canonical_uuid"]
                    .as_str()
                    .unwrap_or("<unknown>")
            );
        } else {
            eprintln!(
                "Failed to reset canonical UUID: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
            std::process::exit(1);
        }
    } else {
        let kernel = boot_kernel(config);
        match kernel.identities_ref().purge(name) {
            Some(prev) => println!("Canonical UUID for \"{name}\" reset (was {prev})."),
            None => {
                eprintln!("No canonical UUID recorded for agent name '{name}'.");
                std::process::exit(1);
            }
        }
    }
}

/// Refs #4614 — `librefang agent merge-history` placeholder.
///
/// The cross-table reassignment is not yet implemented — see the
/// long_about on `AgentCommands::MergeHistory` for the rationale (deep
/// memory-substrate surgery across 10+ tables under one transaction).
fn cmd_agent_merge_history(name: &str, from: &str) {
    eprintln!("merge-history is not yet implemented (refs #4614 follow-up).");
    eprintln!("Reassignment of sessions / memories from {from} to the canonical UUID");
    eprintln!("for agent \"{name}\" requires cross-table SQL surgery in the memory");
    eprintln!("substrate that is being tracked separately.");
    std::process::exit(2);
}

/// Look up the canonical UUID for `name` via the identity-registry
/// endpoint. Returns `None` if no entry exists (or on any HTTP error —
/// the caller surfaces a friendly message).
fn lookup_canonical_uuid(base: &str, name: &str) -> Option<String> {
    let client = daemon_client();
    let resp = client
        .get(format!("{base}/api/agents/identities"))
        .send()
        .ok()?;
    let entries: serde_json::Value = resp.json().ok()?;
    let arr = entries.as_array()?;
    for entry in arr {
        if entry["name"].as_str() == Some(name) {
            return entry["canonical_uuid"].as_str().map(String::from);
        }
    }
    None
}

fn cmd_agent_set(agent_id_str: &str, field: &str, value: &str) {
    match field {
        "model" => {
            if let Some(base) = find_daemon() {
                let agent_id = resolve_agent_id(&base, agent_id_str);
                let client = daemon_client();
                let body = daemon_json(
                    client
                        .put(format!("{base}/api/agents/{agent_id}/model"))
                        .json(&serde_json::json!({"model": value}))
                        .send(),
                );
                if body.get("status").is_some() {
                    println!("Agent {agent_id} model set to {value}.");
                } else {
                    eprintln!(
                        "Failed to set model: {}",
                        body["error"].as_str().unwrap_or("Unknown error")
                    );
                    std::process::exit(1);
                }
            } else {
                eprintln!("No running daemon found. Start one with: librefang start");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Unknown field: {field}. Supported fields: model");
            std::process::exit(1);
        }
    }
}

fn cmd_agent_new(config: Option<PathBuf>, template_name: Option<String>) {
    let all_templates = templates::load_all_templates();
    if all_templates.is_empty() {
        ui::error_with_fix(
            "No agent templates found",
            "Run `librefang init` to set up the agents directory",
        );
        std::process::exit(1);
    }

    // Resolve template: by name or interactive picker
    let chosen = match template_name {
        Some(ref name) => match all_templates.iter().find(|t| t.name == *name) {
            Some(t) => t,
            None => {
                ui::error_with_fix(
                    &format!("Template '{name}' not found"),
                    "Run `librefang agent new` to see available templates",
                );
                std::process::exit(1);
            }
        },
        None => {
            ui::section(&i18n::t("section-agent-templates"));
            ui::blank();
            for (i, t) in all_templates.iter().enumerate() {
                let desc = if t.description.is_empty() {
                    String::new()
                } else {
                    format!("  {}", t.description)
                };
                println!(
                    "    {:>2}. {:<22}{}",
                    i + 1,
                    t.name,
                    colored::Colorize::dimmed(desc.as_str())
                );
            }
            ui::blank();
            let choice = prompt_input("  Choose template [1]: ");
            let idx = if choice.is_empty() {
                0
            } else {
                choice
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(all_templates.len() - 1)
            };
            &all_templates[idx]
        }
    };

    // Spawn the agent
    spawn_template_agent(config, chosen, None);
}

/// Spawn an agent from a template, via daemon or in-process.
fn spawn_template_agent(
    config: Option<PathBuf>,
    template: &templates::AgentTemplate,
    name_override: Option<&str>,
) {
    let prepared = prepared_agent_manifest_from_template(template, name_override);
    let agent_name = prepared.manifest.name.clone();

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": prepared.manifest_toml}))
                .send(),
        );
        if let Some(id) = body["agent_id"].as_str() {
            ui::blank();
            ui::success(&i18n::t_args("agent-spawned", &[("name", &agent_name)]));
            ui::kv(&i18n::t("label-id"), id);
            if let Some(model) = body["model_name"].as_str() {
                let provider = body["model_provider"].as_str().unwrap_or("?");
                ui::kv(&i18n::t("label-model"), &format!("{provider}/{model}"));
            }
            ui::blank();
            ui::hint(&i18n::t_args(
                "hint-chat-with-agent",
                &[("name", &agent_name)],
            ));
        } else {
            ui::error(&i18n::t_args(
                "agent-spawn-failed",
                &[("error", body["error"].as_str().unwrap_or("Unknown error"))],
            ));
            std::process::exit(1);
        }
    } else {
        let kernel = boot_kernel(config);
        match kernel.spawn_agent(prepared.manifest) {
            Ok(id) => {
                ui::blank();
                ui::success(&i18n::t_args(
                    "agent-spawned-inprocess",
                    &[("name", &agent_name)],
                ));
                ui::kv(&i18n::t("label-id"), &id.to_string());
                ui::blank();
                ui::hint(&i18n::t_args(
                    "hint-chat-with-agent",
                    &[("name", &agent_name)],
                ));
                ui::hint(&i18n::t("hint-agent-lost-on-exit"));
                ui::hint(&i18n::t("hint-persistent-agents"));
            }
            Err(e) => {
                ui::error(&i18n::t_args(
                    "agent-spawn-agent-failed",
                    &[("error", &e.to_string())],
                ));
                std::process::exit(1);
            }
        }
    }
}

fn cmd_doctor(json: bool, repair: bool) {
    // BrokenPipe protection for the WHOLE command, not just the --json
    // branch. `librefang doctor | head -5` and similar pipelines drop the
    // reader after a few lines, which on the next stdout write turns into a
    // panic — Rust ignores SIGPIPE by default and translates EPIPE into an
    // io::Error that `println!` unwraps.
    //
    // The pre-existing `write_stdout_safe` helper only covered the
    // `--json` final emission. Hundreds of `ui::*` and bare `println!`
    // calls between the start of cmd_doctor and that emission were still
    // unprotected. Restoring the default SIGPIPE handler for the duration
    // of this command makes the kernel terminate the process cleanly on
    // pipe close instead, covering every print path in this function and
    // the `ui::*` helpers it calls.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let mut checks: Vec<serde_json::Value> = Vec::new();
    let mut all_ok = true;
    let mut repaired = false;

    if !json {
        ui::step(&i18n::t("doctor-title"));
        println!();
    }

    let home = dirs::home_dir();
    if let Some(_h) = &home {
        let librefang_dir = cli_librefang_home();

        // --- Check 1: LibreFang directory ---
        if librefang_dir.exists() {
            if !json {
                ui::check_ok(&format!("LibreFang directory: {}", librefang_dir.display()));
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": "ok", "path": librefang_dir.display().to_string()}));
        } else if repair {
            if !json {
                ui::check_fail("LibreFang directory not found.");
            }
            let answer = prompt_input("    Create it now? [Y/n] ");
            if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
                if std::fs::create_dir_all(&librefang_dir).is_ok() {
                    restrict_dir_permissions(&librefang_dir);
                    let _ = std::fs::create_dir_all(librefang_dir.join("data"));
                    let _ =
                        std::fs::create_dir_all(librefang_dir.join("workspaces").join("agents"));
                    if !json {
                        ui::check_ok("Created LibreFang directory");
                    }
                    repaired = true;
                } else {
                    if !json {
                        ui::check_fail("Failed to create directory");
                    }
                    all_ok = false;
                }
            } else {
                all_ok = false;
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": if repaired { "repaired" } else { "fail" }}));
        } else {
            if !json {
                ui::check_fail("LibreFang directory not found. Run `librefang init` first.");
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": "fail"}));
            all_ok = false;
        }

        // --- Check 2: .env file exists + permissions ---
        let env_path = librefang_dir.join(".env");
        if env_path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&env_path) {
                    let mode = meta.permissions().mode() & 0o777;
                    if mode == 0o600 {
                        if !json {
                            ui::check_ok(".env file (permissions OK)");
                        }
                    } else if repair {
                        let _ = std::fs::set_permissions(
                            &env_path,
                            std::fs::Permissions::from_mode(0o600),
                        );
                        if !json {
                            ui::check_ok(".env file (permissions fixed to 0600)");
                        }
                        repaired = true;
                    } else if !json {
                        ui::check_warn(&format!(
                            ".env file has loose permissions ({:o}), should be 0600",
                            mode
                        ));
                    }
                } else if !json {
                    ui::check_ok(".env file");
                }
            }
            #[cfg(not(unix))]
            {
                if !json {
                    ui::check_ok(".env file");
                }
            }
            checks.push(serde_json::json!({"check": "env_file", "status": "ok"}));
        } else {
            if !json {
                ui::check_warn(
                    ".env file not found (create with: librefang config set-key <provider>)",
                );
            }
            checks.push(serde_json::json!({"check": "env_file", "status": "warn"}));
        }

        // --- Check 3: Config TOML syntax validation ---
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str::<toml::Value>(&config_content) {
                Ok(_) => {
                    if !json {
                        ui::check_ok(&format!("Config file: {}", config_path.display()));
                    }
                    checks.push(serde_json::json!({"check": "config_file", "status": "ok"}));
                }
                Err(e) => {
                    if !json {
                        ui::check_fail(&format!("Config file has syntax errors: {e}"));
                        ui::hint(&i18n::t("hint-config-edit"));
                    }
                    checks.push(serde_json::json!({"check": "config_syntax", "status": "fail", "error": e.to_string()}));
                    all_ok = false;
                }
            }
        } else if repair {
            if !json {
                ui::check_fail("Config file not found.");
            }
            let answer = prompt_input("    Create default config? [Y/n] ");
            if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
                let (provider, api_key_env, model) = detect_best_provider();
                let default_config = render_init_default_config(&provider, &model, &api_key_env);
                let _ = std::fs::create_dir_all(&librefang_dir);
                if std::fs::write(&config_path, default_config).is_ok() {
                    restrict_file_permissions(&config_path);
                    if !json {
                        ui::check_ok("Created default config.toml");
                    }
                    repaired = true;
                } else {
                    if !json {
                        ui::check_fail("Failed to create config.toml");
                    }
                    all_ok = false;
                }
            } else {
                all_ok = false;
            }
            checks.push(serde_json::json!({"check": "config_file", "status": if repaired { "repaired" } else { "fail" }}));
        } else {
            if !json {
                ui::check_fail("Config file not found.");
            }
            checks.push(serde_json::json!({"check": "config_file", "status": "fail"}));
            all_ok = false;
        }

        // --- Check: Version update ---
        {
            let current_version = env!("CARGO_PKG_VERSION");
            let update_channel = load_update_channel_from_config().unwrap_or_default();
            if !json {
                ui::check_ok(&format!(
                    "CLI version: {current_version} (channel: {update_channel})"
                ));
            }
            checks.push(serde_json::json!({"check": "cli_version", "status": "ok", "version": current_version, "channel": update_channel.to_string()}));

            // Try to fetch latest release for the configured channel (best-effort)
            match fetch_latest_release_tag(update_channel) {
                Ok(tag) => {
                    let latest = tag.strip_prefix('v').unwrap_or(&tag);
                    if latest != current_version {
                        if !json {
                            ui::check_warn(&format!(
                                "Update available: {current_version} -> {latest} (see https://github.com/librefang/librefang/releases)"
                            ));
                        }
                        checks.push(serde_json::json!({"check": "version_update", "status": "warn", "current": current_version, "latest": latest}));
                    } else {
                        if !json {
                            ui::check_ok("CLI is up to date");
                        }
                        checks.push(serde_json::json!({"check": "version_update", "status": "ok"}));
                    }
                }
                Err(_) => {
                    if !json {
                        ui::check_warn("Could not check for updates (network unavailable)");
                    }
                    checks.push(serde_json::json!({"check": "version_update", "status": "warn", "reason": "network_error"}));
                }
            }
        }

        // --- Check 4: Port availability ---
        // Read api_listen from config (default: 127.0.0.1:4545)
        let api_listen = {
            let cfg_path = librefang_dir.join("config.toml");
            if cfg_path.exists() {
                std::fs::read_to_string(&cfg_path)
                    .ok()
                    .and_then(|s| toml::from_str::<librefang_types::config::KernelConfig>(&s).ok())
                    .map(|c| c.api_listen)
                    .unwrap_or_else(|| librefang_types::config::DEFAULT_API_LISTEN.to_string())
            } else {
                librefang_types::config::DEFAULT_API_LISTEN.to_string()
            }
        };
        if !json {
            println!();
        }
        let daemon_running = find_daemon();
        if let Some(ref base) = daemon_running {
            if !json {
                ui::check_ok(&format!("Daemon running at {base}"));
            }
            checks.push(serde_json::json!({"check": "daemon", "status": "ok", "url": base}));
        } else {
            if !json {
                ui::check_warn("Daemon not running (start with `librefang start`)");
            }
            checks.push(serde_json::json!({"check": "daemon", "status": "warn"}));

            // Check if the configured port is available
            let bind_addr = if api_listen.starts_with("0.0.0.0") {
                api_listen.replacen("0.0.0.0", "127.0.0.1", 1)
            } else {
                api_listen.clone()
            };
            match std::net::TcpListener::bind(&bind_addr) {
                Ok(_) => {
                    if !json {
                        ui::check_ok(&format!("Port {api_listen} is available"));
                    }
                    checks.push(
                        serde_json::json!({"check": "port", "status": "ok", "address": api_listen}),
                    );
                }
                Err(_) => {
                    if !json {
                        ui::check_warn(&format!("Port {api_listen} is in use by another process"));
                    }
                    checks.push(serde_json::json!({"check": "port", "status": "warn", "address": api_listen}));
                }
            }
        }

        // --- Check 5: Stale daemon.json ---
        let daemon_json_path = librefang_dir.join("daemon.json");
        if daemon_json_path.exists() && daemon_running.is_none() {
            if repair {
                let _ = std::fs::remove_file(&daemon_json_path);
                if !json {
                    ui::check_ok("Removed stale daemon.json");
                }
                repaired = true;
            } else if !json {
                ui::check_warn(
                    "Stale daemon.json found (daemon not running). Run with --repair to clean up.",
                );
            }
            checks.push(serde_json::json!({"check": "stale_daemon_json", "status": if repair { "repaired" } else { "warn" }}));
        }

        // --- Check 6: Database file ---
        let db_path = librefang_dir.join("data").join("librefang.db");
        if db_path.exists() {
            // Quick SQLite magic bytes check
            if let Ok(bytes) = std::fs::read(&db_path) {
                if bytes.len() >= 16 && bytes.starts_with(b"SQLite format 3") {
                    if !json {
                        ui::check_ok("Database file (valid SQLite)");
                    }
                    checks.push(serde_json::json!({"check": "database", "status": "ok"}));
                } else {
                    if !json {
                        ui::check_fail("Database file exists but is not valid SQLite");
                    }
                    checks.push(serde_json::json!({"check": "database", "status": "fail"}));
                    all_ok = false;
                }
            }
        } else {
            if !json {
                ui::check_warn("No database file (will be created on first run)");
            }
            checks.push(serde_json::json!({"check": "database", "status": "warn"}));
        }

        // --- Check 7: Disk space ---
        #[cfg(unix)]
        {
            if let Ok(output) = std::process::Command::new("df")
                .args(["-m", &librefang_dir.display().to_string()])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Parse the available MB from df output (4th column of 2nd line)
                if let Some(line) = stdout.lines().nth(1) {
                    let cols: Vec<&str> = line.split_whitespace().collect();
                    if cols.len() >= 4 {
                        if let Ok(available_mb) = cols[3].parse::<u64>() {
                            if available_mb < 100 {
                                if !json {
                                    ui::check_warn(&format!(
                                        "Low disk space: {available_mb}MB available"
                                    ));
                                }
                                checks.push(serde_json::json!({"check": "disk_space", "status": "warn", "available_mb": available_mb}));
                            } else {
                                if !json {
                                    ui::check_ok(&format!(
                                        "Disk space: {available_mb}MB available"
                                    ));
                                }
                                checks.push(serde_json::json!({"check": "disk_space", "status": "ok", "available_mb": available_mb}));
                            }
                        }
                    }
                }
            }
        }

        // --- Check 8: Agent manifests parse correctly ---
        let agents_dir = librefang_dir.join("workspaces").join("agents");
        if agents_dir.exists() {
            let mut agent_errors = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&agents_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Err(e) = toml::from_str::<AgentManifest>(&content) {
                                agent_errors.push((
                                    path.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string(),
                                    e.to_string(),
                                ));
                            }
                        }
                    }
                }
            }
            if agent_errors.is_empty() {
                if !json {
                    ui::check_ok("Agent manifests are valid");
                }
                checks.push(serde_json::json!({"check": "agent_manifests", "status": "ok"}));
            } else {
                for (file, err) in &agent_errors {
                    if !json {
                        ui::check_fail(&format!("Invalid manifest {file}: {err}"));
                    }
                }
                checks.push(serde_json::json!({"check": "agent_manifests", "status": "fail", "errors": agent_errors.len()}));
                all_ok = false;
            }
        }
    } else {
        if !json {
            ui::check_fail("Could not determine home directory");
        }
        checks.push(serde_json::json!({"check": "home_dir", "status": "fail"}));
        all_ok = false;
    }

    // --- LLM providers ---
    if !json {
        println!("\n  LLM Providers:");
    }
    // Pretty display names for known provider IDs. Anything not listed
    // here falls back to a Title-Case derivation of the raw provider id
    // (e.g. `xiaomi` → `Xiaomi`). Adding a new provider to
    // `PROVIDER_REGISTRY` automatically picks up the fallback so the
    // check loop never silently misses a key — only the cosmetic name
    // needs editing here, not the list of providers checked.
    fn display_name(provider_id: &str) -> String {
        match provider_id {
            "openai" => "OpenAI".to_string(),
            "openrouter" => "OpenRouter".to_string(),
            "deepseek" => "DeepSeek".to_string(),
            "deepinfra" => "DeepInfra".to_string(),
            "byteplus" => "BytePlus".to_string(),
            "azure-openai" => "Azure OpenAI".to_string(),
            "github-copilot" => "GitHub Copilot".to_string(),
            "huggingface" => "Hugging Face".to_string(),
            "openai-codex" => "OpenAI Codex".to_string(),
            "claude-code" => "Claude Code".to_string(),
            "vertex-ai" => "Vertex AI".to_string(),
            "nvidia-nim" => "NVIDIA NIM".to_string(),
            "z.ai" | "zai" => "Z.ai".to_string(),
            "kimi-coding" | "kimi_coding" => "Kimi Coding".to_string(),
            "alibaba-coding-plan" => "Alibaba Coding Plan".to_string(),
            other => {
                // Title-case fallback for unlisted providers so `xiaomi` →
                // `Xiaomi` instead of leaking the raw lowercase id.
                let mut chars = other.chars();
                match chars.next() {
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        }
    }

    // Drive doctor off PROVIDER_REGISTRY so adding a provider to the
    // driver layer never requires a parallel edit here. `GOOGLE_API_KEY`
    // (gemini's alt env) and similar aliases come through automatically.
    // This subsumes the previous hardcoded array (including the byteplus
    // entry from #3274 — now provided automatically by the registry).
    let provider_specs = librefang_runtime::drivers::cloud_provider_key_specs();
    let provider_keys: Vec<(&str, String, &str)> = provider_specs
        .iter()
        .map(|(env_var, provider_id)| (*env_var, display_name(provider_id), *provider_id))
        .collect();

    let mut any_key_set = false;
    for (env_var, name, provider_id) in &provider_keys {
        let set = std::env::var(env_var).is_ok();
        if set {
            // --- Check 9: Live key validation ---
            let valid = test_api_key(provider_id, &std::env::var(env_var).unwrap_or_default());
            if valid {
                if !json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !json {
                ui::check_warn(&format!("{name} ({env_var}) - key rejected (401/403)"));
            }
            any_key_set = true;
            checks.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": if valid { "ok" } else { "warn" }, "live_test": !valid}));
        } else {
            if !json {
                ui::provider_status(name, env_var, false);
            }
            checks.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }

    if !any_key_set {
        if !json {
            println!();
            ui::check_fail(&i18n::t("doctor-no-api-keys"));
            ui::blank();
            ui::section(&i18n::t("section-getting-api-key"));
            ui::suggest_cmd("Groq:", "https://console.groq.com       (free, fast)");
            ui::suggest_cmd("Gemini:", "https://aistudio.google.com    (free tier)");
            ui::suggest_cmd("DeepSeek:", "https://platform.deepseek.com  (low cost)");
            ui::blank();
            ui::hint(&i18n::t("hint-set-key"));
        }
        all_ok = false;
    }

    // --- Check: Network connectivity to configured LLM provider endpoints ---
    {
        let provider_endpoints: &[(&str, &str, &str)] = &[
            ("OPENAI_API_KEY", "OpenAI", "api.openai.com:443"),
            ("ANTHROPIC_API_KEY", "Anthropic", "api.anthropic.com:443"),
            ("GROQ_API_KEY", "Groq", "api.groq.com:443"),
            ("DEEPSEEK_API_KEY", "DeepSeek", "api.deepseek.com:443"),
            (
                "GEMINI_API_KEY",
                "Gemini",
                "generativelanguage.googleapis.com:443",
            ),
            (
                "GOOGLE_API_KEY",
                "Google",
                "generativelanguage.googleapis.com:443",
            ),
            ("OPENROUTER_API_KEY", "OpenRouter", "openrouter.ai:443"),
            ("TOGETHER_API_KEY", "Together", "api.together.xyz:443"),
            ("MISTRAL_API_KEY", "Mistral", "api.mistral.ai:443"),
            ("FIREWORKS_API_KEY", "Fireworks", "api.fireworks.ai:443"),
        ];

        let configured: Vec<_> = provider_endpoints
            .iter()
            .filter(|(env_var, _, _)| std::env::var(env_var).is_ok())
            .collect();

        if !configured.is_empty() {
            if !json {
                println!("\n  Network Connectivity:");
            }
            for (env_var, name, endpoint) in &configured {
                use std::net::{TcpStream, ToSocketAddrs};
                let reachable = endpoint
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut addrs| addrs.next())
                    .map(|addr| {
                        TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3)).is_ok()
                    })
                    .unwrap_or(false);

                if reachable {
                    if !json {
                        ui::check_ok(&format!("{name} endpoint reachable ({endpoint})"));
                    }
                    checks.push(serde_json::json!({"check": "network_connectivity", "provider": name, "endpoint": endpoint, "env_var": env_var, "status": "ok"}));
                } else {
                    if !json {
                        ui::check_warn(&format!("{name} endpoint unreachable ({endpoint})"));
                    }
                    checks.push(serde_json::json!({"check": "network_connectivity", "provider": name, "endpoint": endpoint, "env_var": env_var, "status": "warn"}));
                }
            }
        }
    }

    // --- Check 10: Channel token format validation ---
    if !json {
        println!("\n  Channel Integrations:");
    }
    let channel_keys = [
        ("TELEGRAM_BOT_TOKEN", "Telegram"),
        ("DISCORD_BOT_TOKEN", "Discord"),
        ("SLACK_APP_TOKEN", "Slack App"),
        ("SLACK_BOT_TOKEN", "Slack Bot"),
    ];
    for (env_var, name) in &channel_keys {
        let set = std::env::var(env_var).is_ok();
        if set {
            // Format validation
            let val = std::env::var(env_var).unwrap_or_default();
            let format_ok = match *env_var {
                "TELEGRAM_BOT_TOKEN" => val.contains(':'), // Telegram tokens have format "123456:ABC-DEF..."
                "DISCORD_BOT_TOKEN" => val.len() > 50,     // Discord tokens are typically 59+ chars
                "SLACK_APP_TOKEN" => val.starts_with("xapp-"),
                "SLACK_BOT_TOKEN" => val.starts_with("xoxb-"),
                _ => true,
            };
            if format_ok {
                if !json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !json {
                ui::check_warn(&format!("{name} ({env_var}) - unexpected token format"));
            }
            checks.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": if format_ok { "ok" } else { "warn" }}));
        } else {
            if !json {
                ui::provider_status(name, env_var, false);
            }
            checks.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }

    // --- Check 11: .env keys vs config api_key_env consistency ---
    {
        let librefang_dir = cli_librefang_home();
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
            // Look for api_key_env references in config
            for line in config_str.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("api_key_env") {
                    if let Some(val_part) = rest.strip_prefix('=') {
                        let val = val_part.trim().trim_matches('"');
                        if !val.is_empty() && std::env::var(val).is_err() {
                            if !json {
                                ui::check_warn(&format!(
                                    "Config references {val} but it is not set in env or .env"
                                ));
                            }
                            checks.push(serde_json::json!({"check": "env_consistency", "status": "warn", "missing_var": val}));
                        }
                    }
                }
            }
        }
    }

    // --- Check 12: Config deserialization into KernelConfig ---
    {
        let librefang_dir = cli_librefang_home();
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            if !json {
                println!("\n  Config Validation:");
            }
            let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str::<librefang_types::config::KernelConfig>(&config_content) {
                Ok(cfg) => {
                    if !json {
                        ui::check_ok("Config deserializes into KernelConfig");
                    }
                    checks.push(serde_json::json!({"check": "config_deser", "status": "ok"}));

                    // Check exec policy
                    let mode = format!("{:?}", cfg.exec_policy.mode);
                    let safe_bins_count = cfg.exec_policy.safe_bins.len();
                    if !json {
                        ui::check_ok(&format!(
                            "Exec policy: mode={mode}, safe_bins={safe_bins_count}"
                        ));
                    }
                    checks.push(serde_json::json!({"check": "exec_policy", "status": "ok", "mode": mode, "safe_bins": safe_bins_count}));

                    // Check includes
                    if !cfg.include.is_empty() {
                        let mut include_ok = true;
                        for inc in &cfg.include {
                            let inc_path = librefang_dir.join(inc);
                            if inc_path.exists() {
                                if !json {
                                    ui::check_ok(&format!("Include file: {inc}"));
                                }
                            } else if repair {
                                if !json {
                                    ui::check_warn(&format!("Include file missing: {inc}"));
                                }
                                include_ok = false;
                            } else {
                                if !json {
                                    ui::check_fail(&format!("Include file not found: {inc}"));
                                }
                                include_ok = false;
                                all_ok = false;
                            }
                        }
                        checks.push(serde_json::json!({"check": "config_includes", "status": if include_ok { "ok" } else { "fail" }, "count": cfg.include.len()}));
                    }

                    // Check MCP server configs
                    if !cfg.mcp_servers.is_empty() {
                        let mcp_count = cfg.mcp_servers.len();
                        if !json {
                            ui::check_ok(&format!("MCP servers configured: {mcp_count}"));
                        }
                        for server in &cfg.mcp_servers {
                            // Validate transport config
                            let Some(ref transport) = server.transport else {
                                continue;
                            };
                            match transport {
                                librefang_types::config::McpTransportEntry::Stdio {
                                    command,
                                    ..
                                } => {
                                    if command.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has empty command",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                                librefang_types::config::McpTransportEntry::Sse { url }
                                | librefang_types::config::McpTransportEntry::Http { url } => {
                                    if url.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has empty URL",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                                librefang_types::config::McpTransportEntry::HttpCompat {
                                    base_url,
                                    headers,
                                    tools,
                                } => {
                                    if base_url.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has empty base_url",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has no http_compat tools configured",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if headers.iter().any(|h| h.name.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has an http_compat header with empty name",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if headers.iter().any(|h| {
                                        h.value.as_ref().is_none_or(|value| value.trim().is_empty())
                                            && h.value_env
                                                .as_ref()
                                                .is_none_or(|value| value.trim().is_empty())
                                    }) {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has an http_compat header without value/value_env",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.iter().any(|tool| tool.name.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has an http_compat tool with empty name",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.iter().any(|tool| tool.path.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has an http_compat tool with empty path",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                            }
                        }
                        checks.push(serde_json::json!({"check": "mcp_servers", "status": "ok", "count": mcp_count}));
                    }
                }
                Err(e) => {
                    if !json {
                        ui::check_fail(&format!("Config fails KernelConfig deserialization: {e}"));
                    }
                    checks.push(serde_json::json!({"check": "config_deser", "status": "fail", "error": e.to_string()}));
                    all_ok = false;
                }
            }
        }
    }

    // --- Check 13: Skill registry health ---
    {
        if !json {
            println!("\n  Skills:");
        }
        let skills_dir = cli_librefang_home().join("skills");
        let mut skill_reg = librefang_skills::registry::SkillRegistry::new(skills_dir.clone());
        match skill_reg.load_all() {
            Ok(count) => {
                if !json {
                    ui::check_ok(&format!("Skills loaded: {count}"));
                }
                checks.push(serde_json::json!({"check": "skills", "status": "ok", "count": count}));
            }
            Err(e) => {
                if !json {
                    ui::check_warn(&format!("Failed to load skills: {e}"));
                }
                checks.push(serde_json::json!({"check": "skills", "status": "warn", "error": e.to_string()}));
            }
        }

        // Check for prompt injection issues in skill definitions.
        // Only flag Critical-severity warnings.
        let skills = skill_reg.list();
        let mut injection_warnings = 0;
        for skill in &skills {
            if let Some(ref prompt) = skill.manifest.prompt_context {
                let warnings = librefang_skills::verify::SkillVerifier::scan_prompt_content(prompt);
                let has_critical = warnings.iter().any(|w| {
                    matches!(
                        w.severity,
                        librefang_skills::verify::WarningSeverity::Critical
                    )
                });
                if has_critical {
                    injection_warnings += 1;
                    if !json {
                        ui::check_warn(&format!(
                            "Prompt injection warning in skill: {}",
                            skill.manifest.skill.name
                        ));
                    }
                }
            }
        }
        if injection_warnings > 0 {
            checks.push(serde_json::json!({"check": "skill_injection_scan", "status": "warn", "warnings": injection_warnings}));
        } else {
            if !json {
                ui::check_ok("All skills pass prompt injection scan");
            }
            checks.push(serde_json::json!({"check": "skill_injection_scan", "status": "ok"}));
        }
    }

    // --- Check 14: MCP catalog + configured servers ---
    {
        if !json {
            println!("\n  MCP servers:");
        }
        let librefang_dir = cli_librefang_home();
        let mut catalog = librefang_extensions::catalog::McpCatalog::new(&librefang_dir);
        catalog.load(&librefang_runtime::registry_sync::resolve_home_dir_for_tests());
        let template_count = catalog.len();

        // Count configured [[mcp_servers]] entries in config.toml (if any).
        let configured_count = {
            let config_path = librefang_dir.join("config.toml");
            if config_path.is_file() {
                let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
                toml::from_str::<toml::Value>(&raw)
                    .ok()
                    .and_then(|v| v.as_table().cloned())
                    .and_then(|t| t.get("mcp_servers").cloned())
                    .and_then(|v| v.as_array().cloned())
                    .map(|a| a.len())
                    .unwrap_or(0)
            } else {
                0
            }
        };
        if !json {
            ui::check_ok(&format!("MCP catalog templates: {template_count}"));
            ui::check_ok(&format!("Configured MCP servers: {configured_count}"));
        }
        checks.push(
            serde_json::json!({"check": "mcp_catalog", "status": "ok", "count": template_count}),
        );
        checks.push(serde_json::json!({"check": "mcp_servers_configured", "status": "ok", "count": configured_count}));
    }

    // --- Check 15: Daemon health detail (if running) ---
    if let Some(ref base) = find_daemon() {
        if !json {
            println!("\n  Daemon Health:");
        }
        let client = daemon_client();
        match client.get(format!("{base}/api/health/detail")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
                        if !json {
                            ui::check_ok(&format!("Running agents: {agents}"));
                        }
                        checks.push(serde_json::json!({"check": "daemon_agents", "status": "ok", "count": agents}));
                    }
                    if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                        let hours = uptime / 3600;
                        let mins = (uptime % 3600) / 60;
                        if !json {
                            ui::check_ok(&format!("Daemon uptime: {hours}h {mins}m"));
                        }
                        checks.push(serde_json::json!({"check": "daemon_uptime", "status": "ok", "secs": uptime}));
                    }
                    if let Some(db_status) = body.get("database").and_then(|v| v.as_str()) {
                        if db_status == "connected" || db_status == "ok" {
                            if !json {
                                ui::check_ok("Database connectivity: OK");
                            }
                        } else {
                            if !json {
                                ui::check_fail(&format!("Database status: {db_status}"));
                            }
                            all_ok = false;
                        }
                        checks.push(serde_json::json!({"check": "daemon_db", "status": db_status}));
                    }
                }
            }
            Ok(resp) => {
                if !json {
                    ui::check_warn(&format!("Health detail returned {}", resp.status()));
                }
                checks.push(serde_json::json!({"check": "daemon_health", "status": "warn"}));
            }
            Err(e) => {
                if !json {
                    ui::check_warn(&format!("Failed to query daemon health: {e}"));
                }
                checks.push(serde_json::json!({"check": "daemon_health", "status": "warn", "error": e.to_string()}));
            }
        }

        // Check skills endpoint
        match client.get(format!("{base}/api/skills")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(arr) = body
                        .get("skills")
                        .and_then(|v| v.as_array())
                        .or_else(|| body.as_array())
                    {
                        if !json {
                            ui::check_ok(&format!("Skills loaded in daemon: {}", arr.len()));
                        }
                        checks.push(serde_json::json!({"check": "daemon_skills", "status": "ok", "count": arr.len()}));
                    }
                }
            }
            _ => {}
        }

        // Check MCP servers endpoint
        match client.get(format!("{base}/api/mcp/servers")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(arr) = body
                        .get("configured")
                        .and_then(|v| v.as_array())
                        .or_else(|| body.as_array())
                    {
                        let connected = arr
                            .iter()
                            .filter(|s| {
                                s.get("connected")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                            })
                            .count();
                        if !json {
                            ui::check_ok(&format!(
                                "MCP servers: {} configured, {} connected",
                                arr.len(),
                                connected
                            ));
                        }
                        checks.push(serde_json::json!({"check": "daemon_mcp", "status": "ok", "configured": arr.len(), "connected": connected}));
                    }
                }
            }
            _ => {}
        }

        // Check MCP health endpoint
        match client.get(format!("{base}/api/mcp/health")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries = body.get("health").and_then(|h| h.as_array());
                    if let Some(arr) = entries {
                        let healthy = arr
                            .iter()
                            .filter(|v| {
                                v.get("status")
                                    .and_then(|s| s.as_str())
                                    .map(|s| s.eq_ignore_ascii_case("ready"))
                                    .unwrap_or(false)
                            })
                            .count();
                        let total = arr.len();
                        if healthy == total {
                            if !json {
                                ui::check_ok(&format!(
                                    "MCP server health: {healthy}/{total} healthy"
                                ));
                            }
                        } else if !json {
                            ui::check_warn(&format!(
                                "MCP server health: {healthy}/{total} healthy"
                            ));
                        }
                        checks.push(serde_json::json!({"check": "mcp_health", "status": if healthy == total { "ok" } else { "warn" }, "healthy": healthy, "total": total}));
                    }
                }
            }
            _ => {}
        }
    }

    if !json {
        println!();
    }
    match std::process::Command::new("rustc")
        .arg("--version")
        .output()
    {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Rust: {version}"));
            }
            checks.push(serde_json::json!({"check": "rust", "status": "ok", "version": version}));
        }
        Err(_) => {
            if !json {
                ui::check_fail("Rust toolchain not found");
            }
            checks.push(serde_json::json!({"check": "rust", "status": "fail"}));
            all_ok = false;
        }
    }

    // Python runtime check
    match std::process::Command::new("python3")
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Python: {version}"));
            }
            checks.push(serde_json::json!({"check": "python", "status": "ok", "version": version}));
        }
        _ => {
            // Try `python` instead
            match std::process::Command::new("python")
                .arg("--version")
                .output()
            {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !json {
                        ui::check_ok(&format!("Python: {version}"));
                    }
                    checks.push(
                        serde_json::json!({"check": "python", "status": "ok", "version": version}),
                    );
                }
                _ => {
                    if !json {
                        ui::check_warn("Python not found (needed for Python skill runtime)");
                    }
                    checks.push(serde_json::json!({"check": "python", "status": "warn"}));
                }
            }
        }
    }

    // Node.js runtime check
    match std::process::Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Node.js: {version}"));
            }
            checks.push(serde_json::json!({"check": "node", "status": "ok", "version": version}));
        }
        _ => {
            if !json {
                ui::check_warn("Node.js not found (needed for Node skill runtime)");
            }
            checks.push(serde_json::json!({"check": "node", "status": "warn"}));
        }
    }

    // Framework-based audit checks (see crates/librefang-cli/src/doctor.rs).
    // Each check is its own struct, registered in `doctor::registered_checks`.
    // Migrating the legacy inline checks above into this framework can happen
    // incrementally — adding a new check is one struct + one registry entry,
    // no edits to this function.
    {
        let ctx = doctor::AuditContext {
            librefang_home: cli_librefang_home(),
        };
        for result in doctor::run_all(&ctx) {
            if !json {
                match result.severity {
                    doctor::Severity::Pass | doctor::Severity::Info => {
                        ui::check_ok(&result.summary);
                    }
                    doctor::Severity::Warn => {
                        ui::check_warn(&result.summary);
                        if let Some(hint) = &result.hint {
                            ui::hint(hint);
                        }
                    }
                    doctor::Severity::Error => {
                        ui::check_fail(&result.summary);
                        if let Some(hint) = &result.hint {
                            ui::hint(hint);
                        }
                    }
                }
            }
            let mut entry = serde_json::json!({
                "check": result.name,
                "status": result.severity.as_str(),
                "summary": result.summary,
            });
            if let Some(h) = &result.hint {
                entry["hint"] = serde_json::Value::String(h.clone());
            }
            checks.push(entry);
            if matches!(result.severity, doctor::Severity::Error) {
                all_ok = false;
            }
        }
    }

    if json {
        write_stdout_safe(
            &serde_json::to_string_pretty(&serde_json::json!({
                "all_ok": all_ok,
                "checks": checks,
            }))
            .unwrap_or_default(),
        );
    } else {
        println!();
        if all_ok {
            ui::success(&i18n::t("doctor-all-passed"));
            ui::hint(&i18n::t("hint-start-daemon-cmd"));
        } else if repaired {
            ui::success(&i18n::t("doctor-repairs-applied"));
        } else {
            ui::error(&i18n::t("doctor-some-failed"));
            if !repair {
                ui::hint(&i18n::t("hint-doctor-repair"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dashboard command
// ---------------------------------------------------------------------------

fn cmd_dashboard() {
    let base = if let Some(url) = find_daemon() {
        url
    } else {
        // Auto-start the daemon
        ui::hint(&i18n::t("daemon-no-running-auto"));
        match start_daemon_background() {
            Ok(url) => {
                ui::success(&i18n::t("daemon-started"));
                url
            }
            Err(e) => {
                ui::error_with_fix(
                    &i18n::t_args("daemon-start-fail", &[("error", &e.to_string())]),
                    &i18n::t("daemon-start-fail-fix"),
                );
                std::process::exit(1);
            }
        }
    };

    let url = format!("{base}/");
    ui::success(&i18n::t_args("dashboard-opening", &[("url", &url)]));
    if copy_to_clipboard(&url) {
        ui::hint(&i18n::t("hint-url-copied"));
    }
    if !open_in_browser(&url) {
        ui::hint(&i18n::t_args(
            "hint-could-not-open-browser-visit",
            &[("url", &url)],
        ));
    }
}

// ---------------------------------------------------------------------------
// Shell completion command
// ---------------------------------------------------------------------------

fn cmd_completion(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "librefang", &mut std::io::stdout());
}

// ---------------------------------------------------------------------------
// Workflow commands
// ---------------------------------------------------------------------------

fn cmd_workflow_list() {
    let base = require_daemon("workflow list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/workflows")).send());

    match body.as_array() {
        Some(workflows) if workflows.is_empty() => println!("No workflows registered."),
        Some(workflows) => {
            let mut t = crate::table::Table::new(&["ID", "NAME", "STEPS", "CREATED"]);
            for w in workflows {
                t.add_row(&[
                    w["id"].as_str().unwrap_or("?"),
                    w["name"].as_str().unwrap_or("?"),
                    &w["steps"].as_u64().unwrap_or(0).to_string(),
                    w["created_at"].as_str().unwrap_or("?"),
                ]);
            }
            t.print();
        }
        None => println!("No workflows registered."),
    }
}

fn cmd_workflow_create(file: PathBuf) {
    let base = require_daemon("workflow create");
    if !file.exists() {
        eprintln!("Workflow file not found: {}", file.display());
        std::process::exit(1);
    }
    let contents = std::fs::read_to_string(&file).unwrap_or_else(|e| {
        eprintln!("Error reading workflow file: {e}");
        std::process::exit(1);
    });
    let json_body: serde_json::Value = serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!("Invalid JSON: {e}");
        std::process::exit(1);
    });

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows"))
            .json(&json_body)
            .send(),
    );

    if let Some(id) = body["workflow_id"].as_str() {
        println!("Workflow created successfully!");
        println!("  ID: {id}");
    } else {
        eprintln!(
            "Failed to create workflow: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_workflow_run(workflow_id: &str, input: &str) {
    let base = require_daemon("workflow run");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows/{workflow_id}/run"))
            .json(&serde_json::json!({"input": input}))
            .send(),
    );

    if let Some(output) = body["output"].as_str() {
        println!("Workflow completed!");
        println!("  Run ID: {}", body["run_id"].as_str().unwrap_or("?"));
        println!("  Output:\n{output}");
    } else {
        eprintln!(
            "Workflow failed: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Trigger commands
// ---------------------------------------------------------------------------

fn cmd_trigger_list(agent_id: Option<&str>) {
    let base = require_daemon("trigger list");
    let client = daemon_client();

    let url = match agent_id {
        Some(id) => format!("{base}/api/triggers?agent_id={id}"),
        None => format!("{base}/api/triggers"),
    };
    let body = daemon_json(client.get(&url).send());

    let arr = body["triggers"].as_array().or_else(|| body.as_array());
    match arr {
        Some(triggers) if triggers.is_empty() => println!("No triggers registered."),
        Some(triggers) => {
            let mut tbl = crate::table::Table::new(&[
                "TRIGGER ID",
                "AGENT ID",
                "ENABLED",
                "FIRES",
                "PATTERN",
            ]);
            for t in triggers {
                tbl.add_row(&[
                    t["id"].as_str().unwrap_or("?"),
                    t["agent_id"].as_str().unwrap_or("?"),
                    &t["enabled"].as_bool().unwrap_or(false).to_string(),
                    &t["fire_count"].as_u64().unwrap_or(0).to_string(),
                    t["pattern"].as_str().unwrap_or("?"),
                ]);
            }
            tbl.print();
        }
        None => println!("No triggers registered."),
    }
}

fn cmd_trigger_create(
    agent_id: &str,
    pattern_json: &str,
    prompt: &str,
    max_fires: u64,
    target_agent: Option<&str>,
    cooldown: Option<u64>,
    session_mode: Option<&str>,
) {
    let base = require_daemon("trigger create");
    let agent_id = resolve_agent_id(&base, agent_id);
    let pattern: serde_json::Value = serde_json::from_str(pattern_json).unwrap_or_else(|e| {
        eprintln!("Invalid pattern JSON: {e}");
        eprintln!("Examples:");
        eprintln!("  '\"lifecycle\"'");
        eprintln!("  '{{\"agent_spawned\":{{\"name_pattern\":\"*\"}}}}'");
        eprintln!("  '\"agent_terminated\"'");
        eprintln!("  '\"all\"'");
        std::process::exit(1);
    });

    let mut payload = serde_json::json!({
        "agent_id": agent_id,
        "pattern": pattern,
        "prompt_template": prompt,
        "max_fires": max_fires,
    });
    if let Some(t) = target_agent {
        payload["target_agent_id"] = serde_json::json!(t);
    }
    if let Some(c) = cooldown {
        payload["cooldown_secs"] = serde_json::json!(c);
    }
    if let Some(m) = session_mode {
        payload["session_mode"] = serde_json::json!(m);
    }

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/triggers"))
            .json(&payload)
            .send(),
    );

    if let Some(id) = body["trigger_id"].as_str() {
        println!("Trigger created successfully!");
        println!("  Trigger ID: {id}");
        println!("  Agent ID:   {agent_id}");
        if let Some(t) = target_agent {
            println!("  Target:     {t}");
        }
    } else {
        eprintln!(
            "Failed to create trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_trigger_delete(trigger_id: &str) {
    let base = require_daemon("trigger delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/triggers/{trigger_id}"))
            .send(),
    );

    if body.get("status").is_some() {
        println!("Trigger {trigger_id} deleted.");
    } else {
        eprintln!(
            "Failed to delete trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_trigger_get(trigger_id: &str) {
    let base = require_daemon("trigger get");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/triggers/{trigger_id}"))
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to get trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }

    println!("Trigger ID:    {}", body["id"].as_str().unwrap_or("-"));
    println!(
        "Agent ID:      {}",
        body["agent_id"].as_str().unwrap_or("-")
    );
    println!("Pattern:       {}", body["pattern"]);
    println!(
        "Prompt:        {}",
        body["prompt_template"].as_str().unwrap_or("-")
    );
    println!(
        "Enabled:       {}",
        body["enabled"].as_bool().unwrap_or(false)
    );
    println!(
        "Fire count:    {}",
        body["fire_count"].as_u64().unwrap_or(0)
    );
    println!(
        "Max fires:     {}",
        body["max_fires"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unlimited".to_string())
    );
    if let Some(t) = body["target_agent_id"].as_str() {
        println!("Target agent:  {t}");
    }
    if let Some(c) = body["cooldown_secs"].as_u64() {
        println!("Cooldown:      {c}s");
    }
    if let Some(m) = body["session_mode"].as_str() {
        println!("Session mode:  {m}");
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_trigger_update(
    trigger_id: &str,
    pattern: Option<&str>,
    prompt: Option<&str>,
    enabled: Option<bool>,
    max_fires: Option<u64>,
    cooldown: Option<u64>,
    clear_cooldown: bool,
    session_mode: Option<&str>,
    clear_session_mode: bool,
    target_agent: Option<&str>,
    clear_target_agent: bool,
) {
    let base = require_daemon("trigger update");
    let client = daemon_client();

    let mut payload = serde_json::json!({});
    if let Some(p) = pattern {
        let parsed: serde_json::Value = serde_json::from_str(p).unwrap_or_else(|e| {
            eprintln!("Invalid pattern JSON: {e}");
            std::process::exit(1);
        });
        payload["pattern"] = parsed;
    }
    if let Some(t) = prompt {
        payload["prompt_template"] = serde_json::json!(t);
    }
    if let Some(e) = enabled {
        payload["enabled"] = serde_json::json!(e);
    }
    if let Some(m) = max_fires {
        payload["max_fires"] = serde_json::json!(m);
    }
    if clear_cooldown {
        payload["cooldown_secs"] = serde_json::Value::Null;
    } else if let Some(c) = cooldown {
        payload["cooldown_secs"] = serde_json::json!(c);
    }
    if clear_session_mode {
        payload["session_mode"] = serde_json::Value::Null;
    } else if let Some(m) = session_mode {
        payload["session_mode"] = serde_json::json!(m);
    }
    if clear_target_agent {
        payload["target_agent_id"] = serde_json::Value::Null;
    } else if let Some(a) = target_agent {
        payload["target_agent_id"] = serde_json::json!(a);
    }

    let body = daemon_json(
        client
            .patch(format!("{base}/api/triggers/{trigger_id}"))
            .json(&payload)
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to update trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
    println!("Trigger {trigger_id} updated.");
}

fn cmd_trigger_set_enabled(trigger_id: &str, enabled: bool) {
    let base = require_daemon(if enabled {
        "trigger enable"
    } else {
        "trigger disable"
    });
    let client = daemon_client();
    let payload = serde_json::json!({ "enabled": enabled });
    let body = daemon_json(
        client
            .patch(format!("{base}/api/triggers/{trigger_id}"))
            .json(&payload)
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to {} trigger: {}",
            if enabled { "enable" } else { "disable" },
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
    println!(
        "Trigger {trigger_id} {}.",
        if enabled { "enabled" } else { "disabled" }
    );
}

// ---------------------------------------------------------------------------
// Migrate command
// ---------------------------------------------------------------------------

fn cmd_migrate(args: MigrateArgs) {
    let source = match args.from {
        MigrateSourceArg::Openclaw => librefang_import::MigrateSource::OpenClaw,
        MigrateSourceArg::Langchain => librefang_import::MigrateSource::LangChain,
        MigrateSourceArg::Autogpt => librefang_import::MigrateSource::AutoGpt,
        MigrateSourceArg::Openfang => librefang_import::MigrateSource::OpenFang,
    };

    let source_dir = args.source_dir.unwrap_or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        });
        match source {
            librefang_import::MigrateSource::OpenClaw => home.join(".openclaw"),
            librefang_import::MigrateSource::LangChain => home.join(".langchain"),
            librefang_import::MigrateSource::AutoGpt => home.join("Auto-GPT"),
            librefang_import::MigrateSource::OpenFang => home.join(".openfang"),
        }
    });

    let target_dir = cli_librefang_home();

    println!("Migrating from {} ({})...", source, source_dir.display());
    if args.dry_run {
        println!("  (dry run — no changes will be made)\n");
    }

    let options = librefang_import::MigrateOptions {
        source,
        source_dir,
        target_dir,
        dry_run: args.dry_run,
    };

    let mut sp = progress::auto("Running migration", None);
    match librefang_import::run_migration(&options) {
        Ok(report) => {
            sp.finish("Migration complete");
            report.print_summary();

            // Save migration report
            if !args.dry_run {
                let report_path = options.target_dir.join("migration_report.md");
                if let Err(e) = std::fs::write(&report_path, report.to_markdown()) {
                    eprintln!("Warning: Could not save migration report: {e}");
                } else {
                    println!("\n  Report saved to: {}", report_path.display());
                }
            }
        }
        Err(e) => {
            sp.finish_with_failure(&format!("Migration failed: {e}"));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Skill commands
// ---------------------------------------------------------------------------

/// Resolve the skills directory: global or per-hand workspace.
fn resolve_skills_dir(hand: Option<&str>) -> PathBuf {
    let home = librefang_home();
    match hand {
        None => home.join("skills"),
        Some(hand_id) => {
            let hand_dir = home.join("workspaces").join("hands").join(hand_id);
            if !hand_dir.exists() {
                eprintln!("Hand '{hand_id}' not found at {}", hand_dir.display());
                std::process::exit(1);
            }
            hand_dir.join("skills")
        }
    }
}

fn cmd_skill_install(source: &str, hand: Option<&str>) {
    let skills_dir = resolve_skills_dir(hand);
    std::fs::create_dir_all(&skills_dir).unwrap_or_else(|e| {
        eprintln!("Error creating skills directory: {e}");
        std::process::exit(1);
    });

    let source_path = PathBuf::from(source);
    if source_path.exists() && source_path.is_dir() {
        // Local directory install
        let manifest_path = source_path.join("skill.toml");
        if !manifest_path.exists() {
            // Check if it's an OpenClaw skill
            if librefang_skills::openclaw_compat::detect_openclaw_skill(&source_path) {
                println!("Detected OpenClaw skill format. Converting...");
                match librefang_skills::openclaw_compat::convert_openclaw_skill(&source_path) {
                    Ok(manifest) => {
                        let dest = skills_dir.join(&manifest.skill.name);
                        // Copy skill directory
                        copy_dir_recursive(&source_path, &dest);
                        if let Err(e) = librefang_skills::openclaw_compat::write_librefang_manifest(
                            &dest, &manifest,
                        ) {
                            eprintln!("Failed to write manifest: {e}");
                            std::process::exit(1);
                        }
                        if let Some(h) = hand {
                            println!(
                                "Installed OpenClaw skill '{}' to hand '{h}'",
                                manifest.skill.name
                            );
                        } else {
                            println!("Installed OpenClaw skill: {}", manifest.skill.name);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to convert OpenClaw skill: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            eprintln!("No skill.toml found in {source}");
            std::process::exit(1);
        }

        // Read manifest to get skill name
        let toml_str = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            eprintln!("Error reading skill.toml: {e}");
            std::process::exit(1);
        });
        let manifest: librefang_skills::SkillManifest =
            toml::from_str(&toml_str).unwrap_or_else(|e| {
                eprintln!("Error parsing skill.toml: {e}");
                std::process::exit(1);
            });

        let dest = skills_dir.join(&manifest.skill.name);
        copy_dir_recursive(&source_path, &dest);
        if let Some(h) = hand {
            println!(
                "Installed skill '{}' v{} to hand '{h}'",
                manifest.skill.name, manifest.skill.version
            );
        } else {
            println!(
                "Installed skill: {} v{}",
                manifest.skill.name, manifest.skill.version
            );
        }
    } else {
        // Remote install from FangHub
        let mut sp = progress::auto(&format!("Installing {source}"), None);
        sp.tick(1);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = librefang_skills::marketplace::MarketplaceClient::new(
            librefang_skills::marketplace::MarketplaceConfig::default(),
        );
        match rt.block_on(client.install(source, &skills_dir)) {
            Ok(version) => {
                if let Some(h) = hand {
                    sp.finish(&format!("Installed {source} {version} to hand '{h}'"));
                } else {
                    sp.finish(&format!("Installed {source} {version}"));
                }
            }
            Err(e) => {
                sp.finish_with_failure(&format!("Failed to install skill: {e}"));
                std::process::exit(1);
            }
        }
    }
}

fn cmd_skill_list(hand: Option<&str>) {
    let skills_dir = resolve_skills_dir(hand);

    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir);
    match registry.load_all() {
        Ok(0) => {
            if let Some(h) = hand {
                println!("No skills installed for hand '{h}'.");
            } else {
                println!("No skills installed.");
            }
        }
        Ok(count) => {
            if let Some(h) = hand {
                println!("{count} skill(s) installed for hand '{h}':\n");
            } else {
                println!("{count} skill(s) installed:\n");
            }
            let mut t = crate::table::Table::new(&["NAME", "VERSION", "TOOLS", "DESCRIPTION"]);
            for skill in registry.list() {
                t.add_row(&[
                    &skill.manifest.skill.name,
                    &skill.manifest.skill.version,
                    &skill.manifest.tools.provided.len().to_string(),
                    &skill.manifest.skill.description,
                ]);
            }
            t.print();
        }
        Err(e) => {
            eprintln!("Error loading skills: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_skill_remove(name: &str, hand: Option<&str>) {
    // Route through the safe uninstall path (lock + path-traversal
    // guard) instead of `registry.remove()` which calls `remove_dir_all`
    // with no serialisation against concurrent evolve operations.
    let skills_dir = resolve_skills_dir(hand);
    match librefang_skills::evolution::uninstall_skill(&skills_dir, name) {
        Ok(_) => {
            if let Some(h) = hand {
                println!("Removed skill '{name}' from hand '{h}'");
            } else {
                println!("Removed skill: {name}");
            }
        }
        Err(e) => {
            eprintln!("Failed to remove skill: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_skill_search(query: &str) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = librefang_skills::marketplace::MarketplaceClient::new(
        librefang_skills::marketplace::MarketplaceConfig::default(),
    );
    match rt.block_on(client.search(query)) {
        Ok(results) if results.is_empty() => println!("No skills found for \"{query}\"."),
        Ok(results) => {
            println!("Skills matching \"{query}\":\n");
            for r in results {
                println!("  {} ({})", r.name, r.stars);
                if !r.description.is_empty() {
                    println!("    {}", r.description);
                }
                println!("    {}", r.url);
                println!();
            }
        }
        Err(e) => {
            eprintln!("Search failed: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_skill_test(path: Option<PathBuf>, tool: Option<String>, input: Option<String>) {
    let skill_path = resolve_skill_path(path);
    let prepared =
        librefang_skills::publish::prepare_local_skill(&skill_path).unwrap_or_else(|e| {
            eprintln!("Skill validation failed: {e}");
            std::process::exit(1);
        });

    println!(
        "Validated skill: {} v{}",
        prepared.manifest.skill.name, prepared.manifest.skill.version
    );
    println!(
        "  Runtime: {:?}\n  Source: {}",
        prepared.manifest.runtime.runtime_type,
        prepared.source_dir.display()
    );
    if !prepared.manifest.skill.description.is_empty() {
        println!("  Description: {}", prepared.manifest.skill.description);
    }
    if !prepared.manifest.tools.provided.is_empty() {
        println!(
            "  Tools: {}",
            prepared
                .manifest
                .tools
                .provided
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    print_skill_warnings(&prepared.warnings);

    if prepared.has_critical_warnings() {
        eprintln!("Refusing to execute a skill with critical validation warnings.");
        std::process::exit(1);
    }

    let Some(tool_name) = tool.or_else(|| {
        prepared
            .manifest
            .tools
            .provided
            .first()
            .map(|tool| tool.name.clone())
    }) else {
        println!("Validation only: no tool declared to execute.");
        return;
    };

    let input_json = match input {
        Some(input) => serde_json::from_str::<serde_json::Value>(&input).unwrap_or_else(|err| {
            eprintln!("Invalid --input JSON: {err}");
            std::process::exit(1);
        }),
        None => serde_json::json!({}),
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = if prepared.manifest.runtime.runtime_type == librefang_skills::SkillRuntime::Wasm {
        // WASM skills execute in the real sandbox. We pass no kernel handle:
        // pure-compute tools run end to end, while capability-bearing host
        // calls return an error in the result rather than crashing — the right
        // behaviour for a local smoke test outside a running daemon.
        rt.block_on(librefang_runtime::tool_runner::execute_wasm_skill(
            &prepared.manifest,
            &prepared.source_dir,
            &tool_name,
            &input_json,
            None,
            "cli-test",
        ))
    } else {
        let env_policy = load_skill_env_policy_from_config();
        rt.block_on(librefang_skills::loader::execute_skill_tool(
            &prepared.manifest,
            &prepared.source_dir,
            &tool_name,
            &input_json,
            env_policy.as_ref(),
        ))
    };
    match result {
        Ok(result) => {
            println!("\nTool result ({tool_name}):");
            println!(
                "{}",
                serde_json::to_string_pretty(&result.output).unwrap_or_default()
            );
            if result.is_error {
                std::process::exit(1);
            }
        }
        Err(librefang_skills::SkillError::RuntimeNotAvailable(message)) => {
            println!("\nValidation complete.");
            println!("Execution skipped: {message}");
        }
        Err(err) => {
            eprintln!("Skill execution failed: {err}");
            std::process::exit(1);
        }
    }
}

fn cmd_skill_publish(
    path: Option<PathBuf>,
    repo: Option<String>,
    tag: Option<String>,
    output: Option<PathBuf>,
    dry_run: bool,
) {
    let skill_path = resolve_skill_path(path);
    let prepared =
        librefang_skills::publish::prepare_local_skill(&skill_path).unwrap_or_else(|e| {
            eprintln!("Skill validation failed: {e}");
            std::process::exit(1);
        });

    println!(
        "Preparing skill: {} v{}",
        prepared.manifest.skill.name, prepared.manifest.skill.version
    );
    print_skill_warnings(&prepared.warnings);
    if prepared.has_critical_warnings() {
        eprintln!("Refusing to publish a skill with critical validation warnings.");
        std::process::exit(1);
    }

    let output_dir = output.unwrap_or_else(|| prepared.source_dir.join("dist"));
    let packaged = librefang_skills::publish::package_prepared_skill(&prepared, &output_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to package skill: {e}");
            std::process::exit(1);
        });

    println!(
        "Bundle created: {}\n  SHA256: {}\n  Size: {} bytes",
        packaged.archive_path.display(),
        packaged.sha256,
        packaged.size_bytes
    );

    let repo = repo.unwrap_or_else(|| format!("librefang-skills/{}", packaged.manifest.skill.name));
    let tag = tag.unwrap_or_else(|| format!("v{}", packaged.manifest.skill.version));

    if dry_run {
        println!("Dry run only.");
        println!("  Repo: {repo}\n  Tag: {tag}");
        return;
    }

    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .unwrap_or_else(|_| {
            eprintln!("Set GITHUB_TOKEN or GH_TOKEN to publish, or re-run with --dry-run.");
            std::process::exit(1);
        });

    let release_notes = format!(
        "{}\n\nSHA256: `{}`\n\nInstall with:\n`librefang skill install {}`",
        packaged.manifest.skill.description, packaged.sha256, packaged.manifest.skill.name
    );
    let release_name = format!(
        "{} {}",
        packaged.manifest.skill.name, packaged.manifest.skill.version
    );

    let mut sp = progress::auto(
        &format!("Publishing {}@{tag}", packaged.manifest.skill.name),
        None,
    );
    sp.tick(1);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = librefang_skills::marketplace::MarketplaceClient::new(
        librefang_skills::marketplace::MarketplaceConfig::default(),
    );
    let published = rt
        .block_on(
            client.publish_bundle(librefang_skills::marketplace::MarketplacePublishRequest {
                repo: &repo,
                tag: &tag,
                bundle_path: &packaged.archive_path,
                release_name: &release_name,
                release_notes: &release_notes,
                token: &token,
            }),
        )
        .unwrap_or_else(|e| {
            sp.finish_with_failure(&format!("Publish failed: {e}"));
            std::process::exit(1);
        });

    sp.finish(&format!(
        "Published {} to {}@{}",
        published.asset_name, published.repo, published.tag
    ));
    if !published.html_url.is_empty() {
        println!("Release: {}", published.html_url);
    }
}

fn resolve_skill_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|e| {
            eprintln!("Could not determine current directory: {e}");
            std::process::exit(1);
        })
    })
}

fn print_skill_warnings(warnings: &[librefang_skills::verify::SkillWarning]) {
    if warnings.is_empty() {
        println!("  Warnings: none");
        return;
    }

    println!("  Warnings:");
    for warning in warnings {
        println!(
            "    [{}] {}",
            severity_label(warning.severity),
            warning.message
        );
    }
}

fn severity_label(severity: librefang_skills::verify::WarningSeverity) -> &'static str {
    match severity {
        librefang_skills::verify::WarningSeverity::Info => "info",
        librefang_skills::verify::WarningSeverity::Warning => "warn",
        librefang_skills::verify::WarningSeverity::Critical => "critical",
    }
}

fn cmd_skill_create() {
    let name = prompt_input("Skill name: ");
    let description = prompt_input("Description: ");
    let runtime = prompt_input("Runtime (python/node/wasm) [python]: ");
    let runtime = if runtime.is_empty() {
        "python".to_string()
    } else {
        runtime
    };

    let home = librefang_home();
    let skill_dir = home.join("skills").join(&name);
    std::fs::create_dir_all(skill_dir.join("src")).unwrap_or_else(|e| {
        eprintln!("Error creating skill directory: {e}");
        std::process::exit(1);
    });

    let tool_name = name.replace('-', "_");

    // A Cargo package name must be `[A-Za-z0-9_-]+` and not start with a digit;
    // a skill name can be anything the user typed. Derive a legal package name
    // for the WASM scaffold's Cargo.toml. The artifact name is fixed to
    // `skill` via `[lib] name`, so this only needs to be valid, not meaningful.
    let pkg_name = {
        let cleaned: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let cleaned = cleaned.trim_matches('-');
        if cleaned.is_empty() {
            "skill".to_string()
        } else if cleaned.starts_with(|c: char| c.is_ascii_digit()) {
            format!("skill-{cleaned}")
        } else {
            cleaned.to_string()
        }
    };

    // Per-runtime scaffold: the manifest `entry` path, the files to write
    // (relative to the skill dir), and any extra build steps the author must
    // run before the entry exists.
    struct Scaffold {
        entry: String,
        files: Vec<(String, String)>,
        build_steps: Vec<String>,
    }

    let scaffold = match runtime.as_str() {
        "python" => Scaffold {
            entry: "src/main.py".to_string(),
            files: vec![(
                "src/main.py".to_string(),
                format!(
                    r#"#!/usr/bin/env python3
"""LibreFang skill: {name}"""
import json
import sys

def main():
    payload = json.loads(sys.stdin.read())
    tool_name = payload["tool"]
    input_data = payload["input"]

    # TODO: Implement your skill logic here
    result = {{"result": f"Processed: {{input_data.get('input', '')}}"}}

    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#
                ),
            )],
            build_steps: vec![],
        },
        "node" => Scaffold {
            entry: "src/index.js".to_string(),
            files: vec![(
                "src/index.js".to_string(),
                format!(
                    r#"// LibreFang skill: {name}
const chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {{
  const payload = JSON.parse(Buffer.concat(chunks).toString());
  const input = payload.input || {{}};
  // TODO: Implement your skill logic here
  const result = {{ result: `Processed: ${{input.input ?? ""}}` }};
  process.stdout.write(JSON.stringify(result));
}});
"#
                ),
            )],
            build_steps: vec![],
        },
        "wasm" => Scaffold {
            // Entry is the artifact at the skill root, NOT under target/: the
            // packager (`should_include_entry`) excludes `target/`, so a skill
            // referencing the build dir would publish without its binary. The
            // build step copies the compiled module to the root.
            entry: "skill.wasm".to_string(),
            files: vec![
                (
                    "Cargo.toml".to_string(),
                    format!(
                        r#"[package]
name = "{pkg_name}"
version = "0.1.0"
edition = "2021"

[lib]
# Fixed name so the artifact is always `skill.wasm` regardless of package name.
name = "skill"
crate-type = ["cdylib"]

[dependencies]
librefang-skill = "0.1"
serde_json = "1"

[profile.release]
panic = "abort"
"#
                    ),
                ),
                (
                    "src/lib.rs".to_string(),
                    format!(
                        r#"//! LibreFang skill: {name}
use librefang_skill::{{skill, Request}};
use serde_json::{{json, Value}};

fn handle(req: Request) -> Result<Value, String> {{
    match req.tool.as_str() {{
        "{tool_name}" => {{
            // TODO: Implement your skill logic here.
            let input = req.input.get("input").and_then(Value::as_str).unwrap_or("");
            Ok(json!({{ "result": format!("Processed: {{input}}") }}))
        }}
        other => Err(format!("unknown tool: {{other}}")),
    }}
}}

skill!(handle);
"#
                    ),
                ),
            ],
            build_steps: vec![
                "rustup target add wasm32-unknown-unknown".to_string(),
                "cargo build --release --target wasm32-unknown-unknown".to_string(),
                "cp target/wasm32-unknown-unknown/release/skill.wasm skill.wasm".to_string(),
            ],
        },
        other => {
            eprintln!("Unsupported runtime '{other}'. Choose one of: python, node, wasm.");
            std::process::exit(1);
        }
    };

    let manifest = format!(
        r#"[skill]
name = "{name}"
version = "{version}"
description = "{description}"
author = ""
license = "MIT"
tags = []

[runtime]
type = "{runtime}"
entry = "{entry}"

[[tools.provided]]
name = "{tool_name}"
description = "{description}"
input_schema = {{ type = "object", properties = {{ input = {{ type = "string" }} }}, required = ["input"] }}

[requirements]
tools = []
capabilities = []
"#,
        version = librefang_types::VERSION,
        entry = scaffold.entry,
    );

    std::fs::write(skill_dir.join("skill.toml"), &manifest).unwrap();
    for (rel, content) in &scaffold.files {
        let path = skill_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }

    println!("\nSkill created: {}", skill_dir.display());
    println!("\nFiles:");
    println!("  skill.toml");
    for (rel, _) in &scaffold.files {
        println!("  {rel}");
    }
    println!("\nNext steps:");
    let mut step = 1;
    println!("  {step}. Edit the entry point to implement your skill logic");
    for build_step in &scaffold.build_steps {
        step += 1;
        println!("  {step}. {build_step}");
    }
    step += 1;
    println!(
        "  {step}. Test locally: librefang skill test {}",
        skill_dir.display()
    );
    step += 1;
    println!(
        "  {step}. Install: librefang skill install {}",
        skill_dir.display()
    );
}

/// Print an EvolutionResult as a one-line status.
fn print_evolution_result(result: &librefang_skills::evolution::EvolutionResult) {
    let marker = if result.success { "OK" } else { "FAIL" };
    match &result.version {
        Some(v) => println!("[{marker}] {} (v{v})", result.message),
        None => println!("[{marker}] {}", result.message),
    }
}

/// Resolve a skill by name. Respects `--hand` so evolve operations can
/// target a per-hand workspace skills dir just like `install`/`list`.
fn load_installed_skill(
    name: &str,
    hand: Option<&str>,
) -> (PathBuf, librefang_skills::InstalledSkill) {
    let skills_dir = resolve_skills_dir(hand);
    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir.clone());
    if let Err(e) = registry.load_all() {
        eprintln!("Error loading skill registry: {e}");
        std::process::exit(1);
    }
    match registry.get(name) {
        Some(skill) => (skills_dir, skill.clone()),
        None => {
            eprintln!("Skill '{name}' not found in {}", skills_dir.display());
            std::process::exit(1);
        }
    }
}

fn cmd_skill_evolve(sub: EvolveCommands) {
    match sub {
        EvolveCommands::Create {
            name,
            description,
            context_file,
            tags,
            hand,
        } => {
            let prompt_context = match read_file_or_stdin(&context_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", context_file.display());
                    std::process::exit(1);
                }
            };
            let tag_list: Vec<String> = tags
                .split(',')
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect();
            let skills_dir = resolve_skills_dir(hand.as_deref());
            if let Err(e) = std::fs::create_dir_all(&skills_dir) {
                eprintln!("Failed to create skills dir: {e}");
                std::process::exit(1);
            }
            match librefang_skills::evolution::create_skill(
                &skills_dir,
                &name,
                &description,
                &prompt_context,
                tag_list,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Create failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Update {
            name,
            context_file,
            changelog,
            hand,
        } => {
            let new_ctx = match read_file_or_stdin(&context_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", context_file.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::update_skill(
                &skill,
                &new_ctx,
                &changelog,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Update failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Patch {
            name,
            old_file,
            new_file,
            changelog,
            replace_all,
            hand,
        } => {
            let old_str = match read_file_or_stdin(&old_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", old_file.display());
                    std::process::exit(1);
                }
            };
            let new_str = match read_file_or_stdin(&new_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", new_file.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::patch_skill(
                &skill,
                &old_str,
                &new_str,
                &changelog,
                replace_all,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Patch failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Delete { name, hand } => {
            let skills_dir = resolve_skills_dir(hand.as_deref());
            match librefang_skills::evolution::delete_skill(&skills_dir, &name) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Delete failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Rollback { name, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::rollback_skill(&skill, Some("cli")) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Rollback failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::WriteFile {
            name,
            path,
            source,
            hand,
        } => {
            let content = match read_file_or_stdin(&source) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", source.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::write_supporting_file(&skill, &path, &content) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Write-file failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::RemoveFile { name, path, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::remove_supporting_file(&skill, &path) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Remove-file failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::History { name, json, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            let meta = librefang_skills::evolution::get_evolution_info(&skill);
            if json {
                match serde_json::to_string_pretty(&meta) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("Failed to serialize history: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            println!("Skill: {}", skill.manifest.skill.name);
            println!("Current version: {}", skill.manifest.skill.version);
            println!("Use count: {}", meta.use_count);
            println!("Evolution count: {}", meta.evolution_count);
            if meta.versions.is_empty() {
                println!("\nNo version history recorded.");
                return;
            }
            println!();
            let mut t = crate::table::Table::new(&["VERSION", "TIMESTAMP", "CHANGELOG"]);
            for v in meta.versions.iter().rev() {
                t.add_row(&[&v.version, &v.timestamp, &v.changelog]);
            }
            t.print();
        }
    }
}

// ---------------------------------------------------------------------------
// Skill workshop pending review (#3328)
// ---------------------------------------------------------------------------

fn cmd_skill_pending(sub: PendingCommands) {
    let skills_root = librefang_home().join("skills");
    match sub {
        PendingCommands::List { agent } => {
            let candidates = match &agent {
                Some(a) => librefang_kernel::skill_workshop::storage::list_pending(&skills_root, a),
                None => librefang_kernel::skill_workshop::storage::list_pending_all(&skills_root),
            };
            let candidates = match candidates {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Failed to read pending directory: {e}");
                    std::process::exit(1);
                }
            };
            if candidates.is_empty() {
                println!(
                    "No pending skill candidates.{}",
                    match &agent {
                        Some(a) => format!(" (filter: agent {a})"),
                        None => String::new(),
                    }
                );
                return;
            }
            println!("{:<38}  {:<18}  {:<22}  NAME", "ID", "SOURCE", "CAPTURED");
            for c in candidates {
                let source_label = match &c.source {
                    librefang_kernel::skill_workshop::CaptureSource::ExplicitInstruction {
                        ..
                    } => "explicit_instr",
                    librefang_kernel::skill_workshop::CaptureSource::UserCorrection { .. } => {
                        "user_correction"
                    }
                    librefang_kernel::skill_workshop::CaptureSource::RepeatedToolPattern {
                        ..
                    } => "tool_pattern",
                };
                println!(
                    "{:<38}  {:<18}  {:<22}  {}",
                    c.id,
                    source_label,
                    c.captured_at.format("%Y-%m-%d %H:%M:%S UTC"),
                    c.name
                );
            }
        }
        PendingCommands::Show { id } => {
            let candidate = match librefang_kernel::skill_workshop::storage::load_candidate(
                &skills_root,
                &id,
            ) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to load candidate: {e}");
                    std::process::exit(1);
                }
            };
            let toml_str = match toml::to_string_pretty(&candidate) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to render candidate as TOML: {e}");
                    std::process::exit(1);
                }
            };
            print!("{toml_str}");
        }
        PendingCommands::Approve { id } => {
            match librefang_kernel::skill_workshop::storage::approve_candidate(
                &skills_root,
                &skills_root,
                &id,
            ) {
                Ok(result) => {
                    println!(
                        "Approved candidate {} → installed skill '{}' (v{}).",
                        id,
                        result.skill_name,
                        result.version.unwrap_or_else(|| "?".to_string())
                    );
                }
                Err(e) => {
                    eprintln!("Approve failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        PendingCommands::Reject { id } => {
            match librefang_kernel::skill_workshop::storage::reject_candidate(&skills_root, &id) {
                Ok(()) => println!("Rejected and removed candidate {id}."),
                Err(e) => {
                    eprintln!("Reject failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Channel commands
// ---------------------------------------------------------------------------

// maybe_write_channel_config / notify_daemon_restart removed — they
// supported the interactive in-process channel onboarding flow whose
// callers were dropped when channels moved to sidecars, leaving both
// helpers orphaned.

// ---------------------------------------------------------------------------
// Hand commands
// ---------------------------------------------------------------------------

fn cmd_hand_install(path: &str) {
    let base = require_daemon("hand install");
    let dir = std::path::Path::new(path);
    let toml_path = dir.join("HAND.toml");
    let skill_path = dir.join("SKILL.md");

    if !toml_path.exists() {
        eprintln!(
            "Error: No HAND.toml found in {}",
            dir.canonicalize()
                .unwrap_or_else(|_| dir.to_path_buf())
                .display()
        );
        std::process::exit(1);
    }

    let toml_content = std::fs::read_to_string(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {e}", toml_path.display());
        std::process::exit(1);
    });
    let skill_content = std::fs::read_to_string(&skill_path).unwrap_or_default();

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/install"))
            .json(&serde_json::json!({
                "toml_content": toml_content,
                "skill_content": skill_content,
            }))
            .send(),
    );

    if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }

    println!(
        "Installed hand: {} ({})",
        body["name"].as_str().unwrap_or("?"),
        body["id"].as_str().unwrap_or("?"),
    );
    println!(
        "Use `librefang hand activate {}` to start it.",
        body["id"].as_str().unwrap_or("?")
    );
}

// ---------------------------------------------------------------------------
// Channel commands (sidecar-aware). Replace the pre-#5463 in-process
// wizards: every channel now runs out-of-process, configuration goes
// through the surviving daemon endpoints (GET /api/channels for the
// list, GET /api/channels/registry + POST /api/channels/sidecar/{name}/
// configure for setup, POST /api/channels/reload to apply, plus a local
// `rm` that strips a [[sidecar_channels]] entry from config.toml).
// ---------------------------------------------------------------------------

fn cmd_channel_list() {
    let base = require_daemon("channel list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/channels")).send());
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        println!("No channels configured.");
        println!("Use `librefang channel setup` to add one.");
        return;
    }
    let mut t = crate::table::Table::new(&["NAME", "KIND", "CONFIGURED", "TOKEN", "24H MSGS"]);
    for ch in &items {
        let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let kind = ch.get("category").and_then(|v| v.as_str()).unwrap_or("?");
        let configured = ch
            .get("configured")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let has_token = ch
            .get("has_token")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let msgs = ch.get("msgs_24h").and_then(|v| v.as_u64()).unwrap_or(0);
        t.add_row(&[
            name,
            kind,
            if configured { "yes" } else { "no" },
            if has_token { "yes" } else { "no" },
            &msgs.to_string(),
        ]);
    }
    t.print();
}

fn cmd_channel_reload() {
    let base = require_daemon("channel reload");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/channels/reload"))
            .json(&serde_json::json!({}))
            .send(),
    );
    let started = body
        .get("started")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    println!("Channels reloaded ({started} sidecar(s) started).");
}

fn cmd_channel_setup(name: Option<&str>) {
    let base = require_daemon("channel setup");
    let client = daemon_client();
    // `GET /api/channels` carries the full sidecar describe schema for
    // every discoverable adapter on `fields[]`, so we don't need a
    // separate /registry call for the picker — same list does both
    // jobs.
    let body = daemon_json(client.get(format!("{base}/api/channels")).send());
    let all: Vec<serde_json::Value> = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Resolve the target row: explicit `<NAME>` argument, or interactive
    // picker over unconfigured rows.
    let target = match name {
        Some(n) => all
            .iter()
            .find(|c| c.get("name").and_then(|v| v.as_str()) == Some(n))
            .cloned(),
        None => {
            // Distinguish the two empty-picker cases so the operator
            // knows which is which:
            //  - `all.is_empty()`: daemon's `GET /api/channels` returned
            //    nothing at all — both `sidecar_channel_rows` and
            //    `sidecar_discovery_rows` are empty. That means there
            //    are no `[[sidecar_channels]]` entries AND nothing in
            //    the SIDECAR_CATALOG (the latter is normally only
            //    empty if the SDK wasn't installed alongside the
            //    daemon — fix is `pip install librefang-sdk`).
            //  - all non-empty but `candidates.is_empty()`: the
            //    operator has configured every adapter the catalog
            //    knows about. Use `librefang channel list` to see /
            //    `librefang channel rm <name>` to drop one.
            if all.is_empty() {
                println!("Daemon's channel registry is empty.");
                println!("Install the sidecar SDK so adapters appear in the catalog:");
                println!("  pip install librefang-sdk");
                println!("Then re-run `librefang channel setup`.");
                return;
            }
            let candidates: Vec<&serde_json::Value> = all
                .iter()
                .filter(|c| c.get("configured").and_then(|v| v.as_bool()) != Some(true))
                .collect();
            if candidates.is_empty() {
                println!("Every available channel is already configured.");
                println!("Use `librefang channel list` to see them, or");
                println!("`librefang channel rm <name>` to remove an entry first.");
                return;
            }
            println!("Pick a channel to set up:");
            for (i, ch) in candidates.iter().enumerate() {
                let n = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let d = ch.get("display_name").and_then(|v| v.as_str()).unwrap_or(n);
                println!("  {:>2}. {:<14} {}", i + 1, n, d);
            }
            let choice = prompt_input("Choice [1]: ");
            let idx = if choice.trim().is_empty() {
                0
            } else {
                choice
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(candidates.len() - 1)
            };
            Some(candidates[idx].clone())
        }
    };
    let target = match target {
        Some(t) => t,
        None => {
            ui::error_with_fix(
                &format!("Unknown channel: {}", name.unwrap_or("?")),
                "Run `librefang channel list` to see the available adapters.",
            );
            std::process::exit(1);
        }
    };
    let chan_name = target
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let fields: Vec<serde_json::Value> = target
        .get("fields")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if fields.is_empty() {
        println!("`{chan_name}` exposes no configurable fields — nothing to prompt for.");
        println!("(Hot-reload anyway with `librefang channel reload` if you've already edited config.toml by hand.)");
        return;
    }

    let mut values = serde_json::Map::new();
    for f in &fields {
        let key = f.get("key").and_then(|v| v.as_str()).unwrap_or_default();
        if key.is_empty() {
            continue;
        }
        let label = f.get("label").and_then(|v| v.as_str()).unwrap_or(key);
        let required = f.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
        let ftype = f.get("type").and_then(|v| v.as_str()).unwrap_or("text");
        let has_value = f
            .get("has_value")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let current = f.get("value").and_then(|v| v.as_str()).unwrap_or("");

        // Secret-typed + has_value=true: blank means "keep existing".
        // Non-secret + has current value: show as default-in-brackets.
        let prompt = if ftype == "secret" && has_value {
            format!("  {label} ({key}) [set — leave blank to keep]: ")
        } else if !current.is_empty() {
            format!("  {label} ({key}) [{current}]: ")
        } else if required {
            format!("  {label} ({key}) *: ")
        } else {
            format!("  {label} ({key}): ")
        };
        let entered = prompt_input(&prompt);
        let val = entered.trim();
        if val.is_empty() {
            continue;
        }
        values.insert(key.to_string(), serde_json::Value::String(val.to_string()));
    }

    // Sidecar names come from `SIDECAR_CATALOG` keys — short
    // alphanumeric (`telegram`, `ntfy`, …), URL-safe as-is. No need
    // for percent-encoding.
    let url = format!("{base}/api/channels/sidecar/{chan_name}/configure");
    let payload = serde_json::json!({ "values": values });
    let body = daemon_json(client.post(&url).json(&payload).send());
    // `daemon_json` only logs 5xx; 4xx silently returns the error body.
    // Surface those by checking for the SidecarSaveResult shape. The
    // `ApiErrorResponse` envelope (see librefang-api types.rs:114-164)
    // serializes the human-readable message at both `error.message`
    // (nested, #3639 preferred shape) and `message` (top-level flat
    // alias kept for legacy callers); prefer the nested one, fall
    // through to the flat alias for older deployments.
    if body.get("status").and_then(|v| v.as_str()) != Some("saved") {
        let err = body
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .or_else(|| body.get("message").and_then(|v| v.as_str()))
            .unwrap_or("save failed (no error body)");
        ui::error_with_fix(
            &format!("Save for `{chan_name}` rejected: {err}"),
            "Re-run with corrected values, or check the daemon log for details.",
        );
        std::process::exit(1);
    }
    let restart_required = body
        .get("restart_required")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let shadowed = body
        .get("shadowed_secrets")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if restart_required {
        println!("✓ Saved `{chan_name}` — restart the daemon for changes to apply.");
    } else {
        println!("✓ Saved `{chan_name}` — hot-reload applied.");
    }
    if !shadowed.is_empty() {
        let keys: Vec<String> = shadowed
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        eprintln!(
            "Warning: shell environment variables shadow these tokens — unset them and restart for the new value to take effect: {}",
            keys.join(", "),
        );
    }
}

fn cmd_channel_rm(name: &str) {
    // Strip the matching `[[sidecar_channels]]` entry from
    // ~/.librefang/config.toml in-place, then trigger a daemon reload
    // (best-effort: if no daemon is running, the file edit is enough
    // — the next daemon start will pick up the changed config).
    let home = cli_librefang_home();
    let path = home.join("config.toml");
    let original = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            ui::error_with_fix(
                &format!("Cannot read {}: {e}", path.display()),
                "Run `librefang init` to create the config file.",
            );
            std::process::exit(1);
        }
    };
    let mut doc: toml_edit::DocumentMut = match original.parse() {
        Ok(d) => d,
        Err(e) => {
            ui::error_with_fix(
                &format!("Cannot parse {}: {e}", path.display()),
                "Fix the TOML syntax and retry.",
            );
            std::process::exit(1);
        }
    };
    let arr = match doc
        .get_mut("sidecar_channels")
        .and_then(|v| v.as_array_of_tables_mut())
    {
        Some(a) => a,
        None => {
            println!("No [[sidecar_channels]] entries in config.toml — nothing to remove.");
            return;
        }
    };
    // `toml_edit::ArrayOfTables` has no `retain`; collect matching indices
    // then remove in reverse so earlier indices stay stable.
    let to_remove: Vec<usize> = arr
        .iter()
        .enumerate()
        .filter_map(|(i, t)| match t.get("name").and_then(|v| v.as_str()) {
            Some(n) if n == name => Some(i),
            _ => None,
        })
        .collect();
    let removed = to_remove.len();
    for &i in to_remove.iter().rev() {
        arr.remove(i);
    }
    if removed == 0 {
        println!("No [[sidecar_channels]] entry with name=\"{name}\".");
        return;
    }
    if let Err(e) = std::fs::write(&path, doc.to_string()) {
        ui::error_with_fix(
            &format!("Failed to write {}: {e}", path.display()),
            "Check filesystem permissions.",
        );
        std::process::exit(1);
    }
    println!("✓ Removed {removed} [[sidecar_channels]] entry/entries named `{name}`.");
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            match client
                .post(format!("{base}/api/channels/reload"))
                .json(&serde_json::json!({}))
                .send()
            {
                Ok(r) if r.status().is_success() => println!("  Hot-reloaded daemon."),
                Ok(r) => eprintln!(
                    "  Reload returned {}: change will apply on next daemon restart.",
                    r.status()
                ),
                Err(e) => eprintln!(
                    "  Could not contact daemon for reload ({e}); change will apply on next start."
                ),
            }
        }
        None => println!("  Daemon not running; change will apply on next start."),
    }
}

fn cmd_hand_list() {
    let base = require_daemon("hand list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands")).send());
    // API returns {"hands": [...]} or a bare array
    let arr_val;
    if let Some(arr) = body.get("hands").and_then(|v| v.as_array()) {
        arr_val = arr.clone();
    } else if let Some(arr) = body.as_array() {
        arr_val = arr.clone();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = Some(&arr_val) {
        if arr.is_empty() {
            println!("No hands available.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "NAME", "CATEGORY", "DESCRIPTION"]);
        for h in arr {
            t.add_row(&[
                h["id"].as_str().unwrap_or("?"),
                h["name"].as_str().unwrap_or("?"),
                h["category"].as_str().unwrap_or("?"),
                &h["description"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>(),
            ]);
        }
        t.print();
        println!("\nUse `librefang hand activate <id>` to activate a hand.");
    }
}

fn cmd_hand_active() {
    let base = require_daemon("hand active");
    let client = daemon_client();
    let arr = fetch_active_hand_instances(&base, &client);
    if arr.is_empty() {
        println!("No active hands.");
        return;
    }
    let mut t = crate::table::Table::new(&["INSTANCE", "HAND", "STATUS", "AGENT"]);
    for i in &arr {
        t.add_row(&[
            i["instance_id"].as_str().unwrap_or("?"),
            i["hand_id"].as_str().unwrap_or("?"),
            i["status"].as_str().unwrap_or("?"),
            i["agent_name"].as_str().unwrap_or("?"),
        ]);
    }
    t.print();
}

fn cmd_hand_status(id: Option<&str>) {
    if id.is_none() {
        cmd_hand_active();
        return;
    }

    let id = id.unwrap_or_default();
    let base = require_daemon("hand status");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);

    if let Some(instance) = resolve_hand_instance(&active, id) {
        let hand_id = instance["hand_id"].as_str().unwrap_or(id);
        let hand_body = daemon_json(client.get(format!("{base}/api/hands/{hand_id}")).send());
        let name = hand_body["name"].as_str().unwrap_or(hand_id);
        let status = instance["status"].as_str().unwrap_or("unknown");
        let instance_id = instance["instance_id"].as_str().unwrap_or("?");
        let agent_name = instance["agent_name"].as_str().unwrap_or("?");

        ui::section("Hand Status");
        ui::kv("Hand", hand_id);
        ui::kv("Name", name);
        ui::kv("Instance", instance_id);
        ui::kv("Status", status);
        ui::kv("Agent", agent_name);
        return;
    }

    let hand_body = daemon_json(client.get(format!("{base}/api/hands/{id}")).send());
    if hand_body.get("error").is_some() {
        ui::error(&format!(
            "No active hand or installed hand found for '{id}'."
        ));
        std::process::exit(1);
    }

    ui::section("Hand Status");
    ui::kv("Hand", hand_body["id"].as_str().unwrap_or(id));
    ui::kv("Name", hand_body["name"].as_str().unwrap_or(id));
    ui::kv("Status", "inactive");
    if let Some(description) = hand_body["description"].as_str() {
        if !description.is_empty() {
            ui::kv("Description", description);
        }
    }
}

fn cmd_hand_activate(id: &str) {
    let base = require_daemon("hand activate");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/activate"))
            .header("content-type", "application/json")
            .body("{}")
            .send(),
    );
    if body.get("instance_id").is_some() {
        println!(
            "Hand '{}' activated (instance: {}, agent: {})",
            id,
            body["instance_id"].as_str().unwrap_or("?"),
            body["agent_name"].as_str().unwrap_or("?"),
        );
    } else {
        eprintln!(
            "Failed to activate hand '{}': {}",
            id,
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_hand_deactivate(id: &str) {
    let base = require_daemon("hand deactivate");
    let client = daemon_client();
    // First find the instance ID for this hand
    let arr = fetch_active_hand_instances(&base, &client);
    let instance_id = arr.iter().find_map(|i| {
        if i["hand_id"].as_str() == Some(id) {
            i["instance_id"].as_str().map(|s| s.to_string())
        } else {
            None
        }
    });

    match instance_id {
        Some(iid) => {
            let body = daemon_json(
                client
                    .delete(format!("{base}/api/hands/instances/{iid}"))
                    .send(),
            );
            if body.get("status").is_some() {
                println!("Hand '{id}' deactivated.");
            } else {
                eprintln!(
                    "Failed: {}",
                    body["error"].as_str().unwrap_or("Unknown error")
                );
                std::process::exit(1);
            }
        }
        None => {
            eprintln!("No active instance found for hand '{id}'.");
            std::process::exit(1);
        }
    }
}

fn cmd_hand_info(id: &str) {
    let base = require_daemon("hand info");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands/{id}")).send());
    if body.get("error").is_some() {
        eprintln!("Hand not found: {}", body["error"].as_str().unwrap_or(id));
        std::process::exit(1);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
}

fn cmd_hand_check_deps(id: &str) {
    let base = require_daemon("hand check-deps");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/check-deps"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_hand_install_deps(id: &str) {
    let base = require_daemon("hand install-deps");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/install-deps"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&i18n::t_args("hand-install-deps-success", &[("id", id)]));
        if let Some(results) = body.get("results") {
            println!(
                "{}",
                serde_json::to_string_pretty(results).unwrap_or_default()
            );
        }
    }
}

fn cmd_hand_pause(id: &str) {
    let base = require_daemon("hand pause");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = resolve_hand_instance(&active, id);
    let instance_id = resolved
        .as_ref()
        .and_then(|instance| instance["instance_id"].as_str())
        .unwrap_or(id);
    let hand_label = resolved
        .as_ref()
        .and_then(|instance| instance["hand_id"].as_str())
        .unwrap_or(id);
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/instances/{instance_id}/pause"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    } else {
        ui::success(&i18n::t_args(
            "hand-paused",
            &[("id", &format!("{hand_label} (instance: {instance_id})"))],
        ));
    }
}

fn cmd_hand_resume(id: &str) {
    let base = require_daemon("hand resume");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = resolve_hand_instance(&active, id);
    let instance_id = resolved
        .as_ref()
        .and_then(|instance| instance["instance_id"].as_str())
        .unwrap_or(id);
    let hand_label = resolved
        .as_ref()
        .and_then(|instance| instance["hand_id"].as_str())
        .unwrap_or(id);
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/instances/{instance_id}/resume"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    } else {
        ui::success(&i18n::t_args(
            "hand-resumed",
            &[("id", &format!("{hand_label} (instance: {instance_id})"))],
        ));
    }
}

fn cmd_hand_settings(id: &str) {
    let base = require_daemon("hand settings");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands/{id}/settings")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    }
    if let Some(config) = body.get("config").and_then(|c| c.as_object()) {
        if config.is_empty() {
            ui::step(&format!("Hand '{id}' has no configurable settings."));
        } else {
            ui::section(&format!("Settings for '{id}'"));
            for (k, v) in config {
                println!("  {}: {}", k.bold(), v);
            }
        }
    } else {
        ui::step(&format!("Hand '{id}' has no configurable settings."));
    }
}

fn cmd_hand_set(id: &str, key: &str, value: &str) {
    let base = require_daemon("hand set");
    let client = daemon_client();
    let mut config = serde_json::Map::new();
    config.insert(
        key.to_string(),
        serde_json::Value::String(value.to_string()),
    );
    let body = daemon_json(
        client
            .put(format!("{base}/api/hands/{id}/settings"))
            .json(&serde_json::json!({ "config": config }))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    }
    ui::success(&format!("Set {key}={value} for hand '{id}'."));
}

fn cmd_hand_reload() {
    let base = require_daemon("hand reload");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/hands/reload")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    }
    let added = body["added"].as_u64().unwrap_or(0);
    let updated = body["updated"].as_u64().unwrap_or(0);
    let total = body["total"].as_u64().unwrap_or(0);
    ui::success(&format!(
        "Reloaded hands: {added} added, {updated} updated, {total} total."
    ));
}

fn cmd_hand_chat(id: &str) {
    let base = require_daemon("hand chat");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = match resolve_hand_instance(&active, id) {
        Some(instance) => instance,
        None => {
            ui::error(&format!("No active hand instance found for '{id}'."));
            ui::hint("Activate it first: librefang hand activate");
            std::process::exit(1);
        }
    };
    let instance_id = resolved["instance_id"]
        .as_str()
        .expect("instance_id missing");
    let hand_id = resolved["hand_id"].as_str().unwrap_or(id);
    let hand_name = resolved["hand_name"]
        .as_str()
        .or_else(|| resolved["name"].as_str())
        .unwrap_or(hand_id);

    install_ctrlc_handler();

    println!(
        "{} {} {}",
        "Chat with".bold(),
        hand_name.cyan().bold(),
        "(type /quit to exit)".dimmed()
    );
    println!();

    loop {
        print!("{} ", "you >".green().bold());
        io::stdout().flush().unwrap();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF
        }
        let msg = line.trim();
        if msg.is_empty() {
            continue;
        }
        if msg == "/quit" || msg == "/exit" || msg == "/q" {
            break;
        }

        let resp = client
            .post(format!("{base}/api/hands/instances/{instance_id}/message"))
            .json(&serde_json::json!({"message": msg}))
            .send();

        let body = daemon_json(resp);
        if let Some(err) = body["error"].as_str() {
            ui::error(err);
            continue;
        }
        let reply = body["response"]
            .as_str()
            .or_else(|| body["reply"].as_str())
            .unwrap_or("[no response]");
        println!("{} {}\n", format!("{hand_name} >").cyan().bold(), reply);
    }
}

fn fetch_active_hand_instances(
    base: &str,
    client: &reqwest::blocking::Client,
) -> Vec<serde_json::Value> {
    let body = daemon_json(client.get(format!("{base}/api/hands/active")).send());
    body.get("instances")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default()
}

fn resolve_hand_instance(
    active_instances: &[serde_json::Value],
    id_or_hand: &str,
) -> Option<serde_json::Value> {
    active_instances
        .iter()
        .find(|instance| {
            instance["instance_id"].as_str() == Some(id_or_hand)
                || instance["hand_id"].as_str() == Some(id_or_hand)
        })
        .cloned()
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

fn cmd_config_show() {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!("No configuration found at: {}", config_path.display());
        println!("Run `librefang init` to create one.");
        return;
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        eprintln!("Error reading config: {e}");
        std::process::exit(1);
    });

    println!("# {}\n", config_path.display());
    println!("{content}");
}

fn cmd_config_edit() {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let status = std::process::Command::new(&editor)
        .arg(&config_path)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("Editor exited with: {s}");
        }
        Err(e) => {
            eprintln!("Failed to open editor '{editor}': {e}");
            eprintln!("Set $EDITOR to your preferred editor.");
        }
    }
}

fn cmd_config_get(key: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix"),
        );
        std::process::exit(1);
    });

    // Navigate dotted path
    let mut current = &table;
    for part in key.split('.') {
        match current.get(part) {
            Some(v) => current = v,
            None => {
                ui::error(&i18n::t_args("config-key-not-found", &[("key", key)]));
                std::process::exit(1);
            }
        }
    }

    // Print value
    match current {
        toml::Value::String(s) => println!("{s}"),
        toml::Value::Integer(i) => println!("{i}"),
        toml::Value::Float(f) => println!("{f}"),
        toml::Value::Boolean(b) => println!("{b}"),
        other => println!("{other}"),
    }
}

/// Parse a string as a TOML integer, rejecting values outside i64 range.
/// TOML integers are i64; we never silently truncate `u64 > i64::MAX` into
/// negative numbers (#3461).
fn parse_toml_integer(raw: &str) -> Result<toml::Value, String> {
    if let Ok(v) = raw.parse::<i64>() {
        return Ok(toml::Value::Integer(v));
    }
    if let Ok(v) = raw.parse::<u64>() {
        return match i64::try_from(v) {
            Ok(v) => Ok(toml::Value::Integer(v)),
            Err(_) => Err(format!(
                "value {v} exceeds i64::MAX ({}); TOML cannot store unsigned integers above this bound",
                i64::MAX
            )),
        };
    }
    Err(format!("'{raw}' is not a valid integer"))
}

fn cmd_config_set(key: &str, value: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix-alt"),
        );
        std::process::exit(1);
    });

    // Navigate to parent and set key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error(&i18n::t("config-empty-key"));
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&i18n::t_args("config-key-path-not-found", &[("key", key)]));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];

    // Validate: single-part keys must be known scalar fields, not sections.
    // Writing a section name as a scalar silently breaks config deserialization.
    if parts.len() == 1 {
        let known_scalars = [
            "home_dir",
            "data_dir",
            "log_level",
            "api_listen",
            "network_enabled",
            "api_key",
            "language",
            "max_cron_jobs",
            "usage_footer",
            "workspaces_dir",
        ];
        if !known_scalars.contains(&last_key) {
            ui::error_with_fix(
                &i18n::t_args("config-section-not-scalar", &[("key", last_key)]),
                &i18n::t_args("config-section-not-scalar-fix", &[("key", last_key)]),
            );
            std::process::exit(1);
        }
    }

    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&i18n::t_args("config-parent-not-table", &[("key", key)]));
        std::process::exit(1);
    });

    // Try to preserve type: if the existing value is an integer, parse as int, etc.
    let new_value = if let Some(existing) = tbl.get(last_key) {
        match existing {
            toml::Value::Integer(_) => match parse_toml_integer(value) {
                Ok(v) => v,
                Err(msg) => {
                    ui::error(&msg);
                    std::process::exit(1);
                }
            },
            toml::Value::Float(_) => value
                .parse::<f64>()
                .map(toml::Value::Float)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Boolean(_) => value
                .parse::<bool>()
                .map(toml::Value::Boolean)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            _ => toml::Value::String(value.to_string()),
        }
    } else {
        // No existing value — infer type from the string content
        if let Ok(b) = value.parse::<bool>() {
            toml::Value::Boolean(b)
        } else if let Ok(v) = parse_toml_integer(value) {
            v
        } else if let Ok(f) = value.parse::<f64>() {
            toml::Value::Float(f)
        } else {
            toml::Value::String(value.to_string())
        }
    };

    tbl.insert(last_key.to_string(), new_value);

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-serialize-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-write-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&i18n::t_args(
        "config-set-kv",
        &[("key", key), ("value", value)],
    ));
}

fn cmd_config_unset(key: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix-alt"),
        );
        std::process::exit(1);
    });

    // Navigate to parent table and remove the final key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error(&i18n::t("config-empty-key"));
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&i18n::t_args("config-key-path-not-found", &[("key", key)]));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];
    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&i18n::t_args("config-parent-not-table", &[("key", key)]));
        std::process::exit(1);
    });

    if tbl.remove(last_key).is_none() {
        ui::error(&i18n::t_args("config-key-not-found", &[("key", key)]));
        std::process::exit(1);
    }

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-serialize-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-write-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&i18n::t_args("config-removed-key", &[("key", key)]));
}

fn cmd_config_set_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    let key = prompt_input(&format!("  Paste your {provider} API key: "));
    if key.is_empty() {
        ui::error(&i18n::t("config-no-key"));
        return;
    }

    match dotenv::save_env_key(&env_var, &key) {
        Ok(()) => {
            ui::success(&i18n::t_args("config-saved-key", &[("env_var", &env_var)]));
            // Test the key
            print!("  Testing key... ");
            io::stdout().flush().unwrap();
            if test_api_key(provider, &key) {
                println!("{}", "OK".bright_green());
            } else {
                println!("{}", "could not verify (may still work)".bright_yellow());
            }
        }
        Err(e) => {
            ui::error(&i18n::t_args(
                "config-save-key-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

fn cmd_config_delete_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    match dotenv::remove_env_key(&env_var) {
        Ok(()) => ui::success(&i18n::t_args(
            "config-removed-env",
            &[("env_var", &env_var)],
        )),
        Err(e) => {
            ui::error(&i18n::t_args(
                "config-remove-key-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

fn cmd_config_test_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    if std::env::var(&env_var).is_err() {
        ui::error(&i18n::t_args(
            "config-env-not-set",
            &[("env_var", &env_var)],
        ));
        ui::hint(&i18n::t_args(
            "config-set-key-hint",
            &[("provider", provider)],
        ));
        std::process::exit(1);
    }

    print!("  Testing {provider} ({env_var})... ");
    io::stdout().flush().unwrap();
    if test_api_key(provider, &std::env::var(&env_var).unwrap_or_default()) {
        println!("{}", "OK".bright_green());
    } else {
        println!("{}", "FAILED (401/403)".bright_red());
        ui::hint(&i18n::t_args(
            "config-update-key-hint",
            &[("provider", provider)],
        ));
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Quick chat (OpenClaw alias)
// ---------------------------------------------------------------------------

fn cmd_quick_chat(config: Option<PathBuf>, agent: Option<String>) {
    ensure_initialized(&config);
    tui::chat_runner::run_chat_tui(config, agent);
}

// ---------------------------------------------------------------------------
// MCP server commands (librefang mcp {add,remove,list,catalog})
// ---------------------------------------------------------------------------

fn cmd_mcp_add(name: &str, key: Option<&str>) {
    let home = librefang_home();
    let mut catalog = librefang_extensions::catalog::McpCatalog::new(&home);
    catalog.load(&home);

    // Check template exists
    let template = match catalog.get(name) {
        Some(t) => t.clone(),
        None => {
            ui::error(&format!("Unknown MCP catalog entry: '{name}'"));
            println!("\nAvailable MCP servers (catalog):");
            for t in catalog.list() {
                println!("  {} {} — {}", t.icon, t.id, t.description);
            }
            std::process::exit(1);
        }
    };

    // Reject re-install of an already-configured server by name/template_id.
    // The API path returns 409 here; the CLI was silently overwriting the
    // existing [[mcp_servers]] entry (including edited transport/env/oauth)
    // because upsert_mcp_server_local replaces by name. Users should remove
    // first if they want to re-install.
    let config_path = home.join("config.toml");
    if config_path.is_file() {
        let content = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                ui::error(&format!("Failed to read {}: {e}", config_path.display()));
                std::process::exit(1);
            }
        };
        let parsed: toml::value::Table = match toml::from_str(&content) {
            Ok(t) => t,
            Err(e) => {
                ui::error(&format!("{} is not valid TOML: {e}", config_path.display()));
                std::process::exit(1);
            }
        };
        if let Some(toml::Value::Array(servers)) = parsed.get("mcp_servers") {
            let conflict = servers.iter().any(|v| {
                let t = match v.as_table() {
                    Some(t) => t,
                    None => return false,
                };
                let matches_field = |k: &str| t.get(k).and_then(|n| n.as_str()) == Some(name);
                matches_field("name") || matches_field("template_id")
            });
            if conflict {
                ui::error(&format!(
                    "MCP server '{name}' is already configured. Run \
                     `librefang mcp remove {name}` first if you want to re-install."
                ));
                std::process::exit(1);
            }
        }
    }

    // Set up credential resolver (vault + dotenv + interactive prompt fallback)
    let dotenv_path = home.join(".env");
    let vault_path = home.join("vault.enc");
    let vault = if vault_path.exists() {
        let mut v = librefang_extensions::vault::CredentialVault::new(vault_path);
        if v.unlock().is_ok() {
            Some(v)
        } else {
            None
        }
    } else {
        None
    };
    let mut resolver =
        librefang_extensions::credentials::CredentialResolver::new(vault, Some(&dotenv_path))
            .with_interactive(true);

    // Build provided keys map
    let mut provided_keys = std::collections::HashMap::new();
    if let Some(key_value) = key {
        // Auto-detect which env var to use (first required_env that's a secret)
        if let Some(env_var) = template.required_env.iter().find(|e| e.is_secret) {
            provided_keys.insert(env_var.name.clone(), key_value.to_string());
        }
    }

    let result = match librefang_extensions::installer::install_integration(
        &catalog,
        &mut resolver,
        name,
        &provided_keys,
    ) {
        Ok(r) => r,
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    };

    // Persist the new [[mcp_servers]] entry directly into config.toml.
    let config_path = home.join("config.toml");
    if let Err(e) = upsert_mcp_server_local(&config_path, &result.server) {
        ui::error(&format!("Failed to write config.toml: {e}"));
        std::process::exit(1);
    }

    match &result.status {
        librefang_types::mcp::McpStatus::Ready => ui::success(&result.message),
        librefang_types::mcp::McpStatus::Setup => {
            println!("{}", result.message.yellow());
            println!("\nTo add credentials:");
            for env in &template.required_env {
                if env.is_secret {
                    println!("  librefang vault set {}  # {}", env.name, env.help);
                    if let Some(ref url) = env.get_url {
                        println!("  Get it here: {url}");
                    }
                }
            }
        }
        _ => println!("{}", result.message),
    }

    // If daemon is running, trigger hot-reload.
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let _ = client.post(format!("{base_url}/api/mcp/reload")).send();
    }
}

fn cmd_mcp_remove(name: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    // Resolve by template_id first, fall back to server name.
    let target_name: Option<String> = {
        let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
        let doc: toml::Value =
            toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));
        doc.as_table()
            .and_then(|t| t.get("mcp_servers"))
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|entry| {
                    let tbl = entry.as_table()?;
                    let tid = tbl.get("template_id").and_then(|v| v.as_str());
                    let nm = tbl.get("name").and_then(|v| v.as_str())?;
                    if tid == Some(name) || nm == name {
                        Some(nm.to_string())
                    } else {
                        None
                    }
                })
            })
    };

    let target_name = match target_name {
        Some(n) => n,
        None => {
            ui::error(&format!("MCP server '{name}' is not configured"));
            std::process::exit(1);
        }
    };

    if let Err(e) = remove_mcp_server_local(&config_path, &target_name) {
        ui::error(&format!("Failed to update config.toml: {e}"));
        std::process::exit(1);
    }

    ui::success(&format!("{target_name} removed."));

    // Hot-reload daemon
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let _ = client.post(format!("{base_url}/api/mcp/reload")).send();
    }
}

fn cmd_mcp_catalog(query: Option<&str>) {
    let home = librefang_home();
    let mut catalog = librefang_extensions::catalog::McpCatalog::new(&home);
    catalog.load(&home);

    // Installed state comes from config.mcp_servers' template_id field.
    let installed_template_ids: std::collections::HashSet<String> = {
        let raw = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
        toml::from_str::<toml::Value>(&raw)
            .ok()
            .and_then(|v| v.as_table().cloned())
            .and_then(|t| t.get("mcp_servers").cloned())
            .and_then(|v| v.as_array().cloned())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|v| {
                        v.as_table()
                            .and_then(|t| t.get("template_id"))
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let entries: Vec<_> = if let Some(q) = query {
        catalog.search(q).into_iter().cloned().collect()
    } else {
        catalog.list().into_iter().cloned().collect()
    };

    if entries.is_empty() {
        if let Some(q) = query {
            println!("No MCP catalog entries matching '{q}'.");
        } else {
            println!("No MCP catalog entries available.");
        }
        return;
    }

    // Group by category
    let mut by_category: std::collections::BTreeMap<
        String,
        Vec<&librefang_types::mcp::McpCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for entry in &entries {
        by_category
            .entry(entry.category.to_string())
            .or_default()
            .push(entry);
    }

    for (category, items) in &by_category {
        println!("\n{}", format!("  {category}").bold());
        for item in items {
            let status_badge = if installed_template_ids.contains(&item.id) {
                "[Installed]".green().to_string()
            } else {
                "[Available]".dimmed().to_string()
            };
            println!(
                "    {} {:<20} {:<13} {}",
                item.icon, item.id, status_badge, item.description
            );
        }
    }
    println!();
    println!(
        "  {} catalog entries ({} installed)",
        entries.len(),
        entries
            .iter()
            .filter(|e| installed_template_ids.contains(&e.id))
            .count()
    );
    println!("  Use `librefang mcp add <id>` to install an MCP server.");
}

fn cmd_mcp_list() {
    let home = librefang_home();
    let raw = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let doc: toml::Value = toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));
    let servers = doc
        .as_table()
        .and_then(|t| t.get("mcp_servers"))
        .and_then(|v| v.as_array());
    let Some(servers) = servers else {
        println!("No MCP servers configured.");
        return;
    };
    if servers.is_empty() {
        println!("No MCP servers configured.");
        return;
    }
    println!();
    println!(
        "  {:<28} {:<14} {:<18} details",
        "name", "template_id", "transport"
    );
    for entry in servers {
        let Some(tbl) = entry.as_table() else {
            continue;
        };
        let name = tbl.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let tid = tbl
            .get("template_id")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let (transport, detail) = match tbl.get("transport").and_then(|v| v.as_table()) {
            Some(t) => {
                let ttype = t.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                let detail = match ttype {
                    "stdio" => t
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    "sse" | "http" => t
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    _ => String::new(),
                };
                (ttype.to_string(), detail)
            }
            None => ("-".to_string(), String::new()),
        };
        println!("  {name:<28} {tid:<14} {transport:<18} {detail}");
    }
    println!();
    println!("  Use `librefang mcp catalog` to list installable entries.");
}

/// Local upsert helper — mirrors the API's `upsert_mcp_server_config`.
fn upsert_mcp_server_local(
    config_path: &std::path::Path,
    entry: &librefang_types::config::McpServerConfigEntry,
) -> Result<(), String> {
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        // Propagate parse errors instead of silently defaulting. A
        // malformed config.toml would otherwise be overwritten as a new
        // near-empty file, wiping unrelated sections the user may want
        // to fix by hand.
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        toml::value::Table::new()
    };

    let entry_json = serde_json::to_value(entry).map_err(|e| e.to_string())?;
    let entry_toml = json_to_toml_value_cli(&entry_json);

    let servers = table
        .entry("mcp_servers".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));

    if let toml::Value::Array(ref mut arr) = servers {
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != entry.name)
                .unwrap_or(true)
        });
        arr.push(entry_toml);
    }

    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

fn remove_mcp_server_local(config_path: &std::path::Path, name: &str) -> Result<(), String> {
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        return Ok(());
    };
    if let Some(toml::Value::Array(ref mut arr)) = table.get_mut("mcp_servers") {
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != name)
                .unwrap_or(true)
        });
    }
    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Auth commands (librefang auth chatgpt)
// ---------------------------------------------------------------------------

enum DeviceAuthNextStep {
    ContinueDevice(librefang_runtime::chatgpt_oauth::DeviceAuthPrompt),
    FallbackToBrowser(String),
}

fn resolve_device_auth_start(
    result: Result<
        librefang_runtime::chatgpt_oauth::DeviceAuthPrompt,
        librefang_runtime::chatgpt_oauth::DeviceAuthFlowError,
    >,
) -> Result<DeviceAuthNextStep, String> {
    match result {
        Ok(prompt) => Ok(DeviceAuthNextStep::ContinueDevice(prompt)),
        Err(librefang_runtime::chatgpt_oauth::DeviceAuthFlowError::BrowserFallback { message }) => {
            Ok(DeviceAuthNextStep::FallbackToBrowser(message))
        }
        Err(err) => Err(err.to_string()),
    }
}

async fn authenticate_chatgpt(
    device_auth: bool,
) -> Result<librefang_runtime::chatgpt_oauth::ChatGptAuthResult, String> {
    use librefang_runtime::chatgpt_oauth;

    if device_auth {
        match resolve_device_auth_start(chatgpt_oauth::start_device_auth_flow().await)? {
            DeviceAuthNextStep::ContinueDevice(prompt) => {
                println!("Device authentication requested.");
                println!(
                    "Open this URL in any browser:\n  {}\n",
                    chatgpt_oauth::DEVICE_AUTH_URL
                );
                println!("Enter this one-time code:\n  {}\n", prompt.user_code);
                println!("Do not share this code.");
                println!("Waiting for authorization...");
                return chatgpt_oauth::poll_device_auth_flow(&prompt).await;
            }
            DeviceAuthNextStep::FallbackToBrowser(message) => {
                println!("{message}");
                println!("\nSwitching to the standard browser login flow...\n");
            }
        }
    }

    let (auth_url, port, code_verifier, state) = chatgpt_oauth::start_oauth_flow().await?;

    println!("Opening browser for OpenAI authentication...");
    println!("If the browser does not open, visit:\n  {auth_url}\n");

    if let Err(e) = open::that(&auth_url) {
        eprintln!("Could not open browser automatically: {e}");
        eprintln!("Please open manually: {auth_url}");
    }

    let code = chatgpt_oauth::run_oauth_callback_server(port, &state).await?;
    chatgpt_oauth::exchange_code_for_tokens(&code, &code_verifier, port).await
}

async fn persist_chatgpt_auth(
    auth_result: librefang_runtime::chatgpt_oauth::ChatGptAuthResult,
) -> Result<(), String> {
    use librefang_runtime::chatgpt_oauth;

    let home = librefang_home();
    std::fs::create_dir_all(&home)
        .map_err(|e| format!("Failed to create LibreFang home directory: {e}"))?;

    let access_token = auth_result.access_token;
    let refresh_token = auth_result.refresh_token;
    let secrets_path = write_chatgpt_secrets(
        &home,
        access_token.as_str(),
        refresh_token.as_ref().map(|rt| rt.as_str()),
    )?;

    println!("\nChatGPT tokens saved to {}", secrets_path.display());

    println!("Detecting best available model...");
    let best_model = chatgpt_oauth::fetch_best_codex_model(&access_token).await;
    println!("Selected model: {best_model}");

    update_chatgpt_config(&home, &best_model)?;

    println!("config.toml updated: provider = \"chatgpt\", model = \"{best_model}\"");
    Ok(())
}

fn write_chatgpt_secrets(
    home: &std::path::Path,
    access_token: &str,
    refresh_token: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    let secrets_path = home.join("secrets.env");
    let mut env_vars: Vec<(String, String)> = vec![(
        "CHATGPT_SESSION_TOKEN".to_string(),
        access_token.to_string(),
    )];
    if let Some(rt) = refresh_token {
        env_vars.push(("CHATGPT_REFRESH_TOKEN".to_string(), rt.to_string()));
    }

    let existing = std::fs::read_to_string(&secrets_path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| {
            !l.starts_with("CHATGPT_SESSION_TOKEN=") && !l.starts_with("CHATGPT_REFRESH_TOKEN=")
        })
        .map(|l| l.to_string())
        .collect();

    for (key, val) in &env_vars {
        lines.push(format!("{key}={val}"));
    }

    let mut updated = lines.join("\n");
    if !updated.ends_with('\n') {
        updated.push('\n');
    }

    std::fs::write(&secrets_path, updated)
        .map_err(|e| format!("Failed to write secrets.env: {e}"))?;

    Ok(secrets_path)
}

fn update_chatgpt_config(home: &std::path::Path, best_model: &str) -> Result<(), String> {
    let config_path = home.join("config.toml");
    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if config_str.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        config_str
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| format!("Failed to parse config.toml: {e}"))?
    };

    let dm = doc
        .entry("default_model")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or("default_model is not a table")?;
    dm.insert("provider", toml_edit::value("chatgpt"));
    dm.insert("api_key_env", toml_edit::value("CHATGPT_SESSION_TOKEN"));
    dm.insert("model", toml_edit::value(best_model));
    dm.insert(
        "base_url",
        toml_edit::value(librefang_runtime::chatgpt_oauth::CHATGPT_BASE_URL),
    );

    std::fs::write(&config_path, doc.to_string())
        .map_err(|e| format!("Failed to write config.toml: {e}"))?;

    Ok(())
}

fn cmd_auth_chatgpt(device_auth: bool) {
    println!("Starting ChatGPT authentication flow...\n");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let result: Result<(), String> = rt.block_on(async {
        let auth_result = authenticate_chatgpt(device_auth).await?;
        persist_chatgpt_auth(auth_result).await
    });

    match result {
        Ok(()) => ui::success("ChatGPT authentication complete."),
        Err(e) => {
            ui::error(&format!("ChatGPT authentication failed: {e}"));
            std::process::exit(1);
        }
    }
}

// ─── Credential pool commands (#4965) ───────────────────────────────────────

/// Resolve the active config.toml path. `--config <path>` overrides; else
/// `$LIBREFANG_HOME/config.toml` (or `~/.librefang/config.toml`).
fn pool_config_path(config_override: Option<PathBuf>) -> PathBuf {
    config_override.unwrap_or_else(|| librefang_home().join("config.toml"))
}

/// Parse config.toml into a `toml_edit::DocumentMut` so comments, blank
/// lines, key ordering, and unrelated sections are preserved through any
/// mutation. Exits with a friendly message on missing-file / parse errors.
/// Shared by all three mutating pool commands so the same diagnostic appears
/// for each entry point.
fn pool_load_doc_or_exit(path: &std::path::Path) -> toml_edit::DocumentMut {
    if !path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }
    let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    if content.trim().is_empty() {
        return toml_edit::DocumentMut::new();
    }
    content
        .parse::<toml_edit::DocumentMut>()
        .unwrap_or_else(|e| {
            ui::error_with_fix(
                &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
                &i18n::t("config-parse-fix-alt"),
            );
            std::process::exit(1);
        })
}

fn pool_write_doc_or_exit(path: &std::path::Path, doc: &toml_edit::DocumentMut) {
    std::fs::write(path, doc.to_string()).unwrap_or_else(|e| {
        ui::error(&format!("Failed to write {}: {e}", path.display()));
        std::process::exit(1);
    });
}

fn pool_strategy_canon(input: &str) -> Option<&'static str> {
    match input.to_ascii_lowercase().replace('-', "_").as_str() {
        "fill_first" | "fillfirst" => Some("fill_first"),
        "round_robin" | "roundrobin" => Some("round_robin"),
        "random" => Some("random"),
        "least_used" | "leastused" => Some("least_used"),
        _ => None,
    }
}

/// Locate the `[[credential_pools]]` entry whose `provider` matches
/// `provider_name`, creating the surrounding `ArrayOfTables` if it does not
/// exist yet. Returns `(array, Some(idx))` on hit and `(array, None)` on miss
/// so the caller can decide whether to append or report an error.
fn pool_lookup_doc_mut<'d>(
    doc: &'d mut toml_edit::DocumentMut,
    provider_name: &str,
) -> (&'d mut toml_edit::ArrayOfTables, Option<usize>) {
    // Insert an empty `[[credential_pools]]` if missing. We use
    // `or_insert(Item::ArrayOfTables(...))` so the rendered output retains
    // the canonical TOML form even when the section was absent in the
    // original file.
    let item = doc
        .entry("credential_pools")
        .or_insert(toml_edit::Item::ArrayOfTables(
            toml_edit::ArrayOfTables::new(),
        ));
    let arr = match item.as_array_of_tables_mut() {
        Some(a) => a,
        None => {
            ui::error("config.toml `credential_pools` exists but is not an array of tables");
            std::process::exit(1);
        }
    };
    let idx = arr.iter().position(|t| {
        t.get("provider")
            .and_then(|v| v.as_str())
            .map(|n| n.eq_ignore_ascii_case(provider_name))
            .unwrap_or(false)
    });
    (arr, idx)
}

fn cmd_auth_pool_list(config: Option<PathBuf>, json: bool) {
    // Prefer the running daemon — its snapshot includes live request_count
    // and cooldown telemetry that config.toml alone cannot provide.
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base_url}/api/credential-pools");
        let resp = client.get(&url).send();
        match resp {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().unwrap_or_default();
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&body).unwrap_or_default()
                    );
                    return;
                }
                print_pool_summary_human(&body);
                return;
            }
            Ok(r) => {
                ui::check_warn(&format!(
                    "Daemon returned HTTP {} — falling back to config.toml view",
                    r.status()
                ));
            }
            Err(e) => {
                ui::check_warn(&format!(
                    "Failed to query daemon at {url}: {e} — falling back to config.toml view"
                ));
            }
        }
    }

    // Offline path: render the static config view (no live telemetry).
    let path = pool_config_path(config);
    if !path.exists() {
        if json {
            println!("[]");
        } else {
            ui::check_warn(&format!(
                "No config at {} and daemon is not running.",
                path.display()
            ));
        }
        return;
    }
    let cfg = load_config(Some(&path)).unwrap_or_else(|e| {
        ui::error(&format!("Failed to load config: {e}"));
        std::process::exit(1);
    });
    let mut pools: Vec<serde_json::Value> = cfg
        .credential_pools
        .iter()
        .map(|p| {
            let strategy = match p.strategy {
                librefang_types::config::CredentialPoolStrategy::FillFirst => "fill_first",
                librefang_types::config::CredentialPoolStrategy::RoundRobin => "round_robin",
                librefang_types::config::CredentialPoolStrategy::Random => "random",
                librefang_types::config::CredentialPoolStrategy::LeastUsed => "least_used",
            };
            let mut keys: Vec<&librefang_types::config::CredentialPoolKeyConfig> =
                p.keys.iter().collect();
            keys.sort_by_key(|k| std::cmp::Reverse(k.priority));
            let creds: Vec<serde_json::Value> = keys
                .iter()
                .map(|k| {
                    let resolved = std::env::var(&k.api_key_env).is_ok();
                    serde_json::json!({
                        "label": k.label,
                        "env_var": k.api_key_env,
                        "priority": k.priority,
                        "env_resolved": resolved,
                    })
                })
                .collect();
            serde_json::json!({
                "provider": p.provider,
                "strategy": strategy,
                "total_count": p.keys.len(),
                "credentials": creds,
            })
        })
        .collect();
    // Deterministic alphabetical ordering (matches the HTTP endpoint).
    pools.sort_by(|a, b| {
        a["provider"]
            .as_str()
            .unwrap_or("")
            .cmp(b["provider"].as_str().unwrap_or(""))
    });
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&pools).unwrap_or_default()
        );
    } else {
        print_pool_summary_human(&serde_json::Value::Array(pools));
    }
}

fn print_pool_summary_human(body: &serde_json::Value) {
    let pools = match body.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => {
            println!("{}", "No credential pools configured.".to_string().dimmed());
            println!();
            println!("Add one with:");
            println!(
                "  librefang auth pool add openai OPENAI_API_KEY_1 --label Primary --priority 10"
            );
            return;
        }
    };
    for pool in pools {
        let provider = pool["provider"].as_str().unwrap_or("");
        let strategy = pool["strategy"].as_str().unwrap_or("");
        let total = pool["total_count"].as_u64().unwrap_or(0);
        let available = pool["available_count"].as_u64().unwrap_or(total);
        let header = format!("{provider}  ({strategy})");
        println!("{}", header.bold());
        println!(
            "  keys: {}/{} available",
            available.to_string().bold(),
            total
        );
        if let Some(creds) = pool["credentials"].as_array() {
            for c in creds {
                let label = c["label"].as_str().unwrap_or("");
                let hint = c["key_hint"].as_str().unwrap_or("");
                let env_var = c["env_var"].as_str().unwrap_or("");
                let key_display = if hint.is_empty() { env_var } else { hint };
                let pri = c["priority"].as_u64().unwrap_or(0);
                let reqs = c["request_count"].as_u64();
                let exhausted = c["is_exhausted"].as_bool().unwrap_or(false);
                let env_resolved = c["env_resolved"].as_bool();
                let cooldown = c.get("cooldown_remaining_secs");

                let status: String = if exhausted {
                    if let Some(serde_json::Value::String(s)) = cooldown {
                        if s == "permanent" {
                            "invalid".red().to_string()
                        } else {
                            "exhausted".yellow().to_string()
                        }
                    } else if let Some(serde_json::Value::Number(n)) = cooldown {
                        format!(
                            "{} {}",
                            "cooldown".yellow(),
                            format!("({}s left)", n).dimmed()
                        )
                    } else {
                        "exhausted".yellow().to_string()
                    }
                } else if env_resolved == Some(false) {
                    "env-missing".red().to_string()
                } else {
                    "healthy".green().to_string()
                };

                let reqs_str = reqs.map(|r| format!(" requests={r}")).unwrap_or_default();
                println!(
                    "    - [{label}] {key_display}  priority={pri}{reqs_str}  status={status}"
                );
            }
        }
        println!();
    }
}

/// Best-effort env-var name sanity check used by `auth pool add`. POSIX
/// env-var names are `[A-Z_][A-Z0-9_]*`; reject obvious nonsense (spaces,
/// punctuation, leading digit) at config-time so the operator finds out
/// here instead of seeing "pool has no resolvable keys" from the daemon
/// on next boot.
fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_uppercase() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn cmd_auth_pool_add(
    config: Option<PathBuf>,
    provider: &str,
    env_var: &str,
    label: &str,
    priority: u32,
) {
    if !is_valid_env_var_name(env_var) {
        ui::error(&format!(
            "`{env_var}` is not a valid env var name. Expected uppercase letters, digits, and underscores (e.g. OPENAI_API_KEY_2)."
        ));
        std::process::exit(1);
    }
    // Validate the env var is actually set at add time. Without this the
    // operator can stage a typo into config.toml and only find out at the
    // next daemon boot via a "Credential pool key env var not set — skipping"
    // warning that may go unnoticed. Treat empty/whitespace as unset too —
    // an env var set to "" cannot drive a real provider call.
    match std::env::var(env_var) {
        Ok(v) if !v.trim().is_empty() => {}
        Ok(_) => {
            ui::error_with_fix(
                &format!("env var `{env_var}` is set but empty."),
                &format!("Set it to your API key before adding the pool entry, e.g.\n  export {env_var}=sk-…\nThen retry."),
            );
            std::process::exit(1);
        }
        Err(_) => {
            ui::error_with_fix(
                &format!("env var `{env_var}` is not set in the current shell."),
                &format!("Export it before adding the pool entry, e.g.\n  export {env_var}=sk-…\nThen retry. (The daemon will read it from its own environment at boot time — make sure it's exported there too.)"),
            );
            std::process::exit(1);
        }
    }

    let path = pool_config_path(config);
    let mut doc = pool_load_doc_or_exit(&path);

    {
        let (arr, idx) = pool_lookup_doc_mut(&mut doc, provider);

        match idx {
            Some(i) => {
                // Append to existing pool's keys array-of-tables.
                let pool_tbl = arr.get_mut(i).expect("idx within bounds");
                let keys_item = pool_tbl
                    .entry("keys")
                    .or_insert(toml_edit::Item::ArrayOfTables(
                        toml_edit::ArrayOfTables::new(),
                    ));
                let keys_arr = match keys_item.as_array_of_tables_mut() {
                    Some(a) => a,
                    None => {
                        ui::error(&format!(
                            "Pool for `{provider}` has a `keys` field that is not an array of tables."
                        ));
                        std::process::exit(1);
                    }
                };
                // Duplicate guard: same env_var on the same provider is an error.
                let dup = keys_arr.iter().any(|k| {
                    k.get("api_key_env")
                        .and_then(|v| v.as_str())
                        .map(|e| e == env_var)
                        .unwrap_or(false)
                });
                if dup {
                    ui::error(&format!(
                        "Key with env_var `{env_var}` already exists in pool for provider `{provider}`."
                    ));
                    std::process::exit(1);
                }
                let mut new_key_tbl = toml_edit::Table::new();
                new_key_tbl["api_key_env"] = toml_edit::value(env_var);
                new_key_tbl["label"] = toml_edit::value(label);
                new_key_tbl["priority"] = toml_edit::value(priority as i64);
                keys_arr.push(new_key_tbl);
            }
            None => {
                // Create the pool with default strategy = fill_first.
                let mut pool_tbl = toml_edit::Table::new();
                pool_tbl["provider"] = toml_edit::value(provider);
                pool_tbl["strategy"] = toml_edit::value("fill_first");
                let mut keys_arr = toml_edit::ArrayOfTables::new();
                let mut new_key_tbl = toml_edit::Table::new();
                new_key_tbl["api_key_env"] = toml_edit::value(env_var);
                new_key_tbl["label"] = toml_edit::value(label);
                new_key_tbl["priority"] = toml_edit::value(priority as i64);
                keys_arr.push(new_key_tbl);
                pool_tbl.insert("keys", toml_edit::Item::ArrayOfTables(keys_arr));
                arr.push(pool_tbl);
            }
        }
    }

    pool_write_doc_or_exit(&path, &doc);
    ui::success(&format!(
        "Added key `{label}` (env={env_var}, priority={priority}) to pool for `{provider}`. Restart the daemon or hot-reload config to apply."
    ));
}

fn cmd_auth_pool_remove(config: Option<PathBuf>, provider: &str, env_var: &str) {
    let path = pool_config_path(config);
    let mut doc = pool_load_doc_or_exit(&path);

    let mut empty_pool_removed = false;
    {
        let (arr, idx) = pool_lookup_doc_mut(&mut doc, provider);
        let Some(i) = idx else {
            ui::error(&format!(
                "No credential pool configured for provider `{provider}`."
            ));
            std::process::exit(1);
        };

        let pool_tbl = arr.get_mut(i).expect("idx within bounds");
        let Some(keys_item) = pool_tbl.get_mut("keys") else {
            ui::error(&format!("Pool for `{provider}` has no keys array."));
            std::process::exit(1);
        };
        let Some(keys_arr) = keys_item.as_array_of_tables_mut() else {
            ui::error(&format!(
                "Pool for `{provider}` has a `keys` field that is not an array of tables."
            ));
            std::process::exit(1);
        };
        let before = keys_arr.len();
        // ArrayOfTables has no `retain` — walk indices backwards and remove
        // matching entries one by one so index shifts don't skip neighbors.
        for j in (0..keys_arr.len()).rev() {
            let matches = keys_arr
                .get(j)
                .and_then(|t| t.get("api_key_env"))
                .and_then(|v| v.as_str())
                .map(|e| e == env_var)
                .unwrap_or(false);
            if matches {
                keys_arr.remove(j);
            }
        }
        if keys_arr.len() == before {
            ui::error(&format!(
                "No key with env_var `{env_var}` found in pool for `{provider}`."
            ));
            std::process::exit(1);
        }
        if keys_arr.is_empty() {
            arr.remove(i);
            empty_pool_removed = true;
        }
    }

    pool_write_doc_or_exit(&path, &doc);
    if empty_pool_removed {
        ui::success(&format!(
            "Removed key `{env_var}` from pool for `{provider}`. Pool is now empty and has been removed entirely. Restart the daemon or hot-reload config to apply."
        ));
    } else {
        ui::success(&format!(
            "Removed key `{env_var}` from pool for `{provider}`. Restart the daemon or hot-reload config to apply."
        ));
    }
}

fn cmd_auth_pool_strategy(config: Option<PathBuf>, provider: &str, strategy: &str) {
    let Some(canon) = pool_strategy_canon(strategy) else {
        ui::error(&format!(
            "Unknown strategy `{strategy}`. Valid: fill_first, round_robin, random, least_used."
        ));
        std::process::exit(1);
    };

    let path = pool_config_path(config);
    let mut doc = pool_load_doc_or_exit(&path);

    {
        let (arr, idx) = pool_lookup_doc_mut(&mut doc, provider);
        let Some(i) = idx else {
            ui::error(&format!(
                "No credential pool configured for provider `{provider}`."
            ));
            std::process::exit(1);
        };
        let pool_tbl = arr.get_mut(i).expect("idx within bounds");
        pool_tbl["strategy"] = toml_edit::value(canon);
    }

    pool_write_doc_or_exit(&path, &doc);
    ui::success(&format!(
        "Set pool strategy for `{provider}` to `{canon}`. Restart the daemon or hot-reload config to apply."
    ));
}

// ---------------------------------------------------------------------------
// Vault commands (librefang vault init/set/list/remove)
// ---------------------------------------------------------------------------

fn cmd_vault_init() {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    match vault.init() {
        Ok(()) => ui::success(&i18n::t("vault-initialized")),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

fn cmd_vault_set(key: &str) {
    use zeroize::Zeroizing;

    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error(&i18n::t("vault-not-init-run"));
        std::process::exit(1);
    }

    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    let value = prompt_input(&format!("Enter value for {key}: "));
    if value.is_empty() {
        ui::error(&i18n::t("vault-empty-value"));
        std::process::exit(1);
    }

    match vault.set(key.to_string(), Zeroizing::new(value)) {
        Ok(()) => ui::success(&i18n::t_args("vault-stored", &[("key", key)])),
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-store-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

fn cmd_vault_list() {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        println!("{}", i18n::t("vault-not-init-run"));
        return;
    }

    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    let keys = vault.list_keys();
    if keys.is_empty() {
        println!("Vault is empty.");
    } else {
        println!("Stored credentials ({}):", keys.len());
        for key in keys {
            println!("  {key}");
        }
    }
}

fn cmd_vault_remove(key: &str) {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error(&i18n::t("vault-not-initialized"));
        std::process::exit(1);
    }
    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    match vault.remove(key) {
        Ok(true) => ui::success(&i18n::t_args("vault-removed", &[("key", key)])),
        Ok(false) => println!("{}", i18n::t_args("vault-key-not-found", &[("key", key)])),
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-remove-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

/// Rotate the vault master key by re-encrypting every entry under a fresh
/// 32-byte key. Issue #3651.
///
/// Source of the keys (in order):
///   - OLD: env var `LIBREFANG_VAULT_KEY_OLD` (REQUIRED)
///   - NEW: env var `LIBREFANG_VAULT_KEY_NEW` unless `--from-stdin` is set,
///     in which case stdin is read until EOF and trimmed.
///
/// Both must be base64 of exactly 32 raw bytes (`openssl rand -base64 32`,
/// matches `LIBREFANG_VAULT_KEY` in production). Any other length is
/// rejected up-front before any vault state is touched.
///
/// On success the vault file is atomically replaced (vault.rs's `save()`
/// already writes to `<path>.tmp` and `rename`s — re-using it gives us the
/// atomic-swap-on-disk guarantee for free) and prints the new key fingerprint
/// so the operator has a non-secret confirmation that the rotation took.
fn cmd_vault_rotate_key(from_stdin: bool) {
    use std::io::Read as _;
    use zeroize::Zeroizing;

    let home = librefang_home();
    let vault_path = home.join("vault.enc");

    // Pre-flight: vault must already exist. Refuse on missing file rather
    // than silently `init()` — rotating a vault that was never created is
    // a no-op masking an operator error.
    if !vault_path.exists() {
        ui::error(&i18n::t("vault-rotate-no-vault"));
        std::process::exit(1);
    }

    // Read OLD key from env. Always required.
    let old_key_b64 = match std::env::var("LIBREFANG_VAULT_KEY_OLD") {
        Ok(s) if !s.is_empty() => Zeroizing::new(s),
        _ => {
            ui::error(&i18n::t("vault-rotate-old-key-missing"));
            std::process::exit(1);
        }
    };

    // Read NEW key from stdin or env, depending on the flag. stdin wins
    // when `--from-stdin` is set so a key in env can't accidentally
    // override an explicit stdin pipe.
    let new_key_b64 = if from_stdin {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            ui::error(&i18n::t_args(
                "vault-rotate-stdin-read-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
        let trimmed = buf.trim().to_string();
        if trimmed.is_empty() {
            ui::error(&i18n::t("vault-rotate-stdin-empty"));
            std::process::exit(1);
        }
        Zeroizing::new(trimmed)
    } else {
        match std::env::var("LIBREFANG_VAULT_KEY_NEW") {
            Ok(s) if !s.is_empty() => Zeroizing::new(s),
            _ => {
                ui::error(&i18n::t("vault-rotate-new-key-missing"));
                std::process::exit(1);
            }
        }
    };

    // Reject identical OLD/NEW up-front — silently no-op rotations are a
    // footgun. (`Zeroizing<String>` derefs to `&str` so direct comparison
    // is safe and constant-time on equal-length strings is unnecessary
    // here: this is a configuration check, not a credential check.)
    if old_key_b64.as_str() == new_key_b64.as_str() {
        ui::error(&i18n::t("vault-rotate-same-key"));
        std::process::exit(1);
    }

    // Decode both keys via the same parser the production daemon uses so
    // any rejection here matches what the daemon will reject at boot.
    let old_key_bytes = match librefang_extensions::vault::decode_master_key(&old_key_b64) {
        Ok(k) => k,
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-rotate-old-key-invalid",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    };
    let new_key_bytes = match librefang_extensions::vault::decode_master_key(&new_key_b64) {
        Ok(k) => k,
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-rotate-new-key-invalid",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    };

    // Open + unlock with OLD key. Use `unlock_with_key` so the rotation
    // doesn't accidentally pick up a stale env / keyring value — we want
    // the rotation to fail loudly if `LIBREFANG_VAULT_KEY_OLD` doesn't
    // match the on-disk vault.
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path.clone());
    if let Err(e) = vault.unlock_with_key(old_key_bytes) {
        ui::error(&i18n::t_args(
            "vault-rotate-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    // Verify (or backfill) the sentinel under the OLD key BEFORE rotating.
    // This catches "OLD key decrypted noise" and ensures legacy vaults
    // gain a sentinel during rotation rather than after.
    if let Err(e) = vault.verify_or_install_sentinel() {
        ui::error(&i18n::t_args(
            "vault-rotate-sentinel-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    let entry_count = vault.list_keys().len();

    // Re-encrypt the entire vault under the NEW key. `rewrap_with_new_key`
    // re-uses the proven atomic save path inside vault.rs (write to
    // `<path>.tmp`, fsync, rename) — no separate code path to maintain.
    if let Err(e) = vault.rewrap_with_new_key(new_key_bytes) {
        ui::error(&i18n::t_args(
            "vault-rotate-rewrap-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    ui::success(&i18n::t_args(
        "vault-rotate-success",
        &[("count", &entry_count.to_string())],
    ));
    println!("{}", i18n::t("vault-rotate-next-step"));
}

// ---------------------------------------------------------------------------
// hash-password command
// ---------------------------------------------------------------------------

fn cmd_hash_password(password: Option<String>) {
    let pass = match password {
        Some(p) => p,
        None => {
            let p1 = prompt_input("Enter password: ");
            if p1.is_empty() {
                ui::error("Password cannot be empty.");
                std::process::exit(1);
            }
            let p2 = prompt_input("Confirm password: ");
            if p1 != p2 {
                ui::error("Passwords do not match.");
                std::process::exit(1);
            }
            p1
        }
    };

    match librefang_api::password_hash::hash_password(&pass) {
        Ok(hash) => {
            println!("\n{hash}\n");
            println!("Add to config.toml:");
            println!("  dashboard_pass_hash = \"{hash}\"");
        }
        Err(e) => {
            ui::error(&format!("Failed to hash password: {e}"));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Scaffold commands (librefang new skill/integration)
// ---------------------------------------------------------------------------

fn cmd_scaffold(kind: ScaffoldKind) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let result = match kind {
        ScaffoldKind::Skill => {
            librefang_extensions::installer::scaffold_skill(&cwd.join("my-skill"))
        }
        ScaffoldKind::Mcp => {
            librefang_extensions::installer::scaffold_integration(&cwd.join("my-mcp"))
        }
    };
    match result {
        Ok(msg) => ui::success(&msg),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// New command handlers
// ---------------------------------------------------------------------------

fn cmd_models_list(provider_filter: Option<&str>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = match provider_filter {
            Some(p) => format!("{base}/api/models?provider={p}"),
            None => format!("{base}/api/models"),
        };
        let body = daemon_json(client.get(&url).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body
            .get("models")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array())
        {
            if arr.is_empty() {
                println!("No models found.");
                return;
            }
            let mut t = crate::table::Table::new(&["MODEL", "PROVIDER", "TIER", "CONTEXT"]);
            for m in arr {
                t.add_row(&[
                    m["id"].as_str().unwrap_or("?"),
                    m["provider"].as_str().unwrap_or("?"),
                    m["tier"].as_str().unwrap_or("?"),
                    &m["context_window"].as_u64().unwrap_or(0).to_string(),
                ]);
            }
            t.print();
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        // Standalone: use ModelCatalog directly
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let models = catalog.list_models();
        if json {
            let arr: Vec<serde_json::Value> = models
                .iter()
                .filter(|m| provider_filter.is_none_or(|p| m.provider == p))
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "provider": m.provider,
                        "tier": format!("{:?}", m.tier),
                        "context_window": m.context_window,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        if models.is_empty() {
            println!("No models in catalog.");
            return;
        }
        let mut t = crate::table::Table::new(&["MODEL", "PROVIDER", "TIER", "CONTEXT"]);
        for m in models {
            if let Some(p) = provider_filter {
                if m.provider != p {
                    continue;
                }
            }
            t.add_row(&[
                &m.id,
                &m.provider,
                &format!("{:?}", m.tier),
                &m.context_window.to_string(),
            ]);
        }
        t.print();
    }
}

fn cmd_models_aliases(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/models/aliases")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body.get("aliases").and_then(|v| v.as_array()) {
            let mut t = crate::table::Table::new(&["ALIAS", "RESOLVES TO"]);
            for entry in arr {
                t.add_row(&[
                    entry["alias"].as_str().unwrap_or("?"),
                    entry["model_id"].as_str().unwrap_or("?"),
                ]);
            }
            t.print();
        } else if let Some(obj) = body.as_object() {
            // Fallback for plain {alias: model_id} format
            let mut t = crate::table::Table::new(&["ALIAS", "RESOLVES TO"]);
            for (alias, target) in obj {
                t.add_row(&[alias.as_str(), target.as_str().unwrap_or("?")]);
            }
            t.print();
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let aliases = catalog.list_aliases();
        if json {
            let obj: serde_json::Map<String, serde_json::Value> = aliases
                .iter()
                .map(|(a, t)| (a.to_string(), serde_json::Value::String(t.to_string())))
                .collect();
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
            return;
        }
        let mut t = crate::table::Table::new(&["ALIAS", "RESOLVES TO"]);
        for (alias, target) in aliases {
            t.add_row(&[alias, target]);
        }
        t.print();
    }
}

fn cmd_models_providers(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/providers")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body
            .get("providers")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array())
        {
            let mut t = crate::table::Table::new(&["PROVIDER", "AUTH", "MODELS", "BASE URL"]);
            for p in arr {
                t.add_row(&[
                    p["id"].as_str().unwrap_or("?"),
                    p["auth_status"].as_str().unwrap_or("?"),
                    &p["model_count"].as_u64().unwrap_or(0).to_string(),
                    p["base_url"].as_str().unwrap_or(""),
                ]);
            }
            t.print();
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let providers = catalog.list_providers();
        if json {
            let arr: Vec<serde_json::Value> = providers
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id,
                        "auth_status": format!("{:?}", p.auth_status),
                        "model_count": p.model_count,
                        "base_url": p.base_url,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        let mut t = crate::table::Table::new(&["PROVIDER", "AUTH", "MODELS", "BASE URL"]);
        for p in providers {
            t.add_row(&[
                &p.id,
                &format!("{:?}", p.auth_status),
                &p.model_count.to_string(),
                &p.base_url,
            ]);
        }
        t.print();
    }
}

fn cmd_models_set(model: Option<String>) {
    let model = match model {
        Some(m) => m,
        None => pick_model(),
    };
    let base = require_daemon("models set");
    let client = daemon_client();
    // Use the config set approach through the API
    let body = daemon_json(
        client
            .post(format!("{base}/api/config/set"))
            .json(&serde_json::json!({"path": "default_model.model", "value": model}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "model-set-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("model-set-success", &[("model", &model)]));
    }
}

/// Interactive model picker — shows numbered list, accepts number or model ID.
fn pick_model() -> String {
    let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
    let models = catalog.list_models();

    if models.is_empty() {
        ui::error(&i18n::t("model-no-catalog"));
        std::process::exit(1);
    }

    // Group by provider for display
    let mut by_provider: std::collections::BTreeMap<
        String,
        Vec<&librefang_types::model_catalog::ModelCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for m in models {
        by_provider.entry(m.provider.clone()).or_default().push(m);
    }

    ui::section(&i18n::t("section-select-model"));
    ui::blank();

    let mut numbered: Vec<&str> = Vec::new();
    let mut idx = 1;
    for (provider, provider_models) in &by_provider {
        println!("  {}:", provider.bold());
        for m in provider_models {
            println!("    {idx:>3}. {:<36} {:?}", m.id, m.tier);
            numbered.push(&m.id);
            idx += 1;
        }
    }
    ui::blank();

    loop {
        let input = prompt_input("  Enter number or model ID: ");
        if input.is_empty() {
            continue;
        }
        // Try as number first
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= numbered.len() {
                return numbered[n - 1].to_string();
            }
            ui::error(&i18n::t_args(
                "model-out-of-range",
                &[("max", &numbered.len().to_string())],
            ));
            continue;
        }
        // Accept direct model ID if it exists in catalog
        if models.iter().any(|m| m.id == input) {
            return input;
        }
        // Accept as alias
        if catalog.resolve_alias(&input).is_some() {
            return input;
        }
        // Accept any string (user might know a model not in catalog)
        return input;
    }
}

fn cmd_approvals_list(json: bool) {
    let base = require_daemon("approvals list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/approvals")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("approvals")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No pending approvals.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "AGENT", "TYPE", "REQUEST"]);
        for a in arr {
            t.add_row(&[
                a["id"].as_str().unwrap_or("?"),
                a["agent_name"].as_str().unwrap_or("?"),
                a["approval_type"].as_str().unwrap_or("?"),
                a["description"].as_str().unwrap_or(""),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_approvals_respond(id: &str, approve: bool) {
    let base = require_daemon("approvals");
    let client = daemon_client();
    let endpoint = if approve { "approve" } else { "reject" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/approvals/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "approval-failed",
            &[
                ("action", endpoint),
                ("error", body["error"].as_str().unwrap_or("?")),
            ],
        ));
    } else {
        ui::success(&i18n::t_args(
            "approval-responded",
            &[("id", id), ("action", endpoint)],
        ));
    }
}

fn cmd_cron_list(json: bool) {
    let base = require_daemon("cron list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/cron/jobs")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("jobs")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No scheduled jobs.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "AGENT", "SCHEDULE", "ENABLED", "PROMPT"]);
        for j in arr {
            t.add_row(&[
                j["id"].as_str().unwrap_or("?"),
                j["agent_id"].as_str().unwrap_or("?"),
                j["schedule"]["expr"]
                    .as_str()
                    .or_else(|| j["cron_expr"].as_str())
                    .unwrap_or("?"),
                if j["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                &j["action"]["message"]
                    .as_str()
                    .or_else(|| j["prompt"].as_str())
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>(),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_cron_create(agent: &str, spec: &str, prompt: &str, explicit_name: Option<&str>) {
    let base = require_daemon("cron create");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();

    // Use explicit name if provided, otherwise derive from agent + prompt
    let name = if let Some(n) = explicit_name {
        n.to_string()
    } else {
        let short_prompt: String = prompt
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join("-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(64)
            .collect();
        format!(
            "{}-{}",
            agent,
            if short_prompt.is_empty() {
                "job"
            } else {
                &short_prompt
            }
        )
    };

    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs"))
            .json(&serde_json::json!({
                "agent_id": agent,
                "name": name,
                "schedule": {
                    "kind": "cron",
                    "expr": spec
                },
                "action": {
                    "kind": "agent_turn",
                    "message": prompt
                }
            }))
            .send(),
    );
    if let Some(id) = body["job_id"].as_str().or_else(|| body["id"].as_str()) {
        ui::success(&i18n::t_args("cron-created", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "cron-create-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}

fn cmd_cron_delete(id: &str) {
    let base = require_daemon("cron delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/cron/jobs/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "cron-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("cron-deleted", &[("id", id)]));
    }
}

fn cmd_cron_toggle(id: &str, enable: bool) {
    let base = require_daemon("cron");
    let client = daemon_client();
    let endpoint = if enable { "enable" } else { "disable" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "cron-toggle-failed",
            &[
                ("action", endpoint),
                ("error", body["error"].as_str().unwrap_or("?")),
            ],
        ));
    } else {
        ui::success(&i18n::t_args(
            "cron-toggled",
            &[("id", id), ("action", endpoint)],
        ));
    }
}

fn cmd_sessions(agent: Option<&str>, json: bool, active_only: bool) {
    let base = require_daemon("sessions");
    let client = daemon_client();
    let url = match agent {
        Some(a) => format!("{base}/api/sessions?agent={a}"),
        None => format!("{base}/api/sessions"),
    };
    let body = daemon_json(client.get(&url).send());

    // Build a (agent_id -> set<session_id>) map of currently-running sessions.
    // Walks the unique agent ids in the listing once and asks the per-agent
    // runtime endpoint added in #3172. Cheap on dev-scale agent counts; if
    // this ever becomes a hotspot we can add a single-call /api/runtime.
    let session_arr_owned: Option<Vec<serde_json::Value>> = body
        .get("sessions")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| body.as_array().cloned());
    let mut active_sessions: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    if let Some(arr) = session_arr_owned.as_ref() {
        let agent_ids: std::collections::HashSet<String> = arr
            .iter()
            .filter_map(|s| s["agent_id"].as_str().map(|id| id.to_string()))
            .collect();
        for aid in agent_ids {
            let runtime_url = format!("{base}/api/agents/{aid}/runtime");
            if let Ok(resp) = client.get(&runtime_url).send() {
                if let Ok(items) = resp.json::<Vec<serde_json::Value>>() {
                    let sids: std::collections::HashSet<String> = items
                        .iter()
                        .filter_map(|v| v["session_id"].as_str().map(|s| s.to_string()))
                        .collect();
                    active_sessions.insert(aid, sids);
                }
            }
        }
    }

    let is_running = |s: &serde_json::Value| -> bool {
        let aid = match s["agent_id"].as_str() {
            Some(a) => a,
            None => return false,
        };
        let sid = match s["session_id"].as_str().or_else(|| s["id"].as_str()) {
            Some(s) => s,
            None => return false,
        };
        active_sessions
            .get(aid)
            .is_some_and(|set| set.contains(sid))
    };

    if json {
        // Annotate each session with `state` so JSON consumers see the same
        // signal as the table renderer.
        if let Some(arr) = session_arr_owned.as_ref() {
            let annotated: Vec<serde_json::Value> = arr
                .iter()
                .filter(|s| !active_only || is_running(s))
                .map(|s| {
                    let mut out = s.clone();
                    out["state"] = serde_json::Value::String(
                        if is_running(s) { "running" } else { "idle" }.into(),
                    );
                    out
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&annotated).unwrap_or_default()
            );
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
        return;
    }
    if let Some(arr) = session_arr_owned.as_ref() {
        let filtered: Vec<&serde_json::Value> = arr
            .iter()
            .filter(|s| !active_only || is_running(s))
            .collect();
        if filtered.is_empty() {
            if active_only {
                println!("No active sessions.");
            } else {
                println!("No sessions found.");
            }
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "AGENT", "MSGS", "STATE", "LAST ACTIVE"]);
        for s in filtered {
            let state = if is_running(s) { "running" } else { "idle" };
            let agent_id = s["agent_id"].as_str().unwrap_or("");
            let agent_col = if agent_id.len() > 16 {
                &agent_id[..16]
            } else if agent_id.is_empty() {
                s["agent_name"].as_str().unwrap_or("?")
            } else {
                agent_id
            };
            t.add_row(&[
                s["session_id"]
                    .as_str()
                    .or_else(|| s["id"].as_str())
                    .unwrap_or("?"),
                agent_col,
                &s["message_count"].as_u64().unwrap_or(0).to_string(),
                state,
                s["created_at"]
                    .as_str()
                    .or_else(|| s["last_active"].as_str())
                    .unwrap_or("?"),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_security_status(json: bool) {
    let base = require_daemon("security status");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/health/detail")).send());
    if json {
        let data = serde_json::json!({
            "audit_trail": "merkle_hash_chain_sha256",
            "taint_tracking": "information_flow_labels",
            "wasm_sandbox": "dual_metering_fuel_epoch",
            "wire_protocol": "ofp_hmac_sha256_mutual_auth",
            "api_keys": "zeroizing_auto_wipe",
            "manifests": "ed25519_signed",
            "agent_count": body.get("agent_count").and_then(|v| v.as_u64()),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&data).unwrap_or_default()
        );
        return;
    }
    ui::section(&i18n::t("section-security-status"));
    ui::blank();
    ui::kv(&i18n::t("label-audit-trail"), &i18n::t("value-audit-trail"));
    ui::kv(
        &i18n::t("label-taint-tracking"),
        &i18n::t("value-taint-tracking"),
    );
    ui::kv(
        &i18n::t("label-wasm-sandbox"),
        &i18n::t("value-wasm-sandbox"),
    );
    ui::kv(
        &i18n::t("label-wire-protocol"),
        &i18n::t("value-wire-protocol"),
    );
    ui::kv(&i18n::t("label-api-keys"), &i18n::t("value-api-keys"));
    ui::kv(&i18n::t("label-manifests"), &i18n::t("value-manifests"));
    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
        ui::kv(&i18n::t("label-active-agents"), &agents.to_string());
    }
}

fn cmd_security_audit(limit: usize, json: bool) {
    let base = require_daemon("security audit");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/audit/recent?limit={limit}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("entries")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No audit entries.");
            return;
        }
        let mut t = crate::table::Table::new(&["TIMESTAMP", "AGENT", "TYPE", "EVENT"]);
        for entry in arr {
            let agent_id = entry["agent_id"].as_str().unwrap_or("");
            let agent_col = if agent_id.len() > 16 {
                &agent_id[..16]
            } else if agent_id.is_empty() {
                entry["agent_name"].as_str().unwrap_or("?")
            } else {
                agent_id
            };
            t.add_row(&[
                entry["timestamp"].as_str().unwrap_or("?"),
                agent_col,
                entry["action"]
                    .as_str()
                    .or_else(|| entry["event_type"].as_str())
                    .unwrap_or("?"),
                entry["detail"]
                    .as_str()
                    .or_else(|| entry["description"].as_str())
                    .unwrap_or(""),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_security_verify() {
    let base = require_daemon("security verify");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/audit/verify")).send());
    if body["valid"].as_bool().unwrap_or(false) {
        ui::success(&i18n::t("audit-verified"));
    } else {
        ui::error(&i18n::t("audit-failed"));
        if let Some(msg) = body["error"].as_str() {
            ui::hint(msg);
        }
        std::process::exit(1);
    }
}

/// Destructively reset the local audit trail.
///
/// Truncates `audit_entries` in SQLite and removes the anchor file so the
/// next daemon boot seeds a fresh Merkle chain. Refuses to run while the
/// daemon holds the DB (SQLite WAL mode + writer lock) and without
/// `--confirm`.
fn cmd_audit_reset(config: Option<PathBuf>, confirm: bool) {
    let daemon = daemon_config_context(config.as_deref());
    // `load_config` already eprintln!s the underlying parse / deserialize
    // error (see #5186); printing it again here would double the message.
    let kernel_config = match load_config(config.as_deref()) {
        Ok(cfg) => cfg,
        Err(_) => std::process::exit(1),
    };

    let db_path = kernel_config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| kernel_config.data_dir.join("librefang.db"));

    let anchor_path = match kernel_config.audit.anchor_path.as_ref() {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => kernel_config.data_dir.join(p),
        None => kernel_config.data_dir.join("audit.anchor"),
    };

    if !confirm {
        ui::error("audit reset is destructive — re-run with `--confirm` to proceed");
        ui::blank();
        println!("  Would:");
        println!(
            "    1. DELETE all rows from `audit_entries` in {}",
            db_path.display()
        );
        println!("    2. Remove anchor file {}", anchor_path.display());
        println!("  The Merkle chain will restart from the next audit event.");
        std::process::exit(1);
    }

    // Refuse if daemon is running — SQLite writer lock would block or corrupt.
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &format!("daemon is running at {base}; refusing to touch the audit database"),
            "stop the daemon first: `librefang stop`",
        );
        std::process::exit(1);
    }

    if !db_path.exists() {
        ui::error(&format!("database not found at {}", db_path.display()));
        std::process::exit(1);
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            ui::error(&format!("failed to open {}: {e}", db_path.display()));
            std::process::exit(1);
        }
    };

    let rows_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM audit_entries", [], |r| r.get(0))
        .unwrap_or(0);

    // Remove the anchor FIRST. If the subsequent DB truncation then fails,
    // the next daemon boot sees `read_anchor = None` and re-seeds from the
    // current DB tip — a consistent (if still broken) state the user can
    // retry. The reverse order (DB first, anchor second) would instead
    // leave an empty table alongside a stale anchor, which produces a
    // fresh MISMATCH error the user didn't have before calling reset.
    let anchor_removed = if anchor_path.exists() {
        match std::fs::remove_file(&anchor_path) {
            Ok(()) => true,
            Err(e) => {
                ui::error(&format!(
                    "failed to remove anchor {}: {e}",
                    anchor_path.display()
                ));
                std::process::exit(1);
            }
        }
    } else {
        false
    };

    if let Err(e) = conn.execute("DELETE FROM audit_entries", []) {
        ui::error(&format!("failed to truncate audit_entries: {e}"));
        std::process::exit(1);
    }
    drop(conn);
    // `seq` is `INTEGER PRIMARY KEY` without AUTOINCREMENT, so the next
    // insert after an empty table naturally gets seq = 1. No sqlite_sequence
    // fiddling needed.

    ui::success(&format!(
        "Audit trail reset: removed {rows_before} row(s) from audit_entries{}.",
        if anchor_removed {
            format!(", deleted anchor at {}", anchor_path.display())
        } else {
            " (no anchor file to remove)".to_string()
        }
    ));
    ui::hint("The next daemon boot will seed a fresh Merkle chain from the current tip.");
}

fn cmd_memory_list(agent: &str, json: bool) {
    let base = require_daemon("memory list");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("kv_pairs")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No memory entries for agent '{agent}'.");
            return;
        }
        let mut t = crate::table::Table::new(&["KEY", "VALUE"]);
        for kv in arr {
            t.add_row(&[
                kv["key"].as_str().unwrap_or("?"),
                &kv["value"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(50)
                    .collect::<String>(),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_memory_get(agent: &str, key: &str, json: bool) {
    let base = require_daemon("memory get");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(val) = body["value"].as_str() {
        println!("{val}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_memory_set(agent: &str, key: &str, value: &str) {
    let base = require_daemon("memory set");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .put(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .json(&serde_json::json!({"value": value}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "memory-set-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args(
            "memory-set",
            &[("key", key), ("agent", &agent)],
        ));
    }
}

fn cmd_memory_delete(agent: &str, key: &str) {
    let base = require_daemon("memory delete");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "memory-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args(
            "memory-deleted",
            &[("key", key), ("agent", &agent)],
        ));
    }
}

fn cmd_devices_list(json: bool) {
    let base = require_daemon("devices list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/pairing/devices")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No paired devices.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "NAME", "LAST SEEN"]);
        for d in arr {
            t.add_row(&[
                d["id"].as_str().unwrap_or("?"),
                d["name"].as_str().unwrap_or("?"),
                d["last_seen"].as_str().unwrap_or("?"),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_devices_pair() {
    let base = require_daemon("qr");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/pairing/request")).send());
    if let Some(qr) = body["qr_data"].as_str() {
        ui::section(&i18n::t("section-device-pairing"));
        ui::blank();
        // Render a simple text-based QR representation
        println!("  {}", i18n::t("device-scan-qr"));
        ui::blank();
        println!("  {qr}");
        ui::blank();
        if let Some(code) = body["pairing_code"].as_str() {
            ui::kv(&i18n::t("label-pairing-code"), code);
        }
        if let Some(expires) = body["expires_at"].as_str() {
            ui::kv(&i18n::t("label-expires"), expires);
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_devices_remove(id: &str) {
    let base = require_daemon("devices remove");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/pairing/devices/{id}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "device-remove-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("device-removed", &[("id", id)]));
    }
}

fn cmd_webhooks_list(json: bool) {
    let base = require_daemon("webhooks list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/webhooks")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("webhooks")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No webhooks configured.");
            return;
        }
        let mut t = crate::table::Table::new(&["ID", "NAME", "ENABLED", "URL"]);
        for w in arr {
            t.add_row(&[
                w["id"].as_str().unwrap_or("?"),
                w["name"].as_str().unwrap_or("?"),
                if w["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                w["url"].as_str().unwrap_or(""),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_webhooks_create(agent: &str, url: &str) {
    let base = require_daemon("webhooks create");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();

    // Derive a name from the URL hostname
    let name = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "webhook".to_string());

    let body = daemon_json(
        client
            .post(format!("{base}/api/webhooks"))
            .json(&serde_json::json!({
                "name": format!("{agent}-{name}"),
                "url": url,
                "events": ["all"],
            }))
            .send(),
    );
    if let Some(id) = body["id"].as_str() {
        ui::success(&i18n::t_args("webhook-created", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "webhook-create-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}

fn cmd_webhooks_delete(id: &str) {
    let base = require_daemon("webhooks delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/webhooks/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "webhook-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("webhook-deleted", &[("id", id)]));
    }
}

fn cmd_webhooks_test(id: &str) {
    let base = require_daemon("webhooks test");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/webhooks/{id}/test")).send());
    if body["success"].as_bool().unwrap_or(false) {
        ui::success(&i18n::t_args("webhook-test-ok", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "webhook-test-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}

fn cmd_message(agent: &str, text: &str, json: bool, incognito: bool) {
    let base = require_daemon("message");
    let agent_id = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/agents/{agent_id}/message"))
            .json(&serde_json::json!({"message": text, "incognito": incognito}))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else if let Some(reply) = body["reply"].as_str() {
        println!("{reply}");
    } else if let Some(reply) = body["response"].as_str() {
        println!("{reply}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_system_info(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/status")).send());
        if json {
            let mut data = body.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert(
                    "version".to_string(),
                    serde_json::json!(env!("CARGO_PKG_VERSION")),
                );
                obj.insert("api_url".to_string(), serde_json::json!(base));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&data).unwrap_or_default()
            );
            return;
        }
        ui::section(&i18n::t("section-system-info"));
        ui::blank();
        ui::kv(&i18n::t("label-version"), env!("CARGO_PKG_VERSION"));
        ui::kv(
            &i18n::t("label-status"),
            body["status"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-agents"),
            &body["agent_count"].as_u64().unwrap_or(0).to_string(),
        );
        ui::kv(
            &i18n::t("label-provider"),
            body["default_provider"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-model"),
            body["default_model"].as_str().unwrap_or("?"),
        );
        ui::kv(&i18n::t("label-api"), &base);
        ui::kv(
            &i18n::t("label-data-dir"),
            body["data_dir"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-uptime"),
            &format!("{}s", body["uptime_seconds"].as_u64().unwrap_or(0)),
        );
    } else {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "daemon": "not_running",
                })
            );
            return;
        }
        ui::section(&i18n::t("section-system-info"));
        ui::blank();
        ui::kv(&i18n::t("label-version"), env!("CARGO_PKG_VERSION"));
        ui::kv_warn(
            &i18n::t("label-daemon"),
            &i18n::t("label-daemon-not-running"),
        );
        ui::hint(&i18n::t("hint-start-daemon"));
    }
}

fn cmd_system_version(json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({"version": env!("CARGO_PKG_VERSION")})
        );
        return;
    }
    println!("librefang {}", env!("CARGO_PKG_VERSION"));
}

// ---------------------------------------------------------------------------
// Service management (boot auto-start)
// ---------------------------------------------------------------------------

/// Resolve the absolute path to the current librefang binary.
fn resolve_binary_path() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("librefang"))
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_exe().unwrap_or_else(|_| "librefang".into()))
}

fn cmd_service_install() {
    // Warn if running as root — the service would be installed for root, not
    // the actual user. This catches `sudo librefang service install` mistakes.
    #[cfg(unix)]
    {
        // SAFETY: geteuid() is always safe to call.
        if unsafe { libc::geteuid() } == 0 {
            ui::error(
                "Running as root — the service will be installed for the root account, \
                 not your user. Run without sudo instead.",
            );
            std::process::exit(1);
        }
    }

    let binary = resolve_binary_path();

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let librefang_home = cli_librefang_home();

    #[cfg(target_os = "linux")]
    {
        service_install_linux(&binary, &librefang_home);
    }
    #[cfg(target_os = "macos")]
    {
        service_install_macos(&binary, &librefang_home);
    }
    #[cfg(windows)]
    {
        service_install_windows(&binary);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = &binary;
        ui::error("Auto-start service is not supported on this platform.");
    }
}

#[cfg(target_os = "linux")]
fn service_install_linux(binary: &std::path::Path, librefang_home: &std::path::Path) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Cannot determine home directory.");
            return;
        }
    };
    let service_dir = home.join(".config/systemd/user");
    if let Err(e) = std::fs::create_dir_all(&service_dir) {
        ui::error(&format!("Failed to create {}: {e}", service_dir.display()));
        return;
    }

    let unit = format!(
        "[Unit]\n\
         Description=LibreFang Agent OS Daemon\n\
         Documentation=https://librefang.ai\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={binary} start --foreground\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         WorkingDirectory={home}\n\
         EnvironmentFile=-{home}/env\n\
         EnvironmentFile=-{home}/secrets.env\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        binary = binary.display(),
        home = librefang_home.display(),
    );

    let service_path = service_dir.join("librefang.service");
    if let Err(e) = std::fs::write(&service_path, &unit) {
        ui::error(&format!("Failed to write {}: {e}", service_path.display()));
        return;
    }
    ui::success(&format!("Wrote {}", service_path.display()));

    // Reload and enable
    let reload = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();
    if let Ok(o) = &reload {
        if !o.status.success() {
            ui::error("systemctl --user daemon-reload failed");
            return;
        }
    }
    let enable = std::process::Command::new("systemctl")
        .args(["--user", "enable", "librefang.service"])
        .output();
    match enable {
        Ok(o) if o.status.success() => {
            ui::success("Service enabled (will start on next login)");
            ui::hint("Start now with: systemctl --user start librefang.service");
            // Enable lingering so the user service runs without an active login session
            ui::hint("For headless servers, also run: loginctl enable-linger");
        }
        _ => ui::error("systemctl --user enable librefang.service failed"),
    }
}

#[cfg(target_os = "macos")]
fn service_install_macos(binary: &std::path::Path, librefang_home: &std::path::Path) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Cannot determine home directory.");
            return;
        }
    };
    let agents_dir = home.join("Library/LaunchAgents");
    if let Err(e) = std::fs::create_dir_all(&agents_dir) {
        ui::error(&format!("Failed to create {}: {e}", agents_dir.display()));
        return;
    }

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.librefang.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>start</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>WorkingDirectory</key>
    <string>{home}</string>
    <key>StandardOutPath</key>
    <string>{home}/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>{home}/daemon.log</string>
</dict>
</plist>
"#,
        binary = binary.display(),
        home = librefang_home.display(),
    );

    let plist_path = agents_dir.join("ai.librefang.daemon.plist");

    // Unload existing service first (if any) to avoid launchctl errors
    if plist_path.exists() {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist_path.to_string_lossy()])
            .output();
    }

    if let Err(e) = std::fs::write(&plist_path, &plist) {
        ui::error(&format!("Failed to write {}: {e}", plist_path.display()));
        return;
    }
    ui::success(&format!("Wrote {}", plist_path.display()));

    let load = std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output();
    match load {
        Ok(o) if o.status.success() => {
            ui::success("LaunchAgent loaded (will start on login and now)");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            ui::error(&format!("launchctl load failed: {stderr}"));
        }
        Err(e) => ui::error(&format!("Failed to run launchctl: {e}")),
    }
}

#[cfg(windows)]
fn service_install_windows(binary: &std::path::Path) {
    let value = format!("\"{}\" start", binary.display());
    let output = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "LibreFang",
            "/t",
            "REG_SZ",
            "/d",
            &value,
            "/f",
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            ui::success("Added to Windows startup (HKCU\\...\\Run)");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            ui::error(&format!("Failed to write registry: {stderr}"));
        }
        Err(e) => ui::error(&format!("Failed to run reg.exe: {e}")),
    }
}

fn cmd_service_uninstall() {
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let service_path = home.join(".config/systemd/user/librefang.service");
        if service_path.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "librefang.service"])
                .output();
            match std::fs::remove_file(&service_path) {
                Ok(()) => {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output();
                    ui::success("Removed systemd user service");
                }
                Err(e) => ui::error(&format!("Failed to remove service file: {e}")),
            }
        } else {
            ui::hint("No systemd user service found — nothing to remove.");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let plist_path = home.join("Library/LaunchAgents/ai.librefang.daemon.plist");
        if plist_path.exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist_path.to_string_lossy()])
                .output();
            match std::fs::remove_file(&plist_path) {
                Ok(()) => ui::success("Removed LaunchAgent"),
                Err(e) => ui::error(&format!("Failed to remove plist: {e}")),
            }
        } else {
            ui::hint("No LaunchAgent found — nothing to remove.");
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
                "/f",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success("Removed from Windows startup");
            }
            _ => ui::hint("No startup entry found — nothing to remove."),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        ui::error("Auto-start service is not supported on this platform.");
    }
}

fn cmd_service_status() {
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let service_path = home.join(".config/systemd/user/librefang.service");
        if service_path.exists() {
            ui::success("Systemd user service is registered");
            // Show enabled/active status
            if let Ok(output) = std::process::Command::new("systemctl")
                .args(["--user", "is-enabled", "librefang.service"])
                .output()
            {
                let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
                ui::kv("  Enabled", &status);
            }
            if let Ok(output) = std::process::Command::new("systemctl")
                .args(["--user", "is-active", "librefang.service"])
                .output()
            {
                let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
                ui::kv("  Active", &status);
            }
        } else {
            ui::hint("No systemd user service registered.");
            ui::hint("Run `librefang service install` to set it up.");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let plist_path = home.join("Library/LaunchAgents/ai.librefang.daemon.plist");
        if plist_path.exists() {
            ui::success("LaunchAgent is registered");
            if let Ok(output) = std::process::Command::new("launchctl")
                .args(["list"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let running = stdout.lines().any(|l| l.contains("ai.librefang.daemon"));
                ui::kv("  Loaded", if running { "yes" } else { "not loaded" });
            }
        } else {
            ui::hint("No LaunchAgent registered.");
            ui::hint("Run `librefang service install` to set it up.");
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success("Windows startup entry is registered");
            }
            _ => {
                ui::hint("No startup entry registered.");
                ui::hint("Run `librefang service install` to set it up.");
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        ui::error("Auto-start service is not supported on this platform.");
    }
}

fn cmd_reset(confirm: bool) {
    let librefang_dir = cli_librefang_home();

    if !librefang_dir.exists() {
        println!(
            "Nothing to reset — {} does not exist.",
            librefang_dir.display()
        );
        return;
    }

    if !confirm {
        println!("  This will delete all data in {}", librefang_dir.display());
        println!("  Including: config, database, agent manifests, credentials.");
        println!();
        let answer = prompt_input("  Are you sure? Type 'yes' to confirm: ");
        if answer.trim() != "yes" {
            println!("  Cancelled.");
            return;
        }
    }

    match std::fs::remove_dir_all(&librefang_dir) {
        Ok(()) => ui::success(&i18n::t_args(
            "reset-success",
            &[("path", &librefang_dir.display().to_string())],
        )),
        Err(e) => {
            ui::error(&i18n::t_args(
                "reset-fail",
                &[
                    ("path", &librefang_dir.display().to_string()),
                    ("error", &e.to_string()),
                ],
            ));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

const RELEASE_REPO: &str = "librefang/librefang";
const RELEASES_LATEST_API: &str =
    "https://api.github.com/repos/librefang/librefang/releases/latest";
const RELEASES_API: &str = "https://api.github.com/repos/librefang/librefang/releases";
const SHELL_INSTALLER_URL: &str = "https://librefang.ai/install.sh";
const POWERSHELL_INSTALLER_URL: &str = "https://librefang.ai/install.ps1";

enum UpdateLaunch {
    #[cfg(not(windows))]
    Completed,
    #[cfg(windows)]
    Detached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReleaseComparison {
    Newer,
    SameCore,
    Older,
    Unknown,
}

fn cmd_update(check: bool, version: Option<String>, channel_override: Option<String>) {
    use librefang_types::config::UpdateChannel;

    let current_exe = std::env::current_exe().unwrap_or_else(|e| {
        ui::error(&format!("Cannot determine current executable path: {e}"));
        std::process::exit(1);
    });

    let current_version = env!("CARGO_PKG_VERSION");
    let current_exe_display = current_exe.display().to_string();
    let requested_version = version.as_deref();

    // Resolve update channel: CLI arg > config.toml > default (stable)
    let channel = if let Some(ref ch) = channel_override {
        match ch.parse::<UpdateChannel>() {
            Ok(c) => c,
            Err(e) => {
                ui::error(&e);
                std::process::exit(1);
            }
        }
    } else {
        load_update_channel_from_config().unwrap_or_default()
    };

    ui::section("Update");
    ui::kv("Current", current_version);
    ui::kv("Channel", &channel.to_string());
    ui::kv("Binary", &current_exe_display);

    let latest_tag = if requested_version.is_none() {
        match fetch_latest_release_tag(channel) {
            Ok(tag) => {
                ui::kv("Latest", &tag);
                Some(tag)
            }
            Err(err) => {
                if check {
                    ui::error(&format!("Failed to check latest release: {err}"));
                    std::process::exit(1);
                }
                ui::warn_with_fix(
                    &format!("Could not resolve the latest published release: {err}"),
                    "Retry later, or pass `--version <tag>` to target a specific release.",
                );
                None
            }
        }
    } else {
        if let Some(target) = requested_version {
            ui::kv("Target", target);
        }
        None
    };
    let target_tag = requested_version
        .map(str::to_owned)
        .or_else(|| latest_tag.clone());
    let target_comparison = target_tag
        .as_deref()
        .map(|tag| compare_release_tag(tag, current_version));

    if check {
        match (target_tag.as_deref(), target_comparison) {
            (Some(tag), Some(ReleaseComparison::Newer)) => {
                ui::warn_with_fix(
                    &format!("A newer published release is available: {tag}"),
                    "Run `librefang update` to install it.",
                );
            }
            (Some(tag), Some(ReleaseComparison::SameCore)) => {
                ui::warn_with_fix(
                    &format!(
                        "The published release {tag} uses the same CLI version core as the current binary ({current_version})."
                    ),
                    "Run `librefang update` if you want the latest published build for this version line.",
                );
            }
            (Some(tag), Some(ReleaseComparison::Older)) => {
                ui::success(&format!(
                    "Current binary version {current_version} is ahead of the published release {tag}."
                ));
            }
            (Some(tag), Some(ReleaseComparison::Unknown)) => {
                ui::warn_with_fix(
                    &format!("Could not compare the current binary with release tag {tag}."),
                    "If you want that exact release, run `librefang update --version <tag>`.",
                );
            }
            _ => {
                ui::warn_with_fix(
                    "Unable to determine whether an update is available.",
                    "Retry later when GitHub Releases is reachable.",
                );
            }
        }
        return;
    }

    if requested_version.is_none() {
        match (latest_tag.as_deref(), target_comparison) {
            (Some(tag), Some(ReleaseComparison::Older)) => {
                ui::success(&format!(
                    "Current binary version {current_version} is ahead of the latest published release {tag}."
                ));
                return;
            }
            (Some(tag), Some(ReleaseComparison::Unknown)) => {
                ui::warn_with_fix(
                    &format!(
                        "Could not safely compare the current binary against release tag {tag}."
                    ),
                    &format!(
                        "Re-run with `librefang update --version {tag}` to install it explicitly."
                    ),
                );
                return;
            }
            _ => {}
        }
    }

    let default_install = default_install_executable();
    let cargo_install = cargo_install_executable();
    let target_version = target_tag.as_deref();

    #[cfg(windows)]
    if same_path(&current_exe, &default_install) && find_daemon().is_some() {
        ui::error_with_fix(
            "Stop the running daemon before updating on Windows.",
            "Run `librefang stop`, then `librefang update`, then `librefang start`.",
        );
        std::process::exit(1);
    }

    if same_path(&current_exe, &default_install) {
        match run_official_update(target_version) {
            #[cfg(not(windows))]
            Ok(UpdateLaunch::Completed) => {
                ui::success("LibreFang CLI updated.");
                if let Some(installed) = installed_binary_version(&default_install) {
                    ui::kv("Installed", &installed);
                }
                // Merge any new config defaults added in the updated binary.
                // Spawn the new binary rather than calling cmd_init_upgrade() here,
                // because the current process still holds the old binary's template.
                ui::blank();
                ui::hint("Merging new config defaults...");
                let _ = std::process::Command::new(&default_install)
                    .args(["init", "--upgrade"])
                    .status();
                ui::hint("If the daemon is running, restart it with `librefang restart`.");
            }
            #[cfg(windows)]
            Ok(UpdateLaunch::Detached) => {
                ui::success("Update launched in the background.");
                ui::hint("Open a new terminal after it finishes and run `librefang --version`.");
                ui::hint("If the daemon is running, restart it after the update completes.");
            }
            Err(err) => {
                ui::error(&format!("Update failed: {err}"));
                std::process::exit(1);
            }
        }
        return;
    }

    if same_path(&current_exe, &cargo_install) {
        let cargo_cmd = cargo_update_command(target_version);
        ui::warn_with_fix(
            "This binary was installed with cargo. Running `cargo install` from inside the active executable is intentionally blocked.",
            &cargo_cmd,
        );
        return;
    }

    let official_path = default_install.display().to_string();
    ui::warn_with_fix(
        &format!(
            "Automatic update only supports the official install path ({official_path}). This binary is running from a different location."
        ),
        &manual_installer_command(target_version),
    );
    ui::hint("If this binary came from another package manager, update it with that package manager instead.");
}

fn fetch_latest_release_tag(
    channel: librefang_types::config::UpdateChannel,
) -> Result<String, String> {
    use librefang_types::config::UpdateChannel;

    let client = update_http_client()?;

    match channel {
        UpdateChannel::Stable => {
            // /releases/latest returns the latest non-draft, non-prerelease
            let response = client
                .get(RELEASES_LATEST_API)
                .send()
                .map_err(|e| format!("GitHub request failed: {e}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("GitHub API returned {status}"));
            }
            let body = response
                .json::<serde_json::Value>()
                .map_err(|e| format!("Failed to decode release metadata: {e}"))?;
            body["tag_name"]
                .as_str()
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "Release metadata is missing `tag_name`".to_string())
        }
        UpdateChannel::Beta | UpdateChannel::Rc => {
            // /releases lists all releases, newest first — filter by channel
            let response = client
                .get(RELEASES_API)
                .send()
                .map_err(|e| format!("GitHub request failed: {e}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("GitHub API returned {status}"));
            }
            let releases = response
                .json::<Vec<serde_json::Value>>()
                .map_err(|e| format!("Failed to decode releases list: {e}"))?;

            for release in &releases {
                let draft = release["draft"].as_bool().unwrap_or(false);
                if draft {
                    continue;
                }
                let Some(tag) = release["tag_name"].as_str().filter(|t| !t.is_empty()) else {
                    continue;
                };
                match channel {
                    UpdateChannel::Rc => return Ok(tag.to_string()),
                    UpdateChannel::Beta => {
                        if !tag.contains("-rc") {
                            return Ok(tag.to_string());
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Err(format!(
                "No matching release found for the '{channel}' channel"
            ))
        }
    }
}

fn update_http_client() -> Result<reqwest::blocking::Client, String> {
    crate::http_client::client_builder()
        .user_agent(format!("librefang-cli/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

fn compare_release_tag(tag: &str, current_version: &str) -> ReleaseComparison {
    let Some(release_core) = parse_version_core(normalize_release_tag(tag)) else {
        return ReleaseComparison::Unknown;
    };
    let Some(current_core) = parse_version_core(current_version) else {
        return ReleaseComparison::Unknown;
    };

    match release_core.cmp(&current_core) {
        std::cmp::Ordering::Greater => ReleaseComparison::Newer,
        std::cmp::Ordering::Equal => ReleaseComparison::SameCore,
        std::cmp::Ordering::Less => ReleaseComparison::Older,
    }
}

fn parse_version_core(version: &str) -> Option<Vec<u64>> {
    let core = version.split('-').next()?;
    if core.is_empty() {
        return None;
    }
    core.split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

fn run_official_update(version: Option<&str>) -> Result<UpdateLaunch, String> {
    let script_url = if cfg!(windows) {
        POWERSHELL_INSTALLER_URL
    } else {
        SHELL_INSTALLER_URL
    };
    let script = download_text(script_url)?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        let wrapped = format!(
            "Start-Sleep -Seconds 1\r\n{script}\r\nRemove-Item $MyInvocation.MyCommand.Path -ErrorAction SilentlyContinue\r\n"
        );
        let script_path = write_update_script(&wrapped, "ps1")?;
        let script_arg = script_path.to_string_lossy().to_string();

        let mut command = std::process::Command::new("powershell");
        command
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &script_arg,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
        if let Some(tag) = version {
            command.env("LIBREFANG_VERSION", tag);
        }

        command
            .spawn()
            .map_err(|e| format!("Failed to launch PowerShell updater: {e}"))?;
        Ok(UpdateLaunch::Detached)
    }

    #[cfg(not(windows))]
    {
        let script_path = write_update_script(&script, "sh")?;
        let mut command = std::process::Command::new("sh");
        command.arg(&script_path);
        if let Some(tag) = version {
            command.env("LIBREFANG_VERSION", tag);
        }

        let status = command
            .status()
            .map_err(|e| format!("Failed to run installer: {e}"))?;
        let _ = std::fs::remove_file(&script_path);
        if !status.success() {
            return Err(format!("Installer exited with status {status}"));
        }
        Ok(UpdateLaunch::Completed)
    }
}

fn download_text(url: &str) -> Result<String, String> {
    let client = update_http_client()?;
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Download failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Download returned {status}"));
    }
    response
        .text()
        .map_err(|e| format!("Failed to read response body: {e}"))
}

#[cfg(not(windows))]
fn installed_binary_version(path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

fn write_update_script(contents: &str, extension: &str) -> Result<PathBuf, String> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "librefang-update-{}-{unique}.{extension}",
        std::process::id()
    ));
    std::fs::write(&path, contents).map_err(|e| format!("Failed to write updater script: {e}"))?;
    restrict_file_permissions(&path);
    Ok(path)
}

fn default_install_executable() -> PathBuf {
    cli_librefang_home().join("bin").join(binary_name())
}

fn cargo_install_executable() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".cargo")
        .join("bin")
        .join(binary_name())
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "librefang.exe"
    } else {
        "librefang"
    }
}

fn same_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

fn normalize_release_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

fn cargo_update_command(version: Option<&str>) -> String {
    match version {
        Some(tag) => format!(
            "cargo install --git https://github.com/{RELEASE_REPO} --tag {tag} librefang-cli --force"
        ),
        None => format!(
            "cargo install --git https://github.com/{RELEASE_REPO} librefang-cli --force"
        ),
    }
}

fn manual_installer_command(version: Option<&str>) -> String {
    #[cfg(windows)]
    {
        match version {
            Some(tag) => {
                format!("$env:LIBREFANG_VERSION='{tag}'; irm {POWERSHELL_INSTALLER_URL} | iex")
            }
            None => format!("irm {POWERSHELL_INSTALLER_URL} | iex"),
        }
    }

    #[cfg(not(windows))]
    {
        match version {
            Some(tag) => format!("curl -fsSL {SHELL_INSTALLER_URL} | LIBREFANG_VERSION={tag} sh"),
            None => format!("curl -fsSL {SHELL_INSTALLER_URL} | sh"),
        }
    }
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

fn cmd_uninstall(confirm: bool, keep_config: bool) {
    let librefang_dir = cli_librefang_home();
    let exe_path = std::env::current_exe().ok();

    // Step 1: Show what will be removed
    println!();
    println!(
        "  {}",
        "This will completely uninstall LibreFang from your system."
            .bold()
            .red()
    );
    println!();
    if librefang_dir.exists() {
        if keep_config {
            println!(
                "  • Remove data in {} (keeping config files)",
                librefang_dir.display()
            );
        } else {
            println!("  • Remove {}", librefang_dir.display());
        }
    }
    if let Some(ref exe) = exe_path {
        println!("  • Remove binary: {}", exe.display());
    }
    // Check cargo bin path
    let cargo_bin = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".cargo")
        .join("bin")
        .join(if cfg!(windows) {
            "librefang.exe"
        } else {
            "librefang"
        });
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        println!("  • Remove cargo binary: {}", cargo_bin.display());
    }
    println!("  • Remove auto-start entries (if any)");
    println!("  • Clean PATH from shell configs (if any)");
    println!();

    // Step 2: Confirm
    if !confirm {
        let answer = prompt_input("  Type 'uninstall' to confirm: ");
        if answer.trim() != "uninstall" {
            println!("  Cancelled.");
            return;
        }
        println!();
    }

    // Step 3: Stop running daemon
    if find_daemon().is_some() {
        println!("  {}", i18n::t("uninstall-stopping-daemon"));
        cmd_stop(None);
        // Give it a moment
        std::thread::sleep(std::time::Duration::from_secs(1));
        // Force kill if still alive
        if find_daemon().is_some() {
            if let Some(info) = read_daemon_info(&librefang_dir) {
                force_kill_pid(info.pid);
                let _ = std::fs::remove_file(librefang_dir.join("daemon.json"));
            }
        }
    }

    // Step 4: Remove auto-start entries
    let user_home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    remove_autostart_entries(&user_home);

    // Step 5: Clean PATH from shell configs
    if let Some(ref exe) = exe_path {
        if let Some(bin_dir) = exe.parent() {
            clean_path_entries(&user_home, &bin_dir.to_string_lossy());
        }
    }

    // Step 6: Remove ~/.librefang/ data
    if librefang_dir.exists() {
        if keep_config {
            remove_dir_except_config(&librefang_dir);
            ui::success(&i18n::t("uninstall-removed-data-kept"));
        } else {
            match std::fs::remove_dir_all(&librefang_dir) {
                Ok(()) => ui::success(&i18n::t_args(
                    "uninstall-removed",
                    &[("path", &librefang_dir.display().to_string())],
                )),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-failed",
                    &[
                        ("path", &librefang_dir.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                )),
            }
        }
    }

    // Step 7: Remove cargo bin copy if it exists and is separate from current exe
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        match std::fs::remove_file(&cargo_bin) {
            Ok(()) => ui::success(&i18n::t_args(
                "uninstall-removed",
                &[("path", &cargo_bin.display().to_string())],
            )),
            Err(e) => ui::error(&i18n::t_args(
                "uninstall-remove-failed",
                &[
                    ("path", &cargo_bin.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )),
        }
    }

    // Step 8: Remove the binary itself (skip if already removed with ~/.librefang/)
    if let Some(exe) = exe_path {
        if exe.exists() {
            remove_self_binary(&exe);
        }
    }

    println!();
    ui::success(&i18n::t("uninstall-goodbye"));
}

/// Remove auto-start / launch-agent / systemd entries.
#[allow(unused_variables)]
fn remove_autostart_entries(home: &std::path::Path) {
    #[cfg(windows)]
    {
        // Windows: remove from HKCU\Software\Microsoft\Windows\CurrentVersion\Run
        let output = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
                "/f",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success(&i18n::t("uninstall-removed-autostart-win"));
            }
            _ => {} // Entry didn't exist — that's fine
        }
    }

    #[cfg(target_os = "macos")]
    {
        let plist = home.join("Library/LaunchAgents/ai.librefang.desktop.plist");
        if plist.exists() {
            // Unload first
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist.to_string_lossy()])
                .output();
            match std::fs::remove_file(&plist) {
                Ok(()) => ui::success(&i18n::t("uninstall-removed-launch-agent")),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-launch-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let desktop_file = home.join(".config/autostart/LibreFang.desktop");
        if desktop_file.exists() {
            match std::fs::remove_file(&desktop_file) {
                Ok(()) => ui::success(&i18n::t("uninstall-removed-autostart-linux")),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-autostart-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }

        // Also check for systemd user service
        let service_file = home.join(".config/systemd/user/librefang.service");
        if service_file.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "librefang.service"])
                .output();
            match std::fs::remove_file(&service_file) {
                Ok(()) => {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output();
                    ui::success(&i18n::t("uninstall-removed-systemd"));
                }
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-systemd-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }
    }
}

/// Remove lines from shell config files that add librefang to PATH.
#[allow(unused_variables)]
fn clean_path_entries(home: &std::path::Path, librefang_dir: &str) {
    #[cfg(not(windows))]
    {
        let shell_files = [
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".profile"),
            home.join(".zshrc"),
            home.join(".config/fish/config.fish"),
        ];

        for path in &shell_files {
            if !path.exists() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let filtered: Vec<&str> = content
                .lines()
                .filter(|line| !is_librefang_path_line(line, librefang_dir))
                .collect();
            if filtered.len() < content.lines().count() {
                let new_content = filtered.join("\n");
                // Preserve trailing newline if original had one
                let new_content = if content.ends_with('\n') {
                    format!("{new_content}\n")
                } else {
                    new_content
                };
                if std::fs::write(path, &new_content).is_ok() {
                    ui::success(&i18n::t_args(
                        "uninstall-cleaned-path",
                        &[("path", &path.display().to_string())],
                    ));
                }
            }
        }
    }

    #[cfg(windows)]
    {
        // Read User PATH via PowerShell, filter out librefang entries, write back
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "[Environment]::GetEnvironmentVariable('PATH', 'User')",
            ])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let current = String::from_utf8_lossy(&out.stdout);
                let current = current.trim();
                if !current.is_empty() {
                    let dir_lower = librefang_dir.to_lowercase();
                    let filtered: Vec<&str> = current
                        .split(';')
                        .filter(|entry| {
                            let e = entry.trim().to_lowercase();
                            !e.is_empty() && !e.contains("librefang") && !e.contains(&dir_lower)
                        })
                        .collect();
                    if filtered.len() < current.split(';').count() {
                        let new_path = filtered.join(";");
                        let ps_cmd = format!(
                            "[Environment]::SetEnvironmentVariable('PATH', '{}', 'User')",
                            new_path.replace('\'', "''")
                        );
                        let result = std::process::Command::new("powershell")
                            .args(["-NoProfile", "-Command", &ps_cmd])
                            .output();
                        if result.is_ok_and(|o| o.status.success()) {
                            ui::success(&i18n::t("uninstall-cleaned-path-win"));
                        }
                    }
                }
            }
        }
    }
}

/// Returns true if a shell config line is an librefang PATH export.
/// Must match BOTH an librefang reference AND a PATH-setting pattern.
#[cfg(any(not(windows), test))]
fn is_librefang_path_line(line: &str, librefang_dir: &str) -> bool {
    let lower = line.to_lowercase();
    let has_librefang =
        lower.contains("librefang") || lower.contains(&librefang_dir.to_lowercase());
    if !has_librefang {
        return false;
    }
    // Match common PATH-setting patterns
    lower.contains("export path=")
        || lower.contains("export path =")
        || lower.starts_with("path=")
        || lower.contains("set -gx path")
        || lower.contains("fish_add_path")
}

/// Remove everything in ~/.librefang/ except config files.
fn remove_dir_except_config(librefang_dir: &std::path::Path) {
    let keep = ["config.toml", ".env", "secrets.env"];
    let Ok(entries) = std::fs::read_dir(librefang_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if keep.contains(&name_str.as_ref()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Remove the currently-running binary.
fn remove_self_binary(exe_path: &std::path::Path) {
    #[cfg(unix)]
    {
        // On Unix, running binaries can be unlinked — the OS keeps the inode
        // alive until the process exits.
        match std::fs::remove_file(exe_path) {
            Ok(()) => ui::success(&i18n::t_args(
                "uninstall-removed",
                &[("path", &exe_path.display().to_string())],
            )),
            Err(e) => ui::error(&i18n::t_args(
                "uninstall-remove-failed",
                &[
                    ("path", &exe_path.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )),
        }
    }

    #[cfg(windows)]
    {
        // Windows locks running executables. Rename first, then spawn a
        // detached process that waits briefly and deletes the renamed file.
        let old_path = exe_path.with_extension("exe.old");
        if std::fs::rename(exe_path, &old_path).is_err() {
            ui::error(&format!(
                "Could not rename binary for deferred deletion: {}",
                exe_path.display()
            ));
            return;
        }

        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        let del_cmd = format!(
            "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"",
            old_path.display()
        );
        let _ = std::process::Command::new("cmd.exe")
            .args(["/C", &del_cmd])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
            .spawn();

        ui::success(&i18n::t_args(
            "uninstall-removed",
            &[("path", &exe_path.display().to_string())],
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compare_release_tag, daemon_log_path_for_config, daemon_log_path_for_home,
        detached_daemon_args, find_daemon_with_probe, is_valid_env_var_name, normalize_daemon_addr,
        normalize_release_tag, parse_toml_integer, parse_version_core, pool_strategy_canon,
        resolve_device_auth_start, resolve_hand_instance, AuthCommands, ChannelCommands, Cli,
        Commands, DeviceAuthNextStep, GatewayCommands, MemoryCommands, ReleaseComparison,
    };
    use clap::Parser;
    use serde_json::json;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    // --- Config set numeric parsing (#3461) ---

    #[test]
    fn parse_toml_integer_accepts_normal_i64() {
        match parse_toml_integer("42").unwrap() {
            toml::Value::Integer(v) => assert_eq!(v, 42),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn parse_toml_integer_accepts_i64_max() {
        match parse_toml_integer(&i64::MAX.to_string()).unwrap() {
            toml::Value::Integer(v) => assert_eq!(v, i64::MAX),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn parse_toml_integer_rejects_u64_max_instead_of_truncating() {
        // u64::MAX as i64 would silently become -1 — we must error instead.
        let err = parse_toml_integer(&u64::MAX.to_string()).unwrap_err();
        assert!(err.contains("exceeds i64::MAX"), "got: {err}");
    }

    #[test]
    fn parse_toml_integer_rejects_non_integer() {
        assert!(parse_toml_integer("not-a-number").is_err());
    }

    // --- Doctor command unit tests ---

    #[test]
    fn test_start_accepts_tail_flag() {
        let cli = Cli::parse_from(["librefang", "start", "--tail"]);
        match cli.command {
            Some(Commands::Start {
                tail,
                foreground,
                spawned,
            }) => {
                assert!(tail);
                assert!(!foreground);
                assert!(!spawned);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_restart_accepts_tail_flag() {
        let cli = Cli::parse_from(["librefang", "restart", "--tail"]);
        match cli.command {
            Some(Commands::Restart { tail, foreground }) => {
                assert!(tail);
                assert!(!foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_gateway_start_accepts_tail_flag() {
        let cli = Cli::parse_from(["librefang", "gateway", "start", "--tail"]);
        match cli.command {
            Some(Commands::Gateway(GatewayCommands::Start { tail, foreground })) => {
                assert!(tail);
                assert!(!foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_gateway_restart_accepts_tail_flag() {
        let cli = Cli::parse_from(["librefang", "gateway", "restart", "--tail"]);
        match cli.command {
            Some(Commands::Gateway(GatewayCommands::Restart { tail, foreground })) => {
                assert!(tail);
                assert!(!foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_start_accepts_foreground_flag() {
        let cli = Cli::parse_from(["librefang", "start", "--foreground"]);
        match cli.command {
            Some(Commands::Start {
                tail,
                foreground,
                spawned,
            }) => {
                assert!(!tail);
                assert!(foreground);
                assert!(!spawned);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_restart_accepts_foreground_flag() {
        let cli = Cli::parse_from(["librefang", "restart", "--foreground"]);
        match cli.command {
            Some(Commands::Restart { tail, foreground }) => {
                assert!(!tail);
                assert!(foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_gateway_start_accepts_foreground_flag() {
        let cli = Cli::parse_from(["librefang", "gateway", "start", "--foreground"]);
        match cli.command {
            Some(Commands::Gateway(GatewayCommands::Start { tail, foreground })) => {
                assert!(!tail);
                assert!(foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_gateway_restart_accepts_foreground_flag() {
        let cli = Cli::parse_from(["librefang", "gateway", "restart", "--foreground"]);
        match cli.command {
            Some(Commands::Gateway(GatewayCommands::Restart { tail, foreground })) => {
                assert!(!tail);
                assert!(foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_auth_chatgpt_accepts_device_auth_flag() {
        let cli = Cli::parse_from(["librefang", "auth", "chatgpt", "--device-auth"]);
        match cli.command {
            Some(Commands::Auth(AuthCommands::Chatgpt { device_auth })) => {
                assert!(device_auth);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_resolve_device_auth_start_continues_device_path() {
        let prompt = librefang_runtime::chatgpt_oauth::DeviceAuthPrompt {
            device_auth_id: "device-1".to_string(),
            user_code: "ABCD-EFGH".to_string(),
            interval_secs: 9,
        };

        match resolve_device_auth_start(Ok(prompt.clone())).unwrap() {
            DeviceAuthNextStep::ContinueDevice(actual) => assert_eq!(actual, prompt),
            DeviceAuthNextStep::FallbackToBrowser(_) => panic!("unexpected fallback"),
        }
    }

    #[test]
    fn test_resolve_device_auth_start_requests_browser_fallback_on_unsupported_error() {
        let err = librefang_runtime::chatgpt_oauth::DeviceAuthFlowError::BrowserFallback {
            message: "fallback".to_string(),
        };

        match resolve_device_auth_start(Err(err)).unwrap() {
            DeviceAuthNextStep::FallbackToBrowser(message) => assert_eq!(message, "fallback"),
            DeviceAuthNextStep::ContinueDevice(_) => panic!("unexpected device continuation"),
        }
    }

    #[test]
    fn test_start_rejects_tail_with_foreground() {
        let cli = Cli::try_parse_from(["librefang", "start", "--tail", "--foreground"]);
        assert!(cli.is_err());
    }

    #[test]
    fn test_detached_daemon_args_include_config_and_spawned_flag() {
        let args = detached_daemon_args(Some(Path::new("/tmp/librefang.toml")));
        assert_eq!(
            args,
            vec![
                OsString::from("--config"),
                OsString::from("/tmp/librefang.toml"),
                OsString::from("start"),
                OsString::from("--spawned"),
            ]
        );
    }

    #[test]
    fn test_daemon_log_path_uses_logs_directory() {
        let home = Path::new("/tmp/librefang-home");
        assert_eq!(
            daemon_log_path_for_home(home),
            home.join("logs").join("daemon.log")
        );
    }

    #[test]
    fn test_daemon_log_path_respects_custom_config_home_dir() {
        let temp_root = std::env::temp_dir().join(format!(
            "librefang-cli-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_root).unwrap();
        let config_path = temp_root.join("config.toml");
        let custom_home = temp_root.join("custom-home");
        fs::write(
            &config_path,
            format!("home_dir = {:?}\n", custom_home.display().to_string()),
        )
        .unwrap();

        assert_eq!(
            daemon_log_path_for_config(Some(&config_path)),
            custom_home.join("logs").join("daemon.log")
        );

        let _ = fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn test_doctor_skill_registry_loads() {
        let skills_dir = std::env::temp_dir().join("librefang-doctor-test-skills");
        let mut skill_reg = librefang_skills::registry::SkillRegistry::new(skills_dir);
        let count = skill_reg.load_all().unwrap_or(0);
        assert_eq!(skill_reg.count(), count);
    }

    #[test]
    fn test_doctor_extension_registry_loads_templates() {
        let tmp = std::env::temp_dir().join("librefang-doctor-test-ext");
        let _ = std::fs::create_dir_all(&tmp);
        let mut catalog = librefang_extensions::catalog::McpCatalog::new(&tmp);
        let count = catalog.load(&librefang_runtime::registry_sync::resolve_home_dir_for_tests());
        assert_eq!(catalog.len(), count);
    }

    #[test]
    fn test_doctor_config_deser_default() {
        // Default KernelConfig should serialize/deserialize round-trip
        let config = librefang_types::config::KernelConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: librefang_types::config::KernelConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.api_listen, config.api_listen);
    }

    #[test]
    fn test_doctor_config_include_field() {
        let config_toml = r#"
api_listen = "127.0.0.1:4545"
include = ["providers.toml", "agents.toml"]

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
        let config: librefang_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.include.len(), 2);
        assert_eq!(config.include[0], "providers.toml");
        assert_eq!(config.include[1], "agents.toml");
    }

    #[test]
    fn test_doctor_exec_policy_field() {
        let config_toml = r#"
api_listen = "127.0.0.1:4545"

[exec_policy]
mode = "allowlist"
safe_bins = ["ls", "cat", "echo"]
timeout_secs = 30

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
        let config: librefang_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(
            config.exec_policy.mode,
            librefang_types::config::ExecSecurityMode::Allowlist
        );
        assert_eq!(config.exec_policy.safe_bins.len(), 3);
        assert_eq!(config.exec_policy.timeout_secs, 30);
    }

    #[test]
    fn test_doctor_mcp_transport_validation() {
        let config_toml = r#"
api_listen = "127.0.0.1:4545"

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"

[[mcp_servers]]
name = "github"
timeout_secs = 30

[mcp_servers.transport]
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
"#;
        let config: librefang_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].name, "github");
        match config.mcp_servers[0].transport.as_ref().unwrap() {
            librefang_types::config::McpTransportEntry::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected Stdio transport"),
        }
    }

    #[test]
    fn test_doctor_http_compat_transport_validation() {
        let config_toml = r#"
api_listen = "127.0.0.1:4545"

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"

[[mcp_servers]]
name = "http-tools"
timeout_secs = 30

[mcp_servers.transport]
type = "http_compat"
base_url = "http://127.0.0.1:11235"

[[mcp_servers.transport.headers]]
name = "Authorization"
value_env = "HTTP_TOOLS_TOKEN"

[[mcp_servers.transport.tools]]
name = "search"
description = "Search HTTP backend"
path = "/search"
method = "get"
request_mode = "query"
response_mode = "json"
input_schema = { type = "object" }
"#;
        let config: librefang_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].name, "http-tools");
        match config.mcp_servers[0].transport.as_ref().unwrap() {
            librefang_types::config::McpTransportEntry::HttpCompat {
                base_url,
                headers,
                tools,
            } => {
                assert_eq!(base_url, "http://127.0.0.1:11235");
                assert_eq!(headers.len(), 1);
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "search");
            }
            _ => panic!("Expected HttpCompat transport"),
        }
    }

    #[test]
    fn test_doctor_skill_injection_scan_clean() {
        let clean_content = "This is a normal skill prompt with helpful instructions.";
        let warnings = librefang_skills::verify::SkillVerifier::scan_prompt_content(clean_content);
        assert!(warnings.is_empty(), "Clean content should have no warnings");
    }

    #[test]
    fn test_doctor_hook_event_variants() {
        // Verify all 4 hook event types are constructable
        use librefang_types::agent::HookEvent;
        let events = [
            HookEvent::BeforeToolCall,
            HookEvent::AfterToolCall,
            HookEvent::BeforePromptBuild,
            HookEvent::AgentLoopEnd,
        ];
        assert_eq!(events.len(), 4);
    }

    // --- Uninstall command unit tests ---

    #[test]
    fn test_uninstall_path_line_filter() {
        use super::is_librefang_path_line;
        let dir = "/home/user/.librefang/bin";

        // Should match: librefang PATH exports
        assert!(is_librefang_path_line(
            r#"export PATH="$HOME/.librefang/bin:$PATH""#,
            dir
        ));
        assert!(is_librefang_path_line(
            r#"export PATH="/home/user/.librefang/bin:$PATH""#,
            dir
        ));
        assert!(is_librefang_path_line(
            "set -gx PATH $HOME/.librefang/bin $PATH",
            dir
        ));
        assert!(is_librefang_path_line(
            "fish_add_path $HOME/.librefang/bin",
            dir
        ));

        // Should NOT match: unrelated PATH exports
        assert!(!is_librefang_path_line(
            r#"export PATH="$HOME/.cargo/bin:$PATH""#,
            dir
        ));
        assert!(!is_librefang_path_line(
            r#"export PATH="/usr/local/bin:$PATH""#,
            dir
        ));

        // Should NOT match: librefang lines that aren't PATH-related
        assert!(!is_librefang_path_line("# librefang config", dir));
        assert!(!is_librefang_path_line("alias of=librefang", dir));
    }

    #[test]
    fn test_update_command_parses() {
        let cli = Cli::parse_from(["librefang", "update"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Update {
                check: false,
                version: None,
                channel: None,
            })
        ));
    }

    #[test]
    fn test_update_check_command_parses() {
        let cli = Cli::parse_from(["librefang", "update", "--check"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Update {
                check: true,
                version: None,
                channel: None,
            })
        ));
    }

    #[test]
    fn test_update_channel_command_parses() {
        let cli = Cli::parse_from(["librefang", "update", "--channel", "rc"]);
        match cli.command {
            Some(Commands::Update { channel, .. }) => {
                assert_eq!(channel.as_deref(), Some("rc"));
            }
            _ => panic!("Expected Update command"),
        }
    }

    #[test]
    fn test_spawn_alias_parses() {
        let cli = Cli::parse_from(["librefang", "spawn", "coder", "--name", "backend-coder"]);
        assert!(matches!(cli.command, Some(Commands::Spawn(_))));
    }

    #[test]
    fn test_agents_alias_parses() {
        let cli = Cli::parse_from(["librefang", "agents", "--json"]);
        assert!(matches!(cli.command, Some(Commands::Agents { json: true })));
    }

    #[test]
    fn test_kill_alias_parses() {
        let cli = Cli::parse_from(["librefang", "kill", "agent-123"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Kill { agent_id }) if agent_id == "agent-123"
        ));
    }

    #[test]
    fn test_agent_spawn_dry_run_parses() {
        let cli = Cli::parse_from(["librefang", "agent", "spawn", "--dry-run", "agent.toml"]);
        assert!(matches!(cli.command, Some(Commands::Agent(_))));
    }

    #[test]
    fn test_hand_status_parses() {
        let cli = Cli::parse_from(["librefang", "hand", "status", "researcher"]);
        assert!(matches!(cli.command, Some(Commands::Hand(_))));
    }

    #[test]
    fn test_skill_test_parses() {
        let cli = Cli::parse_from(["librefang", "skill", "test", ".", "--tool", "summarize"]);
        assert!(matches!(cli.command, Some(Commands::Skill(_))));
    }

    #[test]
    fn test_skill_publish_parses() {
        let cli = Cli::parse_from([
            "librefang",
            "skill",
            "publish",
            ".",
            "--repo",
            "librefang-skills/demo",
            "--dry-run",
        ]);
        assert!(matches!(cli.command, Some(Commands::Skill(_))));
    }

    #[test]
    fn test_channel_list_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "list"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::List))
        ));
    }

    #[test]
    fn test_channel_setup_with_name_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "setup", "telegram"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::Setup { name: Some(ref n) })) if n == "telegram"
        ));
    }

    #[test]
    fn test_channel_setup_picker_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "setup"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::Setup { name: None }))
        ));
    }

    #[test]
    fn test_channel_reload_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "reload"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::Reload))
        ));
    }

    #[test]
    fn test_channel_rm_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "rm", "telegram"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::Rm { ref name })) if name == "telegram"
        ));
    }

    #[test]
    fn test_normalize_release_tag_strips_v_prefix() {
        assert_eq!(normalize_release_tag("v0.3.56"), "0.3.56");
        assert_eq!(normalize_release_tag("0.3.56"), "0.3.56");
    }

    #[test]
    fn test_parse_version_core_strips_release_suffix() {
        assert_eq!(parse_version_core("0.3.56-20260312"), Some(vec![0, 3, 56]));
        assert_eq!(parse_version_core("0.3.56"), Some(vec![0, 3, 56]));
    }

    #[test]
    fn test_compare_release_tag_detects_newer_release() {
        assert_eq!(
            compare_release_tag("v0.3.57-20260312", "0.3.56"),
            ReleaseComparison::Newer
        );
    }

    #[test]
    fn test_compare_release_tag_detects_same_core_release() {
        assert_eq!(
            compare_release_tag("v0.3.56-20260312", "0.3.56"),
            ReleaseComparison::SameCore
        );
    }

    #[test]
    fn test_compare_release_tag_detects_older_release() {
        assert_eq!(
            compare_release_tag("v0.3.55-20260312", "0.3.56"),
            ReleaseComparison::Older
        );
    }

    #[test]
    fn test_resolve_hand_instance_matches_hand_id() {
        let instances = vec![serde_json::json!({
            "instance_id": "inst-1",
            "hand_id": "researcher",
            "status": "running",
            "agent_name": "researcher-agent"
        })];
        let resolved =
            resolve_hand_instance(&instances, "researcher").expect("hand should resolve");
        assert_eq!(resolved["instance_id"].as_str(), Some("inst-1"));
    }

    #[test]
    fn test_resolve_hand_instance_matches_instance_id() {
        let instances = vec![serde_json::json!({
            "instance_id": "inst-1",
            "hand_id": "researcher"
        })];
        let resolved =
            resolve_hand_instance(&instances, "inst-1").expect("instance should resolve");
        assert_eq!(resolved["hand_id"].as_str(), Some("researcher"));
    }

    // --- WithTraceId log-format wrapper tests ---
    //
    // The wrapper is the Rust-side counterpart of the Loki `derivedFields`
    // regex provisioned in `deploy/grafana/provisioning/datasources/loki.yml`.
    // It must (a) be a transparent passthrough when no OTel context is active
    // (the common case for one-shot CLI commands and early boot), and (b)
    // emit `trace_id=<32-hex>` exactly when a context is live so the Loki
    // regex resolves it into a clickable trace link.
    //
    // We can't easily build a live OTel context inside a unit test without
    // spinning up an exporter, so the OTel-active path is covered by the
    // live integration test described in `deploy/OBSERVABILITY.md`. These
    // tests pin the no-OTel-context behaviour, which is what regresses
    // first if someone refactors the wrapper.

    #[test]
    fn test_with_trace_id_passthrough_without_otel_context() {
        use super::WithTraceId;
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;
        use tracing_subscriber::layer::SubscriberExt;

        // Capture writer: collects every byte written by the fmt layer so the
        // test can assert on the rendered line. Wrapped in Arc<Mutex<Vec<u8>>>
        // so both the subscriber and the test body share a view.
        #[derive(Clone)]
        struct VecWriter(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for VecWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for VecWriter {
            type Writer = VecWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = VecWriter(buf.clone());
        let inner = tracing_subscriber::fmt::format()
            .without_time()
            .with_target(false)
            .compact();
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .event_format(WithTraceId(inner));
        let subscriber = tracing_subscriber::registry().with(layer);

        // Scope the dispatcher to this test so we don't fight the global
        // subscriber installed by other tests in the binary.
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("hello world");
        });

        let line = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");
        assert!(
            line.contains("hello world"),
            "expected the inner formatter to render the message, got: {line:?}"
        );
        assert!(
            !line.contains("trace_id="),
            "expected NO trace_id prefix when no OTel context is active, got: {line:?}"
        );
    }

    #[test]
    fn test_with_trace_id_format_matches_loki_regex() {
        // Pin the exact format we emit so the Loki `derivedFields` regex in
        // `deploy/grafana/provisioning/datasources/loki.yml` keeps resolving:
        // `matcherRegex: 'trace_id="?([0-9a-f]{32})"?'`.
        //
        // If someone changes the format string in `WithTraceId::format_event`
        // (e.g. to `traceId={...}` or to upper-case hex), this test fails
        // before the change reaches Grafana and silently breaks log↔trace
        // linking in the dashboards.
        let trace_id_u128: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef_u128;
        let rendered = format!("trace_id={trace_id_u128:032x} ");
        assert_eq!(
            rendered, "trace_id=0123456789abcdef0123456789abcdef ",
            "trace_id format must be 32 lowercase hex chars with no quotes"
        );

        // Mimic the Loki regex `trace_id="?([0-9a-f]{32})"?` without pulling
        // in a regex crate just for one assertion: locate the `trace_id=`
        // prefix, optionally consume a quote, then take 32 chars and verify
        // they are all lowercase hex.
        let needle = "trace_id=";
        let pos = rendered
            .find(needle)
            .expect("emitted line must contain trace_id=");
        let after = &rendered[pos + needle.len()..];
        let after = after.strip_prefix('"').unwrap_or(after);
        let hex: String = after.chars().take(32).collect();
        assert_eq!(hex.len(), 32, "expected 32 hex chars, got {hex:?}");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "expected lowercase hex, got {hex:?}"
        );
        assert_eq!(hex, "0123456789abcdef0123456789abcdef");
    }

    /// Pins the daemon's compact-format behavior: when an event fires inside
    /// a span carrying `agent.id` / `session.id` fields, the rendered line
    /// MUST include both as inline span suffix tokens (the format
    /// `tracing-subscriber`'s `Compact` formatter emits is
    /// `<level> <span_name>: <message> <field>=<value> ...`). Daemon log
    /// search relies on this to correlate any line back to the originating
    /// agent + session — see also the `#[instrument]` on `run_agent_loop`
    /// in `librefang-runtime/src/agent_loop.rs`.
    #[test]
    fn with_trace_id_compact_format_carries_agent_and_session_ids_from_span() {
        use super::WithTraceId;
        use std::sync::{Arc, Mutex};
        use tracing::{info_span, warn};
        use tracing_subscriber::fmt::MakeWriter;
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Clone)]
        struct VecWriter(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for VecWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for VecWriter {
            type Writer = VecWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = VecWriter(buf.clone());
        let inner = tracing_subscriber::fmt::format()
            .without_time()
            .with_target(false)
            .compact();
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .event_format(WithTraceId(inner));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let span = info_span!(
                "run_agent_loop",
                agent.id = "agent-uuid-aaaa",
                session.id = "session-uuid-bbbb",
            );
            let _entered = span.enter();
            warn!("shell exec full mode");
        });

        let captured = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");
        assert!(
            captured.contains("agent.id=\"agent-uuid-aaaa\""),
            "expected agent.id span field in line, got: {captured:?}"
        );
        assert!(
            captured.contains("session.id=\"session-uuid-bbbb\""),
            "expected session.id span field in line, got: {captured:?}"
        );
        assert!(
            captured.contains("run_agent_loop"),
            "expected span name prefix, got: {captured:?}"
        );
        assert!(
            captured.contains("shell exec full mode"),
            "expected original message preserved, got: {captured:?}"
        );
    }

    // --- Daemon detection / launcher port logic (#3582) ---
    //
    // These exercise the `find_daemon_with_probe` core, which was extracted
    // from `find_daemon_in_home` so the HTTP probe can be faked in unit
    // tests instead of binding sockets or making real requests.

    fn write_daemon_json(home: &Path, listen_addr: &str) {
        let body = json!({
            "pid": 4242u32,
            "listen_addr": listen_addr,
            "started_at": "1970-01-01T00:00:00Z",
            "version": "0.0.0-test",
            "platform": "test",
        });
        fs::write(home.join("daemon.json"), body.to_string()).expect("write daemon.json");
    }

    #[test]
    fn normalize_daemon_addr_rewrites_bind_all_to_loopback() {
        // `0.0.0.0:4545` is the default bind-all address; on macOS, probing
        // it directly can hang, so the launcher rewrites to 127.0.0.1.
        assert_eq!(normalize_daemon_addr("0.0.0.0:4545"), "127.0.0.1:4545");
    }

    #[test]
    fn normalize_daemon_addr_leaves_explicit_loopback_alone() {
        assert_eq!(normalize_daemon_addr("127.0.0.1:4545"), "127.0.0.1:4545");
    }

    #[test]
    fn normalize_daemon_addr_leaves_other_hosts_alone() {
        // A user who explicitly bound to a LAN IP should keep it.
        assert_eq!(
            normalize_daemon_addr("192.168.1.10:4545"),
            "192.168.1.10:4545"
        );
    }

    #[test]
    fn find_daemon_with_probe_returns_none_when_no_daemon_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // No daemon.json written. Probe must NOT be invoked.
        let probe_called = std::cell::Cell::new(false);
        let got = find_daemon_with_probe(tmp.path(), |_url| {
            probe_called.set(true);
            true
        });
        assert!(got.is_none());
        assert!(
            !probe_called.get(),
            "probe must not run when daemon.json is absent — saves a network round-trip"
        );
    }

    #[test]
    fn find_daemon_with_probe_returns_none_on_unparseable_daemon_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join("daemon.json"), "not valid json {{{").unwrap();
        let got = find_daemon_with_probe(tmp.path(), |_url| true);
        assert!(
            got.is_none(),
            "corrupt daemon.json must not be treated as a live daemon"
        );
    }

    #[test]
    fn find_daemon_with_probe_returns_base_url_on_healthy_probe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_daemon_json(tmp.path(), "127.0.0.1:4545");

        let seen = std::cell::Cell::new(None);
        let got = find_daemon_with_probe(tmp.path(), |url| {
            seen.set(Some(url.to_string()));
            true
        });

        // The probe receives the /api/health URL...
        assert_eq!(
            seen.into_inner().as_deref(),
            Some("http://127.0.0.1:4545/api/health")
        );
        // ...and the caller gets back the *base* URL (no /api/health suffix).
        assert_eq!(got.as_deref(), Some("http://127.0.0.1:4545"));
    }

    #[test]
    fn find_daemon_with_probe_normalizes_bind_all_in_url() {
        // Regression: ensure 0.0.0.0 in daemon.json is rewritten to 127.0.0.1
        // BEFORE we hand the URL to the probe (and before we return it).
        let tmp = tempfile::tempdir().expect("tempdir");
        write_daemon_json(tmp.path(), "0.0.0.0:4545");

        let seen = std::cell::Cell::new(None);
        let got = find_daemon_with_probe(tmp.path(), |url| {
            seen.set(Some(url.to_string()));
            true
        });

        assert_eq!(
            seen.into_inner().as_deref(),
            Some("http://127.0.0.1:4545/api/health"),
            "probe must see normalized 127.0.0.1 URL, never 0.0.0.0"
        );
        assert_eq!(got.as_deref(), Some("http://127.0.0.1:4545"));
    }

    #[test]
    fn find_daemon_with_probe_returns_none_on_failed_probe() {
        // Stale daemon.json (process gone, port in use by something else, or
        // returning 5xx) — probe returns false → caller gets None.
        let tmp = tempfile::tempdir().expect("tempdir");
        write_daemon_json(tmp.path(), "127.0.0.1:4545");
        let got = find_daemon_with_probe(tmp.path(), |_url| false);
        assert!(got.is_none());
    }

    // Regression guard for #4923: `memory store` must parse identically to
    // `memory set` so the alias added in this PR is wired up correctly.
    #[test]
    fn memory_store_alias_parses_identically_to_memory_set() {
        let via_set =
            Cli::try_parse_from(["librefang", "memory", "set", "coder", "my-key", "my-value"])
                .expect("memory set must parse");
        let via_store = Cli::try_parse_from([
            "librefang",
            "memory",
            "store",
            "coder",
            "my-key",
            "my-value",
        ])
        .expect("memory store alias must parse");

        let (set_agent, set_key, set_val) = match via_set.command.unwrap() {
            Commands::Memory(MemoryCommands::Set { agent, key, value }) => (agent, key, value),
            _ => panic!("unexpected variant from 'memory set'"),
        };
        let (store_agent, store_key, store_val) = match via_store.command.unwrap() {
            Commands::Memory(MemoryCommands::Set { agent, key, value }) => (agent, key, value),
            _ => panic!("unexpected variant from 'memory store'"),
        };

        assert_eq!(set_agent, store_agent);
        assert_eq!(set_key, store_key);
        assert_eq!(set_val, store_val);
    }

    // ── Credential pool CLI helpers (#4965) ───────────────────────────────────

    #[test]
    fn is_valid_env_var_name_accepts_standard_shapes() {
        assert!(is_valid_env_var_name("OPENAI_API_KEY"));
        assert!(is_valid_env_var_name("OPENAI_API_KEY_2"));
        assert!(is_valid_env_var_name("_PRIVATE"));
        assert!(is_valid_env_var_name("A"));
        assert!(is_valid_env_var_name("X1"));
    }

    #[test]
    fn is_valid_env_var_name_rejects_garbage() {
        // Leading digit, lowercase, spaces, punctuation, empty — all rejected.
        assert!(!is_valid_env_var_name(""));
        assert!(!is_valid_env_var_name("1FOO"));
        assert!(!is_valid_env_var_name("foo"));
        assert!(!is_valid_env_var_name("FOO BAR"));
        assert!(!is_valid_env_var_name("FOO-BAR"));
        assert!(!is_valid_env_var_name("FOO.BAR"));
        assert!(!is_valid_env_var_name("FOO$"));
        assert!(!is_valid_env_var_name(" FOO"));
    }

    #[test]
    fn pool_strategy_canon_accepts_known_strategies() {
        assert_eq!(pool_strategy_canon("fill_first"), Some("fill_first"));
        assert_eq!(pool_strategy_canon("Fill-First"), Some("fill_first"));
        assert_eq!(pool_strategy_canon("FILLFIRST"), Some("fill_first"));
        assert_eq!(pool_strategy_canon("round_robin"), Some("round_robin"));
        assert_eq!(pool_strategy_canon("RoundRobin"), Some("round_robin"));
        assert_eq!(pool_strategy_canon("random"), Some("random"));
        assert_eq!(pool_strategy_canon("least_used"), Some("least_used"));
        assert_eq!(pool_strategy_canon("LEASTUSED"), Some("least_used"));
    }

    #[test]
    fn pool_strategy_canon_rejects_unknown() {
        assert_eq!(pool_strategy_canon(""), None);
        assert_eq!(pool_strategy_canon("foo"), None);
        assert_eq!(pool_strategy_canon("priority"), None);
        assert_eq!(pool_strategy_canon("rand"), None);
    }

    /// Round-trip a config.toml fragment containing comments and an unrelated
    /// section through `toml_edit::DocumentMut`. Proves the parser preserves
    /// the bits the mutating pool commands rely on: comments survive,
    /// unrelated tables stay intact, and a freshly inserted
    /// `[[credential_pools]]` lands at the bottom without rewriting the
    /// rest of the file. (The actual cmd_auth_pool_* functions are private
    /// CLI orchestrators that exit the process on error and call `ui::*`
    /// helpers, so we test the underlying mutation primitive directly.)
    #[test]
    fn toml_edit_roundtrip_preserves_comments_and_unrelated_sections() {
        let original = r#"# top-of-file comment
api_listen = "127.0.0.1:4545"

[default_model]
# inline comment in default_model
provider = "anthropic"
model = "claude-3-5-sonnet"
api_key_env = "ANTHROPIC_API_KEY"

# trailing comment before our edit
"#;
        let mut doc: toml_edit::DocumentMut = original.parse().expect("fragment must parse");
        // Insert a credential_pools entry the same way the CLI's add-on-no-pool
        // path does — building an ArrayOfTables and pushing one table into it.
        let item = doc
            .entry("credential_pools")
            .or_insert(toml_edit::Item::ArrayOfTables(
                toml_edit::ArrayOfTables::new(),
            ));
        let arr = item
            .as_array_of_tables_mut()
            .expect("just inserted as array of tables");
        let mut pool_tbl = toml_edit::Table::new();
        pool_tbl["provider"] = toml_edit::value("anthropic");
        pool_tbl["strategy"] = toml_edit::value("fill_first");
        let mut keys_arr = toml_edit::ArrayOfTables::new();
        let mut key_tbl = toml_edit::Table::new();
        key_tbl["api_key_env"] = toml_edit::value("ANTHROPIC_API_KEY_2");
        key_tbl["label"] = toml_edit::value("Backup");
        key_tbl["priority"] = toml_edit::value(5_i64);
        keys_arr.push(key_tbl);
        pool_tbl.insert("keys", toml_edit::Item::ArrayOfTables(keys_arr));
        arr.push(pool_tbl);

        let rendered = doc.to_string();
        // All three comments survive verbatim.
        assert!(
            rendered.contains("# top-of-file comment"),
            "top comment missing: {rendered}"
        );
        assert!(
            rendered.contains("# inline comment in default_model"),
            "inline comment missing: {rendered}"
        );
        assert!(
            rendered.contains("# trailing comment before our edit"),
            "trailing comment missing: {rendered}"
        );
        // Unrelated section intact.
        assert!(rendered.contains("[default_model]"));
        assert!(rendered.contains("provider = \"anthropic\""));
        // New section present with the expected canonical shape.
        assert!(rendered.contains("[[credential_pools]]"));
        assert!(rendered.contains("[[credential_pools.keys]]"));
        assert!(rendered.contains("api_key_env = \"ANTHROPIC_API_KEY_2\""));
        assert!(rendered.contains("label = \"Backup\""));
        assert!(rendered.contains("priority = 5"));
    }
}
